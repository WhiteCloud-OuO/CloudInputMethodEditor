//! TSF 客户端：每个 `text_edit` 宿主窗一个文档管理器 + 上下文，文本存储桥到 RichEdit。
//!
//! Server UI 线程只建一次 `ITfThreadMgr`；键盘消息先给 `ITfKeystrokeMgr`，被输入法吃掉就
//! 不再交给 RichEdit。CloudIME 的候选窗 / 提交因此能作用到 windowless RichEdit。

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Controls::RichEdit::ITextServices;
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfContext, ITfDocumentMgr, ITfKeystrokeMgr, ITfThreadMgr, TF_POPF_ALL,
};
use windows::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};
use windows::core::{BOOL, Error, Interface, Result};

use super::text_store::{self, TextStoreHandle};

thread_local! {
    /// UI 线程的 TSF 管理器；按需初始化，失败后不反复重试。
    static THREAD: RefCell<Option<Rc<TsfThread>>> = const { RefCell::new(None) };
}

struct TsfThread {
    thread_mgr: ITfThreadMgr,
    keystroke: ITfKeystrokeMgr,
    client_id: u32,
}

impl TsfThread {
    fn get() -> Option<Rc<Self>> {
        THREAD.with(|slot| {
            if slot.borrow().is_none() {
                match Self::initialize() {
                    Ok(thread) => *slot.borrow_mut() = Some(Rc::new(thread)),
                    Err(error) => {
                        tracing::warn!(%error, "text_edit：TSF 初始化失败，退化为普通 RichEdit 输入");
                        return None;
                    }
                }
            }
            slot.borrow().clone()
        })
    }

    fn initialize() -> Result<Self> {
        unsafe {
            // Server UI 线程原本没初始化 COM；TSF 要求 STA。
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let thread_mgr: ITfThreadMgr =
            unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)? };
        let client_id = unsafe { thread_mgr.Activate()? };
        let keystroke: ITfKeystrokeMgr = thread_mgr.cast()?;
        Ok(Self {
            thread_mgr,
            keystroke,
            client_id,
        })
    }

    fn set_focus(&self, document: Option<&ITfDocumentMgr>) {
        let _ = unsafe { self.thread_mgr.SetFocus(document) };
    }
}

/// 一块 `text_edit` 的 TSF 上下文。
pub(super) struct TsfContext {
    thread: Rc<TsfThread>,
    document: ITfDocumentMgr,
    store: TextStoreHandle,
}

impl TsfContext {
    pub(super) fn create(hwnd: HWND, services: ITextServices, line_height: i32) -> Result<Self> {
        let thread = TsfThread::get().ok_or_else(|| Error::from(E_FAIL))?;
        let document = unsafe { thread.thread_mgr.CreateDocumentMgr()? };
        let (store, handle) = text_store::create(hwnd, services, line_height);
        let unknown = store.cast::<windows::core::IUnknown>()?;
        let mut context: Option<ITfContext> = None;
        let mut cookie = 0u32;
        unsafe {
            document.CreateContext(thread.client_id, 0, &unknown, &mut context, &mut cookie)?;
            let context = context.ok_or_else(|| Error::from(E_FAIL))?;
            document.Push(&context)?;
        }
        Ok(Self {
            thread,
            document,
            store: handle,
        })
    }

    pub(super) fn focus(&self) {
        self.thread.set_focus(Some(&self.document));
    }

    pub(super) fn blur(&self) {
        self.thread.set_focus(None);
    }

    /// 把一个键盘消息先交给 TSF；返回 `true` = 输入法吃掉了，RichEdit 不要再处理。
    pub(super) fn key(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => unsafe {
                let test = self.thread.keystroke.TestKeyDown(wparam, lparam);
                if !test.map(BOOL::as_bool).unwrap_or(false) {
                    return false;
                }
                self.thread
                    .keystroke
                    .KeyDown(wparam, lparam)
                    .map(BOOL::as_bool)
                    .unwrap_or(false)
            },
            WM_KEYUP | WM_SYSKEYUP => unsafe {
                let test = self.thread.keystroke.TestKeyUp(wparam, lparam);
                if !test.map(BOOL::as_bool).unwrap_or(false) {
                    return false;
                }
                self.thread
                    .keystroke
                    .KeyUp(wparam, lparam)
                    .map(BOOL::as_bool)
                    .unwrap_or(false)
            },
            _ => false,
        }
    }

    pub(super) fn notify_selection(&self) {
        self.store.notify_selection();
    }

    pub(super) fn notify_layout(&self) {
        self.store.notify_layout();
    }
}

impl Drop for TsfContext {
    fn drop(&mut self) {
        self.thread.set_focus(None);
        let _ = unsafe { self.document.Pop(TF_POPF_ALL) };
    }
}
