//! 候选窗显示期间把 Caps Lock 临时当退格用。
//!
//! 系统在按键消息送到 TSF 之前就翻转了大写锁定状态，TSF 的 `OnTestKeyDown` 里把
//! `VK_CAPITAL` 吃掉也拦不住切换，所以这里用 `WH_KEYBOARD_LL`：低级钩子在系统更新
//! 按键状态之前跑，返回 1 就不会切换大小写。命中后注入一次 `VK_BACK`，退格照常走
//! TSF → Server 那条链路（行内拼音与候选窗一起更新），不绕过组句直接改文档。
//!
//! 只在候选窗显示着时生效。TSF 自己「Caps Lock 亮着时单击 Shift」会补发一次扫描码为
//! 0 的 Caps Lock 来清锁定（`clear_caps_lock`），那种注入必须放行，否则会变成退格。

use std::cell::Cell;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
};

thread_local! {
    /// 当前装着的钩子。只在 UI 线程上装 / 卸（候选窗显示 / 收起时）。
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
}

/// 候选窗显示着没有。UI 线程在 `show` / `hide` 时写，钩子回调在同一线程读。
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// 候选窗弹出时所在的前台窗口：切到别的程序后候选窗还没收干净时，别把 Caps Lock 吞掉。
static FOREGROUND: AtomicIsize = AtomicIsize::new(0);

/// 上一次 Caps Lock 按下被吃掉了：配对的抬起也要吃，别把半截按键还给应用。
static SWALLOW_UP: AtomicBool = AtomicBool::new(false);

/// 候选窗显示 / 收起时更新开关。钩子只在显示期间存在，避免在没组句时也拦截系统按键。
pub(crate) fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
    if on {
        FOREGROUND.store(
            unsafe { GetForegroundWindow().0 as isize },
            Ordering::Relaxed,
        );
        install();
    } else {
        FOREGROUND.store(0, Ordering::Relaxed);
        SWALLOW_UP.store(false, Ordering::Relaxed);
        uninstall();
    }
}

fn install() {
    HOOK.with(|hook| {
        if hook.get().is_some() {
            return;
        }
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) } {
            Ok(installed) => {
                hook.set(Some(installed));
                tracing::debug!("候选窗 Caps Lock 转退格钩子已装");
            }
            Err(error) => {
                tracing::warn!(%error, "装 Caps Lock 转退格钩子失败，候选窗里 Caps Lock 仍按大写切换");
            }
        }
    });
}

fn uninstall() {
    HOOK.with(|hook| {
        if let Some(installed) = hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(installed) };
        }
    });
}

/// UI 线程退出前收尾；候选窗先收的话这里通常已经是空的。
pub(crate) fn shutdown() {
    ACTIVE.store(false, Ordering::Relaxed);
    FOREGROUND.store(0, Ordering::Relaxed);
    SWALLOW_UP.store(false, Ordering::Relaxed);
    uninstall();
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: `nCode >= 0` 时系统保证 `lParam` 指向一个 `KBDLLHOOKSTRUCT`。
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if event.vkCode == u32::from(VK_CAPITAL.0) {
            let key_down = wparam.0 == WM_KEYDOWN as usize;
            let key_up = wparam.0 == WM_KEYUP as usize;
            let expected = FOREGROUND.load(Ordering::Relaxed);
            let same_foreground =
                expected != 0 && unsafe { GetForegroundWindow().0 as isize } == expected;
            if should_swallow(
                CapsKey {
                    scan_code: event.scanCode,
                    injected: event.flags.contains(LLKHF_INJECTED),
                    key_down,
                },
                ACTIVE.load(Ordering::Relaxed),
                modifier_down(),
                same_foreground,
            ) {
                SWALLOW_UP.store(true, Ordering::Relaxed);
                send_backspace();
                return LRESULT(1);
            }
            if key_up && SWALLOW_UP.swap(false, Ordering::Relaxed) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 这一按键该不该吞掉换成退格。单独抽出来，测试不用真的装钩子。
#[derive(Debug, Clone, Copy)]
struct CapsKey {
    scan_code: u32,
    injected: bool,
    key_down: bool,
}

fn should_swallow(
    event: CapsKey,
    active: bool,
    modifier_down: bool,
    same_foreground: bool,
) -> bool {
    if !active || !same_foreground || !event.key_down || modifier_down {
        return false;
    }
    // TSF `clear_caps_lock` 的补发：扫描码 0 的注入 Caps Lock，放行。
    // 其它注入（自动化 / 键盘重映射）带正常扫描码，照样当退格。
    !(event.injected && event.scan_code == 0)
}

/// Shift / Ctrl / Alt / Win 按着时 Caps Lock 原样放行，避免影响组合键。
fn modifier_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0)
}

/// 注入一次退格。扫描码 `0x0E` 是 `VK_BACK` 的 PC 扫描码，Chromium 系宿主也认。
fn send_backspace() {
    let key = |released: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_BACK,
                wScan: 0x0E,
                dwFlags: if released {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(false), key(true)];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        tracing::warn!(sent, "Caps Lock 转退格：SendInput 没有全部发出");
    }
}

#[cfg(test)]
mod tests {
    use super::{CapsKey, should_swallow};

    fn caps(scan_code: u32, injected: bool, key_down: bool) -> CapsKey {
        CapsKey {
            scan_code,
            injected,
            key_down,
        }
    }

    #[test]
    fn caps_lock_acts_as_backspace_only_while_the_candidate_window_is_open() {
        assert!(should_swallow(caps(0x3A, false, true), true, false, true));
        assert!(!should_swallow(caps(0x3A, false, true), false, false, true));
    }

    #[test]
    fn key_up_and_modifier_combinations_pass_through() {
        assert!(!should_swallow(caps(0x3A, false, false), true, false, true));
        assert!(!should_swallow(caps(0x3A, false, true), true, true, true));
    }

    #[test]
    fn another_foreground_window_passes_through() {
        assert!(!should_swallow(caps(0x3A, false, true), true, false, false));
    }

    #[test]
    fn the_ime_caps_lock_clear_injection_is_not_swallowed() {
        // TSF 的 `clear_caps_lock`：注入、扫描码 0。
        assert!(!should_swallow(caps(0, true, true), true, false, true));
        // 自动化 / 重映射工具带正常扫描码的注入仍按 Caps Lock 处理。
        assert!(should_swallow(caps(0x3A, true, true), true, false, true));
    }
}
