//! 最小 TSF UI-less 宿主：验证云朵输入法的 UIElement 数据链路。
//!
//! 运行前先确保当前输入法是云朵输入法；窗口出现后点一下再打字（或加 `--auto` 自动敲 `nihao`）。
//! 控制台会打印 `Begin` / `Update` / `End` 与候选页、选中；退出时打印文本存区内容。
//!
//! 不随安装包发布；只用于开发验证。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, SetFocus, VIRTUAL_KEY, VK_A, VK_H, VK_I, VK_N,
    VK_O, VK_SPACE,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITextStoreACP, ITfCandidateListUIElementBehavior, ITfContext,
    ITfDocumentMgr, ITfKeystrokeMgr, ITfSource, ITfThreadMgr, ITfThreadMgrEx, ITfUIElementMgr,
    ITfUIElementSink, TF_TMAE_UIELEMENTENABLEDONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, MSG,
    PostQuitMessage, RegisterClassW, SW_SHOW, SetForegroundWindow, SetTimer, ShowWindow,
    TranslateMessage, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{ComObject, Error, Interface, PCWSTR, Result, w};

use crate::text_store::TextStore;
use crate::uielement_sink::UiElementSink;

const CLASS_NAME: PCWSTR = w!("CloudIMEUilessHost");
const AUTO_TIMER: usize = 1;
const AUTO_PICK_TIMER: usize = 2;
const AUTO_DONE_TIMER: usize = 3;

/// `--auto-pick` 的目标：元素管理器 + 当前元素 id。
type PickTarget = (ITfUIElementMgr, Rc<Cell<Option<u32>>>);

thread_local! {
    /// `--auto-pick` 用：元素管理器 + 当前元素 id，`WM_TIMER` 里拿它们调 SetSelection / Finalize。
    static PICK_TARGET: RefCell<Option<PickTarget>> = const { RefCell::new(None) };

    /// 自动跑时是不是 `--auto-pick`：决定要不要发空格、以及要不要在打字后调宿主动作。
    static PICK_MODE: Cell<bool> = const { Cell::new(false) };
}

struct Host {
    _thread_mgr: ITfThreadMgr,
    _doc_mgr: ITfDocumentMgr,
    _context: ITfContext,
    store_object: ComObject<TextStore>,
    _store: ITextStoreACP,
    _ui_sink: ITfUIElementSink,
    manager: ITfUIElementMgr,
    element_id: Rc<Cell<Option<u32>>>,
    keystroke: ITfKeystrokeMgr,
}

/// 建 TSF 线程、文档与文本存区；`hwnd` 就是文本存区的窗口。
fn setup(hwnd: HWND) -> Result<Host> {
    unsafe {
        let thread_mgr: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let ex: ITfThreadMgrEx = thread_mgr.cast()?;
        let mut tid = 0u32;
        ex.ActivateEx(&mut tid, TF_TMAE_UIELEMENTENABLEDONLY)?;
        println!(
            "[host] thread active flags=0x{:X}",
            ex.GetActiveFlags().unwrap_or(0)
        );

        let manager: ITfUIElementMgr = thread_mgr.cast()?;
        let element_id = Rc::new(Cell::new(None));
        let ui_sink: ITfUIElementSink =
            UiElementSink::new(manager.clone(), element_id.clone()).into();
        let source: ITfSource = thread_mgr.cast()?;
        source.AdviseSink(&ITfUIElementSink::IID, &ui_sink)?;

        let doc_mgr = thread_mgr.CreateDocumentMgr()?;
        let store_object = ComObject::new(TextStore::new(hwnd));
        let store: ITextStoreACP = store_object.to_interface();
        let mut context: Option<ITfContext> = None;
        let mut cookie = 0u32;
        doc_mgr.CreateContext(tid, 0, &store, &mut context, &mut cookie)?;
        let context = context.ok_or_else(|| Error::from(E_FAIL))?;
        doc_mgr.Push(&context)?;
        thread_mgr.SetFocus(&doc_mgr)?;

        let keystroke: ITfKeystrokeMgr = thread_mgr.cast()?;
        Ok(Host {
            _thread_mgr: thread_mgr,
            _doc_mgr: doc_mgr,
            _context: context,
            store_object,
            _store: store,
            _ui_sink: ui_sink,
            manager,
            element_id,
            keystroke,
        })
    }
}

/// 把窗口消息里的按键先交给 TSF（UI-less 宿主必须自己转发）；返回是否被吃。
fn route_key(keystroke: &ITfKeystrokeMgr, msg: &MSG) -> bool {
    unsafe {
        match msg.message {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if keystroke
                    .TestKeyDown(msg.wParam, msg.lParam)
                    .is_ok_and(|eaten| eaten.as_bool())
                {
                    return true;
                }
                if keystroke
                    .KeyDown(msg.wParam, msg.lParam)
                    .is_ok_and(|eaten| eaten.as_bool())
                {
                    return true;
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                if keystroke
                    .TestKeyUp(msg.wParam, msg.lParam)
                    .is_ok_and(|eaten| eaten.as_bool())
                {
                    return true;
                }
                if keystroke
                    .KeyUp(msg.wParam, msg.lParam)
                    .is_ok_and(|eaten| eaten.as_bool())
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn send_key(vk: VIRTUAL_KEY) {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[down, up], size_of::<INPUT>() as i32);
    }
}

fn type_auto(pick: bool) {
    for vk in [VK_N, VK_I, VK_H, VK_A, VK_O] {
        send_key(vk);
        std::thread::sleep(Duration::from_millis(100));
    }
    if !pick {
        send_key(VK_SPACE);
    }
}

/// 宿主侧动作：`SetSelection(1)` 再 `Finalize()`，验证服务端的 UIElement 动作链路。
fn pick_and_finalize() {
    PICK_TARGET.with(|target| {
        let target = target.borrow();
        let Some((manager, id)) = target.as_ref() else {
            return;
        };
        let Some(id) = id.get() else {
            println!("[auto-pick] 没有活跃的 UIElement");
            return;
        };
        let Ok(element) = (unsafe { manager.GetUIElement(id) }) else {
            println!("[auto-pick] GetUIElement({id}) 失败");
            return;
        };
        let Ok(behavior) = element.cast::<ITfCandidateListUIElementBehavior>() else {
            println!("[auto-pick] 元素没有 Behavior 接口");
            return;
        };
        println!("[auto-pick] SetSelection(1)");
        if let Err(error) = unsafe { behavior.SetSelection(1) } {
            println!("[auto-pick] SetSelection 失败: {error}");
        }
        println!("[auto-pick] Finalize()");
        if let Err(error) = unsafe { behavior.Finalize() } {
            println!("[auto-pick] Finalize 失败: {error}");
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == AUTO_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), AUTO_TIMER) };
            let pick = PICK_MODE.with(Cell::get);
            println!("[auto] 发送 nihao{}", if pick { "" } else { " + 空格" });
            type_auto(pick);
            let (timer, delay) = if pick {
                (AUTO_PICK_TIMER, 800)
            } else {
                (AUTO_DONE_TIMER, 1500)
            };
            unsafe { SetTimer(Some(hwnd), timer, delay, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == AUTO_PICK_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), AUTO_PICK_TIMER) };
            pick_and_finalize();
            unsafe { SetTimer(Some(hwnd), AUTO_DONE_TIMER, 1500, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == AUTO_DONE_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), AUTO_DONE_TIMER) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 跑测试宿主；`auto` 时窗口出现 1.2 秒后自动敲 `nihao`（默认再补空格）；
/// `pick` 时不补空格，改为 0.8 秒后调 `SetSelection(1)` + `Finalize()`。
pub fn run(auto: bool, pick: bool) {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    PICK_MODE.with(|mode| mode.set(pick));

    let hinstance = unsafe { GetModuleHandleW(None) }.expect("GetModuleHandleW");
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance.into(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            CLASS_NAME,
            w!("CloudIME UI-less Host — 点这里再打字"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            220,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .expect("CreateWindowExW");

    let host = match setup(hwnd) {
        Ok(host) => host,
        Err(error) => {
            println!("TSF 初始化失败: {error}");
            return;
        }
    };
    PICK_TARGET.with(|target| {
        *target.borrow_mut() = Some((host.manager.clone(), host.element_id.clone()));
    });
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    if auto {
        unsafe { SetTimer(Some(hwnd), AUTO_TIMER, 1200, None) };
    } else {
        println!("点一下窗口，然后用云朵输入法打字；关闭窗口退出。");
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if route_key(&host.keystroke, &msg) {
            continue;
        }
        let _ = unsafe { TranslateMessage(&msg) };
        unsafe { DispatchMessageW(&msg) };
    }
    PICK_TARGET.with(|target| *target.borrow_mut() = None);
    println!("文本存区: {:?}", host.store_object.get().text());
    drop(host);
    unsafe { CoUninitialize() };
}
