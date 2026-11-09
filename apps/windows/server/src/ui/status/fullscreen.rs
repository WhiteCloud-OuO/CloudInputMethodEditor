//! 前台是全屏应用（游戏、看视频、演示）时收起状态条，退出全屏再放出来。
//!
//! F11 进全屏不换前台窗口、没有事件可等，状态条显示期间每 300ms 看一次；判定和候选窗共用
//! [`crate::fullscreen`] 的缓存（D3D 独占 + 普通铺满都收）。

use core::cell::Cell;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SW_HIDE, SW_SHOWNA, SetTimer, ShowWindow,
};

use crate::fullscreen::{self, FullscreenKind};

/// 状态条窗口上的定时器编号。
pub(super) const TIMER_ID: usize = 1;

const INTERVAL_MS: u32 = 300;

/// 显示状态条；`auto_hide` 开着时前台正全屏就先收着并开始检查，关着则一直显示、不检查。
pub(super) fn show(hwnd: HWND, hidden: &Cell<bool>, auto_hide: bool) {
    if !auto_hide {
        let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
        hidden.set(false);
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
        return;
    }
    unsafe { SetTimer(Some(hwnd), TIMER_ID, INTERVAL_MS, None) };
    hidden.set(false);
    on_timer(hwnd, hidden);
    if !hidden.get() {
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
    }
}

/// 收起状态条（云朵输入法不在前台 / 关掉了），不再检查。
pub(super) fn hide(hwnd: HWND, hidden: &Cell<bool>) {
    let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
    hidden.set(false);
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
}

/// 每 300ms 一次：进全屏收起，出全屏放回。
pub(super) fn on_timer(hwnd: HWND, hidden: &Cell<bool>) {
    let fullscreen = fullscreen::kind() != FullscreenKind::None;
    if fullscreen != hidden.get() {
        hidden.set(fullscreen);
        let command = if fullscreen { SW_HIDE } else { SW_SHOWNA };
        let _ = unsafe { ShowWindow(hwnd, command) };
    }
}
