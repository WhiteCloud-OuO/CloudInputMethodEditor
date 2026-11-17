//! 同步模态对话框的共用宿主。
//!
//! 内置对话框的 `show` 都是同步阻塞的：在当前独立窗口的 UI 上下文里建一块 owned window，
//! 跑嵌套消息循环等按钮点击，期间挡住所有者。这个模块只负责这套壳子，具体内容由调用方
//! 生成 [`Document`] 并给出「区域名 → 返回下标」的映射。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetWindowRect, IsWindow, IsWindowVisible, MSG, PostMessageW,
    PostQuitMessage, TranslateMessage, WM_APP,
};

use cloudime_render::Document;

use crate::auxiliary_window_template::{
    AuxiliaryWindowTemplate, DialogOptions, PanelAction, PanelEvent,
};
use crate::current::CurrentWindow;
use crate::text_edit::TextHostWindow;
use crate::view::Theme;

/// 按钮点击后叫醒嵌套消息循环的自定义消息。
const WAKE: u32 = WM_APP + 0x62;

/// 对话框里的非按钮事件处理器（行点击 / 滚轮）。
type RegionEvents = Box<dyn FnMut(&PanelEvent, &DialogHandle)>;

/// 对话框内容的更新入口：换文档后重新渲染并贴图。
///
/// 辅助窗口模板是分层的、没有 `WM_PAINT` 重绘路径，内容变了必须再走一次
/// `show_centered` 才会重新出图。
pub(super) struct DialogHandle {
    template: Rc<AuxiliaryWindowTemplate>,
    owner: HWND,
}

impl DialogHandle {
    /// 换掉对话框内容并立即重画。
    pub(super) fn set_document(&self, document: Document) {
        self.template.set_document(document);
        self.refresh();
    }

    /// 按当前所有者矩形重新居中并贴图。
    pub(super) fn refresh(&self) {
        let mut owner = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.owner, &mut owner);
        }
        self.template
            .show_centered(owner, cloud_ui_types::LayoutMode::Vertical);
    }
}

/// 按当前独立窗口最近一帧的背景色选深 / 浅主题。
pub(super) fn theme() -> Theme {
    crate::current::background().map_or_else(Theme::dark, |color| {
        let luma =
            0.299 * f32::from(color.r) + 0.587 * f32::from(color.g) + 0.114 * f32::from(color.b);
        if luma > 128.0 {
            Theme::light()
        } else {
            Theme::dark()
        }
    })
}

/// 对话框遇到非法状态时异常终止程序（不是 panic）。
pub(super) fn abort_with(message: &str) -> ! {
    tracing::error!(message, "对话框非法状态，程序将异常终止");
    std::process::abort();
}

/// 一块同步阻塞的模态对话框；`Drop` 时恢复所有者。
pub(super) struct BlockingPanel {
    template: Rc<AuxiliaryWindowTemplate>,
    result: Arc<AtomicI32>,
    owner: HWND,
    owner_was_enabled: bool,
    /// 所有者的 RichEdit 宿主：独立顶层窗口，`EnableWindow(owner, false)` 管不到。
    edit_hosts: Vec<Rc<TextHostWindow>>,
    /// 打开之前线程里的焦点窗口；关闭时还给所有者。
    previous_focus: Option<HWND>,
    /// 非按钮点击 / 滚轮：事件线程只入队，`pump_value` 里再交给 `on_event`。
    pending: Arc<Mutex<VecDeque<PanelEvent>>>,
    /// 处理行点击 / 滚轮（可更新对话框内容）；只在 UI 线程的 `pump_value` 里调用。
    on_event: RefCell<Option<RegionEvents>>,
}

impl BlockingPanel {
    /// 用已生成的面板内容打开对话框。`button` 把点击区域名映射成返回下标。
    ///
    /// 创建失败时不改所有者状态，直接返回错误，由调用方决定退化成什么结果。
    pub(super) fn open(
        current: &CurrentWindow,
        document: Document,
        button: fn(&str) -> Option<u8>,
    ) -> windows::core::Result<Self> {
        Self::open_inner(
            current,
            document,
            move |region| button(region).map(i32::from),
            None,
        )
    }

    /// 打开对话框，并把非按钮点击 / 滚轮交给 `on_event`。
    ///
    /// `button` 返回 `Some` 时关闭对话框、把该值作为结果；返回 `None` 的点击与滚轮
    /// 排进队列，在 [`pump_value`](Self::pump_value) 里交给 `on_event`，可以更新文档后继续等。
    pub(super) fn open_with(
        current: &CurrentWindow,
        document: Document,
        button: fn(&str) -> Option<i32>,
        on_event: impl FnMut(&PanelEvent, &DialogHandle) + 'static,
    ) -> windows::core::Result<Self> {
        Self::open_inner(current, document, button, Some(Box::new(on_event)))
    }

    fn open_inner<F>(
        current: &CurrentWindow,
        document: Document,
        button: F,
        on_event: Option<RegionEvents>,
    ) -> windows::core::Result<Self>
    where
        F: Fn(&str) -> Option<i32> + Send + 'static,
    {
        // 先记下焦点：建模板 / 禁用所有者都会让原点窗口先丢掉焦点，关闭时要还回去。
        let previous_focus = unsafe { GetFocus() };
        let previous_focus = (!previous_focus.0.is_null()).then_some(previous_focus);
        let template = Rc::new(AuxiliaryWindowTemplate::dialog(
            current.painter.clone(),
            current.hwnd,
            DialogOptions { mask: false },
        )?);
        template.attach();
        template.set_top_most(current.top_most);
        // 拖拽时这些独立顶层窗口要跟着所有者一起挪（编辑框宿主 / 蒙版）。
        template.set_owner_followers(current.edit_hosts.clone(), current.followers.clone());
        // 背景 / 圆角由对话框文档自己画；关掉模板底色，避免两套圆角在边缘叠出亮边。
        template.set_background(false);
        template.set_document(document);
        template.set_focused(true);
        template.set_modal(true);

        // `i32::MIN` = 还没选；按钮结果可以是 0.. / -1（取消）。
        let result = Arc::new(AtomicI32::new(i32::MIN));
        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let forward = on_event.is_some();
        let dialog_hwnd = Arc::new(AtomicIsize::new(template.hwnd().0 as isize));
        template.set_events(Box::new({
            let result = Arc::clone(&result);
            let pending = Arc::clone(&pending);
            let dialog_hwnd = Arc::clone(&dialog_hwnd);
            move |event: PanelEvent| {
                let wake = || {
                    let hwnd = HWND(dialog_hwnd.load(Ordering::SeqCst) as *mut _);
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WAKE, WPARAM(0), LPARAM(0));
                    }
                };
                if event.action == PanelAction::Click
                    && let Some(index) = event.region.as_deref().and_then(&button)
                {
                    result.store(index, Ordering::SeqCst);
                    wake();
                    return;
                }
                if forward && matches!(event.action, PanelAction::Click | PanelAction::Wheel) {
                    pending
                        .lock()
                        .expect("对话框事件队列被毒化")
                        .push_back(event);
                    wake();
                }
            }
        }));

        let mut owner = RECT::default();
        unsafe {
            let _ = GetWindowRect(current.hwnd, &mut owner);
        }
        // 先记下焦点：禁用所有者会让它名下的 RichEdit 宿主先丢掉焦点。
        let owner_was_enabled = unsafe { IsWindowEnabled(current.hwnd).as_bool() };
        if owner_was_enabled {
            unsafe {
                let _ = EnableWindow(current.hwnd, false);
            }
        }
        // 挡住所有者名下的 RichEdit 宿主。
        for host in &current.edit_hosts {
            host.set_input_blocked(true);
        }
        template.show_centered(owner, cloud_ui_types::LayoutMode::Vertical);
        Ok(Self {
            template,
            result,
            owner: current.hwnd,
            owner_was_enabled,
            edit_hosts: current.edit_hosts.clone(),
            previous_focus,
            pending,
            on_event: RefCell::new(on_event),
        })
    }

    /// 面板模板；给需要在其上摆宿主窗口的对话框用。
    pub(super) fn template(&self) -> &AuxiliaryWindowTemplate {
        &self.template
    }

    /// 对话框是否真的显示出来了；没显示时调用方不必建输入宿主。
    pub(super) fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.template.hwnd()).as_bool() }
    }

    /// 跑嵌套消息循环直到按钮返回 / 窗口销毁，返回按钮结果（取消 / 关闭为 `-1`）。
    pub(super) fn pump_value(&self) -> i32 {
        if !self.is_visible() {
            return -1;
        }
        let mut message = MSG::default();
        loop {
            let value = self.result.load(Ordering::SeqCst);
            if value != i32::MIN {
                return value;
            }
            if self.drain_pending() {
                continue;
            }
            let received = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if !received.as_bool() {
                if received.0 == 0 {
                    // 把外层的 quit 还回去，别让应用以为消息循环已经结束。
                    unsafe { PostQuitMessage(0) };
                }
                return -1;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if !unsafe { IsWindow(Some(self.template.hwnd())) }.as_bool() {
                return -1;
            }
            self.drain_pending();
        }
    }

    /// 把排队的点击 / 滚轮交给调用方；处理过事件返回 `true`。
    fn drain_pending(&self) -> bool {
        let mut handled = false;
        loop {
            let event = self
                .pending
                .lock()
                .expect("对话框事件队列被毒化")
                .pop_front();
            let Some(event) = event else {
                return handled;
            };
            handled = true;
            if let Some(handler) = self.on_event.borrow_mut().as_mut() {
                handler(
                    &event,
                    &DialogHandle {
                        template: Rc::clone(&self.template),
                        owner: self.owner,
                    },
                );
            }
        }
    }

    /// 老接口：只关心 0–255 的按钮下标；取消 / 关闭都按 0 返回。
    pub(super) fn pump(&self) -> u8 {
        let value = self.pump_value();
        if (0..=i32::from(u8::MAX)).contains(&value) {
            value as u8
        } else {
            0
        }
    }
}

impl Drop for BlockingPanel {
    fn drop(&mut self) {
        for host in &self.edit_hosts {
            host.set_input_blocked(false);
        }
        if self.owner_was_enabled {
            unsafe {
                let _ = EnableWindow(self.owner, true);
            }
        }
        if let Some(focus) = self.previous_focus
            && unsafe { IsWindow(Some(focus)) }.as_bool()
        {
            unsafe {
                let _ = SetFocus(Some(focus));
            }
        }
    }
}
