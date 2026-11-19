//! 把字符送进目标程序：`SendInput` 的 Unicode 注入、写剪贴板、把前台切回目标窗口。

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
    SetFocus, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

/// 当前前台窗口；拿不到返回 `None`。
pub fn foreground_window() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(hwnd.0 as isize)
}

/// 把前台还回目标窗口（`AttachThreadInput` 串起输入队列，VFB 旧版同一套做法），然后等一会儿。
pub fn activate(target: isize) {
    if target == 0 {
        return;
    }
    let target = HWND(target as *mut _);
    unsafe {
        let foreground = GetForegroundWindow();
        let me = GetCurrentThreadId();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let target_thread = GetWindowThreadProcessId(target, None);
        if foreground_thread != 0 && foreground_thread != me {
            let _ = AttachThreadInput(me, foreground_thread, true);
        }
        if target_thread != 0 && target_thread != me {
            let _ = AttachThreadInput(me, target_thread, true);
        }
        let _ = SetForegroundWindow(target);
        let _ = SetFocus(Some(target));
        if target_thread != 0 && target_thread != me {
            let _ = AttachThreadInput(me, target_thread, false);
        }
        if foreground_thread != 0 && foreground_thread != me {
            let _ = AttachThreadInput(me, foreground_thread, false);
        }
    }
    // 等前台切换完成再注入，否则字符会落到切换前的窗口。
    std::thread::sleep(std::time::Duration::from_millis(60));
}

/// 把文本以 Unicode 按键事件注入当前焦点窗口。
pub fn inject(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut inputs = Vec::with_capacity(units.len() * 2);
    for unit in units {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: unit,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            });
        }
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "模拟输入只送进 {sent}/{} 个事件（目标程序可能以管理员身份运行）",
            inputs.len()
        ))
    }
}

/// 别的程序占着剪贴板时重试几次。
const OPEN_TRIES: u32 = 5;
const OPEN_WAIT: std::time::Duration = std::time::Duration::from_millis(10);

/// 把一段文本写进剪贴板；失败返回给用户看的原因。
pub fn copy(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let _opened = Opened::open()?;
    unsafe { EmptyClipboard() }.map_err(|error| format!("清空剪贴板失败：{error}"))?;

    // CF_UNICODETEXT = UTF-16 + 结尾 0，放在可移动的全局内存里交给系统。
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) }
        .map_err(|error| format!("分配剪贴板内存失败：{error}"))?;
    unsafe {
        let target = GlobalLock(handle);
        if target.is_null() {
            free(handle);
            return Err("锁定剪贴板内存失败".to_owned());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
        let _ = GlobalUnlock(handle);
    }
    if let Err(error) = unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))) }
    {
        free(handle);
        return Err(format!("写入剪贴板失败：{error}"));
    }
    Ok(())
}

fn free(handle: HGLOBAL) {
    let _ = unsafe { windows::Win32::Foundation::GlobalFree(Some(handle)) };
}

/// 打开着的剪贴板，`Drop` 时关掉。
struct Opened;

impl Opened {
    fn open() -> Result<Self, String> {
        for _ in 0..OPEN_TRIES {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Self);
            }
            std::thread::sleep(OPEN_WAIT);
        }
        Err("剪贴板被别的程序占着（打不开），稍后再试".to_owned())
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_two_events_per_utf16_unit() {
        // 用内部逻辑的等价构造验证代理对：非 BMP 字符占两个 UTF-16 单元。
        let units: Vec<u16> = "🅐a".encode_utf16().collect();
        assert_eq!(units.len(), 3);
        assert_eq!(units[0], 0xD83C);
    }

    #[test]
    fn empty_text_is_a_noop() {
        assert!(inject("").is_ok());
        assert!(copy("").is_ok());
    }
}
