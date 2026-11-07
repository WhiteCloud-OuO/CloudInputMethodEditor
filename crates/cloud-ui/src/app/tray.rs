//! 通知区域（托盘）图标：`Shell_NotifyIcon` + 右键菜单。
//!
//! 一个 `TrayIcon` 挂在创建它的应用窗口上，窗口销毁时一起摘掉；图标默认取 exe 内嵌的
//! 应用图标（`winresource` 写入的 ID 1），取不到时退回系统通用图标。
//! 左键 / 双击 / 右键 / 菜单项都通过 `AppEvent::Tray` 交回应用，显示 / 隐藏窗口由应用
//! 调 `Context::show` / `Context::hide`。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON,
    IsIconic, IsWindowVisible, LR_SHARED, LoadImageW, MF_STRING, PostMessageW, SM_CXSMICON,
    SM_CYSMICON, SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

use crate::window::module_handle;
use crate::window_class::WindowClass;

/// 弹出托盘菜单时借用的小窗口类。
static MENU_CLASS: WindowClass = WindowClass::new();

/// 托盘图标收到的鼠标 / 菜单事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    /// 左键单击。
    LeftClick,

    /// 左键双击。
    DoubleClick,

    /// 右键单击；如果配置了菜单，菜单会同时弹出。
    RightClick,

    /// 点了菜单项；内容是 `TrayMenuItem::id`。
    Menu(String),
}

/// 托盘菜单里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    /// 事件里带回来的稳定名字。
    pub id: String,

    /// 菜单上显示的文字。
    pub label: String,
}

impl TrayMenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// 托盘图标配置。
#[derive(Debug, Clone, Default)]
pub struct TrayOptions {
    /// 悬停提示；空时用窗口标题。
    pub tooltip: String,

    /// 右键菜单；空时右键只派事件、不弹菜单。
    pub menu: Vec<TrayMenuItem>,
}

impl TrayOptions {
    /// 带提示文字的配置。
    pub fn new(tooltip: impl Into<String>) -> Self {
        Self {
            tooltip: tooltip.into(),
            menu: Vec::new(),
        }
    }

    /// 设置右键菜单。
    pub fn menu(mut self, items: Vec<TrayMenuItem>) -> Self {
        self.menu = items;
        self
    }
}

/// 一个已经注册的托盘图标。字段只在 UI 线程上使用。
pub(crate) struct TrayIcon {
    owner: HWND,
    data: NOTIFYICONDATAW,
    menu: HMENU,
    /// 菜单命令 ID → `TrayMenuItem::id`。
    commands: Vec<(u32, String)>,
    /// 主窗口隐藏 / 最小化时，菜单借用的离屏前台窗口；`0` 表示还没建。
    menu_owner: Cell<isize>,
}

impl TrayIcon {
    /// 在 `owner` 的通知区域里注册图标。
    pub(crate) fn new(owner: HWND, tooltip: &str, options: &TrayOptions) -> Result<Self> {
        let icon = load_icon()?;

        let menu = unsafe { CreatePopupMenu()? };
        let mut commands = Vec::with_capacity(options.menu.len());
        for (index, item) in options.menu.iter().enumerate() {
            let id = index as u32 + 1;
            let label = HSTRING::from(item.label.as_str());
            if let Err(error) = unsafe { AppendMenuW(menu, MF_STRING, id as usize, &label) } {
                tracing::warn!(%error, "托盘菜单项添加失败");
                continue;
            }
            commands.push((id, item.id.clone()));
        }

        let mut data = NOTIFYICONDATAW {
            cbSize: core::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: owner,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: super::WM_TRAY_ICON,
            hIcon: icon,
            ..Default::default()
        };
        let tip = if tooltip.is_empty() {
            "Cloud UI"
        } else {
            tooltip
        };
        let tip: Vec<u16> = tip.encode_utf16().take(data.szTip.len() - 1).collect();
        data.szTip[..tip.len()].copy_from_slice(&tip);

        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            unsafe {
                let _ = DestroyMenu(menu);
            }
            return Err(Error::from(windows::Win32::Foundation::E_FAIL));
        }
        Ok(Self {
            owner,
            data,
            menu,
            commands,
            menu_owner: Cell::new(0),
        })
    }

    /// Explorer 重启后重新注册（`TaskbarCreated` 消息）。
    pub(crate) fn readd(&self) {
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) }.as_bool() {
            tracing::warn!("托盘图标重新注册失败");
        }
    }

    /// 右键弹出菜单；返回选中的菜单项。必须在 UI 线程调用。
    pub(crate) fn show_menu(&self) -> Option<TrayEvent> {
        if self.commands.is_empty() {
            return None;
        }
        // 主窗口隐藏 / 最小化 / 不在前台时，让它当前台窗口会失败或把窗口提到前面；
        // 借一块离屏但可见的小窗口做 owner，菜单才能正常显示、并在点别处时收起。
        let previous = unsafe { GetForegroundWindow() };
        let usable = previous == self.owner
            && unsafe { IsWindowVisible(self.owner).as_bool() && !IsIconic(self.owner).as_bool() };
        let owner = if usable {
            self.owner
        } else {
            self.menu_owner()?
        };
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            // 菜单在窗口失去前台时会立刻消失；先把自己提到前台，弹完再发一条空消息。
            let _ = SetForegroundWindow(owner);
            let command = TrackPopupMenuEx(
                self.menu,
                TPM_RIGHTBUTTON.0 | TPM_RETURNCMD.0 | TPM_NONOTIFY.0,
                point.x,
                point.y,
                owner,
                None,
            )
            .0;
            let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
            if !previous.is_invalid() && previous != owner {
                let _ = SetForegroundWindow(previous);
            }
            if command <= 0 {
                return None;
            }
            self.commands
                .iter()
                .find(|(id, _)| *id == command as u32)
                .map(|(_, id)| TrayEvent::Menu(id.clone()))
        }
    }

    /// 取菜单用的离屏窗口；第一次需要时创建，之后复用。
    fn menu_owner(&self) -> Option<HWND> {
        let cached = self.menu_owner.get();
        if cached != 0 {
            return Some(HWND(cached as *mut _));
        }
        MENU_CLASS
            .ensure(|| WNDCLASSEXW {
                lpfnWndProc: Some(menu_proc),
                hInstance: module_handle(),
                lpszClassName: w!("CloudUITrayMenu"),
                ..Default::default()
            })
            .ok()?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("CloudUITrayMenu"),
                w!(""),
                WS_POPUP,
                -32000,
                -32000,
                1,
                1,
                None,
                None,
                Some(module_handle()),
                None,
            )
            .ok()?
        };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        self.menu_owner.set(hwnd.0 as isize);
        Some(hwnd)
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
            let _ = DestroyMenu(self.menu);
            let menu_owner = self.menu_owner.get();
            if menu_owner != 0 {
                let _ = DestroyWindow(HWND(menu_owner as *mut _));
            }
        }
    }
}

/// 离屏菜单窗口只负责承接前台激活和菜单消息，其余都交回系统默认处理。
unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 取 exe 内嵌的应用图标（`winresource` 写入的资源 ID 1）。
///
/// 托盘按小图标尺寸显示，直接用 `LoadIconW` 拿 32×32 大图标会被 Shell 缩小得发糊；
/// 这里用 `LoadImageW` 按 `SM_CXSMICON` 取。`LR_SHARED` 让系统缓存句柄，不归我们销毁。
fn load_icon() -> Result<HICON> {
    unsafe {
        let cx = GetSystemMetrics(SM_CXSMICON);
        let cy = GetSystemMetrics(SM_CYSMICON);
        let handle = LoadImageW(
            Some(module_handle()),
            // MAKEINTRESOURCE(1)：资源 ID 1 编成指针。
            PCWSTR(std::ptr::without_provenance::<u16>(1)),
            IMAGE_ICON,
            cx,
            cy,
            LR_SHARED,
        )
        .or_else(|_| LoadImageW(None, IDI_APPLICATION, IMAGE_ICON, cx, cy, LR_SHARED))?;
        Ok(HICON(handle.0))
    }
}

/// 把托盘回调消息里的鼠标事件翻译成 [`TrayEvent`]。
pub(crate) fn click_event(lparam: isize) -> Option<TrayEvent> {
    match (lparam as u32) & 0xFFFF {
        WM_LBUTTONUP => Some(TrayEvent::LeftClick),
        WM_LBUTTONDBLCLK => Some(TrayEvent::DoubleClick),
        WM_RBUTTONUP => Some(TrayEvent::RightClick),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_event_reads_low_word() {
        assert_eq!(
            click_event(WM_LBUTTONUP as isize),
            Some(TrayEvent::LeftClick)
        );
        assert_eq!(
            click_event(WM_LBUTTONDBLCLK as isize),
            Some(TrayEvent::DoubleClick)
        );
        assert_eq!(
            click_event(WM_RBUTTONUP as isize),
            Some(TrayEvent::RightClick)
        );
        assert_eq!(click_event(0), None);
    }

    #[test]
    fn click_event_ignores_high_word() {
        // 通知区域可能在 lparam 高字里带图标 ID；判断只看低字。
        let lparam = ((7u32 << 16) | WM_RBUTTONUP) as isize;
        assert_eq!(click_event(lparam), Some(TrayEvent::RightClick));
    }
}
