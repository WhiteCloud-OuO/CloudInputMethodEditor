//! 剪贴板：脚本的 `cloudime.clipboard.settext` / `gettext`。
//!
//! 为什么在 Server 做：Lua 的标准库里没有剪贴板，而 Server 跑在**用户会话**里（不是 AppContainer），
//! 直接用 Win32 的剪贴板 API 就行 —— 不必像早先那份示例那样借 `powershell Set-Clipboard` 绕一圈。
//!
//! 剪贴板是全局资源：别的进程正开着它时 `OpenClipboard` 会失败，这里短等重试几次；还不行就报一句
//! 原因，让脚本自己决定（`pcall` 或忽略）。**读进来的内容不设防** —— 剪贴板里可能有别的程序（比如
//! 密码管理器）刚放进去的东西，这是「用户自己写的脚本、风险自担」那一档，`lua.md` 里写明了。

use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 打开剪贴板的尝试次数与间隔：别的进程通常只占几毫秒。
const OPEN_TRIES: u32 = 5;
const OPEN_WAIT: Duration = Duration::from_millis(10);

/// 写一段文本进剪贴板（覆盖原来的内容）。失败返回一句给用户看的原因。
pub(crate) fn set_text(text: &str) -> Result<(), String> {
    let _open = Opened::open()?;
    unsafe { EmptyClipboard() }.map_err(|error| format!("清空剪贴板失败：{error}"))?;

    // CF_UNICODETEXT 要的是「UTF-16 + 结尾的 0」，放在可移动的全局内存里交给系统
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
    // 交给剪贴板之后这份内存归系统管：失败才需要自己释放
    if let Err(error) = unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))) }
    {
        free(handle);
        return Err(format!("写入剪贴板失败：{error}"));
    }
    Ok(())
}

/// 读剪贴板里的文本：没有文本（图片 / 文件 / 空）返回 `None`，打不开返回原因。
pub(crate) fn get_text() -> Result<Option<String>, String> {
    let _open = Opened::open()?;
    let Ok(handle) = (unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }) else {
        return Ok(None);
    };
    let global = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(global) };
    if pointer.is_null() {
        return Err("锁定剪贴板内存失败".to_owned());
    }
    let text = unsafe {
        let units = pointer.cast::<u16>();
        let mut length = 0usize;
        while *units.add(length) != 0 {
            length += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(units, length));
        let _ = GlobalUnlock(global);
        text
    };
    Ok((!text.is_empty()).then_some(text))
}

/// 释放一块全局内存（失败就算了，这里已经在错误路径上）。
fn free(handle: HGLOBAL) {
    let _ = unsafe { windows::Win32::Foundation::GlobalFree(Some(handle)) };
}

/// 打开着的剪贴板：`Drop` 时关掉，别把全局资源留着。
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

    /// 写进去、读出来是同一段文本（中文 / 换行 / emoji 都过一遍）。
    /// 剪贴板是全局资源：先把原来的存起来、测完放回去；要是在 CI 这种没有剪贴板的会话里打不开，
    /// 就按「环境不支持」跳过。
    #[test]
    fn set_then_get_round_trips() {
        let saved = get_text().ok().flatten();
        let sample = "剪贴板测试 abc\n第二行 😀";
        if set_text(sample).is_err() {
            return; // 打不开剪贴板的环境（比如无会话的 CI）跳过
        }
        assert_eq!(get_text().unwrap().as_deref(), Some(sample));

        // 放回原来的（原来是空的就清成空）
        let _ = set_text(saved.as_deref().unwrap_or(""));
    }
}
