//! 任务栏「中 / 英」图标的右键菜单。做法同小狼毫：`TrackPopupMenuEx` 挂在输入框所在窗口上，同步取回点的项。
//! 固定四项：灰显的标题、分隔线、「设置」「重启输入法服务」；中 / 英切换不在这里。

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{HSTRING, PCWSTR};

use cloudime_platform::protocol::IndicatorCommand;

/// 灰显标题的命令 id：给 0，点不到也取不回。
const ID_TITLE: u32 = 0;
const ID_SETTINGS: u32 = 1;
const ID_RESTART: u32 = 2;

/// 在 `point`（屏幕坐标）弹出菜单，阻塞到用户点了某项或点别处关掉。返回选中项要交给 Server 的命令。
pub(crate) fn track(owner: HWND, point: POINT) -> Option<IndicatorCommand> {
    let menu = unsafe { CreatePopupMenu() }.ok()?;
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED,
            ID_TITLE as usize,
            &HSTRING::from("云朵输入法"),
        )
    };
    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_SETTINGS as usize,
            &HSTRING::from("设置"),
        )
    };
    let _ = unsafe {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_RESTART as usize,
            &HSTRING::from("重启输入法服务"),
        )
    };
    // 图标在任务栏上，菜单往上弹
    let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN;
    let id = unsafe { TrackPopupMenuEx(menu, flags.0, point.x, point.y, owner, None) };
    let _ = unsafe { DestroyMenu(menu) };
    match id.0 as u32 {
        ID_SETTINGS => Some(IndicatorCommand::OpenSettings),
        ID_RESTART => Some(IndicatorCommand::RestartServer),
        _ => None,
    }
}
