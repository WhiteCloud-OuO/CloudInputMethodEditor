//! 任务栏「中 / 英」图标右键菜单（见 [`crate::com::mode::menu`]）落到文本服务上。

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId,
};

use cloudime_platform::protocol::IndicatorCommand;

use super::TextService_Impl;
use crate::com::log::log;
use crate::com::mode::menu;

impl TextService_Impl {
    pub(super) fn show_indicator_menu(&self, point: POINT) {
        let Some(owner) = self.menu_owner() else {
            log("右键菜单：找不到本线程的窗口，不弹");
            return;
        };
        if let Some(command) = menu::track(owner, point) {
            self.send_indicator(command);
        }
    }

    pub(super) fn send_indicator(&self, command: IndicatorCommand) {
        if matches!(
            command,
            IndicatorCommand::OpenSettings | IndicatorCommand::OpenDownload
        ) {
            // 设置程序 / 浏览器由 Server 起；前台权在点菜单的这边，让出去它的窗口才能到前面
            let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        }
        match self.engine.borrow_mut().as_mut() {
            Some(client) => {
                if let Err(error) = client.indicator(command) {
                    log(&format!("给 Server 发指示器指令失败: {error}"));
                }
            }
            None => log("指示器指令：没连上 Server"),
        }
    }

    /// 菜单要挂在本线程的窗口上：先取焦点输入框所在窗口，没有再看前台窗口是不是本线程的。
    fn menu_owner(&self) -> Option<HWND> {
        let from_context = self
            .thread_mgr
            .borrow()
            .as_ref()
            .and_then(|thread_mgr| unsafe {
                let view = thread_mgr
                    .GetFocus()
                    .ok()?
                    .GetTop()
                    .ok()?
                    .GetActiveView()
                    .ok()?;
                view.GetWnd().ok()
            });
        from_context
            .or_else(|| {
                let foreground = unsafe { GetForegroundWindow() };
                let thread = unsafe { GetWindowThreadProcessId(foreground, None) };
                (thread == unsafe { GetCurrentThreadId() }).then_some(foreground)
            })
            .filter(|hwnd| !hwnd.is_invalid())
    }
}
