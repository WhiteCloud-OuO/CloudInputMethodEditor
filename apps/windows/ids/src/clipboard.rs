//! 写剪贴板：结果双击后用。
//!
//! 与 Server 的脚本剪贴板同一套 Win32 调用；这里只需要写，不需要读。

use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 别的程序占着剪贴板时重试几次。
const OPEN_TRIES: u32 = 5;
const OPEN_WAIT: Duration = Duration::from_millis(10);

/// 把一段文本写进剪贴板；失败返回给用户看的原因。
pub fn set_text(text: &str) -> Result<(), String> {
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
