//! 同步模态对话框的共用宿主。
//!
//! 内置对话框的 `show` 都是同步阻塞的：在当前独立窗口的 UI 上下文里建一块 owned window，
//! 跑嵌套消息循环等按钮点击，期间挡住所有者。这个模块只负责这套壳子，具体内容由调用方
//! 生成 [`Document`] 并给出「区域名 → 返回下标」的映射。

use std::rc::Rc;
use std::sync::Arc;
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

        let result = Arc::new(AtomicI32::new(-1));
        let dialog_hwnd = Arc::new(AtomicIsize::new(template.hwnd().0 as isize));
        template.set_events(Box::new({
            let result = Arc::clone(&result);
            move |event: PanelEvent| {
                if event.action != PanelAction::Click {
                    return;
                }
                let Some(index) = event.region.as_deref().and_then(button) else {
                    return;
                };
                result.store(i32::from(index), Ordering::SeqCst);
                let hwnd = HWND(dialog_hwnd.load(Ordering::SeqCst) as *mut _);
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WAKE, WPARAM(0), LPARAM(0));
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

    /// 跑嵌套消息循环直到按钮返回 / 窗口销毁。
    pub(super) fn pump(&self) -> u8 {
        if !self.is_visible() {
            return 0;
        }
        let mut message = MSG::default();
        loop {
            let value = self.result.load(Ordering::SeqCst);
            if value >= 0 {
                return value as u8;
            }
            let received = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if !received.as_bool() {
                if received.0 == 0 {
                    // 把外层的 quit 还回去，别让应用以为消息循环已经结束。
                    unsafe { PostQuitMessage(0) };
                }
                return 0;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if !unsafe { IsWindow(Some(self.template.hwnd())) }.as_bool() {
                return 0;
            }
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
