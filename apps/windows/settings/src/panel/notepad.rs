//! 用记事本打开脚本文件；「新建脚本」还要把模板文本直接塞进它的文档区。
//!
//! 塞文本只对经典记事本有效：它的文档区是一个 `Edit` 子控件，收 `WM_SETTEXT` 就等于替换正文。
//! Windows 11 商店版记事本是 WinUI 应用、没有 `Edit` 子控件，塞不进去 —— 文件里已经写好模板，
//! 用户看到的内容不受影响，只是少这一下兜底（记一条日志）。
//!
//! 等窗口出来与填字都在后台线程做：这两步要轮询几百毫秒，别卡住设置界面。

use std::path::Path;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    SendMessageW, WM_SETTEXT,
};
use windows::core::{BOOL, w};

use crate::log;

/// 等记事本窗口出来的上限与轮询间隔（毫秒）。
const WAIT_LIMIT_MS: u32 = 4000;
const POLL_MS: u32 = 100;

/// 记事本主窗口的类名（不受界面语言影响）；装正文的子控件类名是 `Edit`。
const NOTEPAD_CLASS: &str = "Notepad";

/// 用记事本打开 `path`，不碰它的内容：「编辑此脚本」。
pub(crate) fn open(path: &Path) {
    if let Err(error) = std::process::Command::new("notepad.exe").arg(path).spawn() {
        log::warn(format!("打开记事本失败：{error}"));
    }
}

/// 用记事本打开 `path`，并把 `text` 塞进它的文档区：「新建脚本」。
pub(crate) fn open_with_text(path: &Path, text: &str) {
    let child = match std::process::Command::new("notepad.exe").arg(path).spawn() {
        Ok(child) => child,
        Err(error) => {
            log::warn(format!("打开记事本失败：{error}"));
            return;
        }
    };
    // 进程号要和句柄分开拿：句柄马上丢掉没关系（Windows 上关句柄不结束那个进程）。
    let pid = child.id();
    drop(child);
    let path = path.to_path_buf();
    let text = text.to_owned();
    std::thread::spawn(move || {
        let mut left = WAIT_LIMIT_MS.div_ceil(POLL_MS);
        while left > 0 {
            if let Some(edit) = document_edit(pid) {
                set_text(edit, &text);
                return;
            }
            unsafe { Sleep(POLL_MS) };
            left -= 1;
        }
        log::warn(format!(
            "没等到记事本的文档区，{} 的模板没塞进去（文件里已经写好，不影响用）",
            path.display()
        ));
    });
}

/// 记事本装正文的那个 `Edit` 子控件。
///
/// 优先找**我们刚起的那只**（按进程号）：机器上可能本来就开着别的记事本，按类名找会把模板塞错窗口。
/// 商店版记事本会把活交给已经在跑的实例、我们的进程号不见了，这时才按类名兜底。
fn document_edit(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        ours: HWND,
        any: HWND,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` 是下面传进来的 `&mut Search`，回调期间一直有效。
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() || !is_notepad(hwnd) {
            return true.into();
        }
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == search.pid {
            search.ours = hwnd;
            return false.into();
        }
        if search.any.0.is_null() {
            search.any = hwnd;
        }
        true.into()
    }

    let mut search = Search {
        pid,
        ours: HWND(std::ptr::null_mut()),
        any: HWND(std::ptr::null_mut()),
    };
    unsafe {
        let _ = EnumWindows(
            Some(visit),
            LPARAM(std::ptr::from_mut(&mut search).cast::<core::ffi::c_void>() as isize),
        );
    }
    let main = if search.ours.0.is_null() {
        search.any
    } else {
        search.ours
    };
    if main.0.is_null() {
        return None;
    }
    unsafe { FindWindowExW(Some(main), None, w!("Edit"), None) }
        .ok()
        .filter(|edit| !edit.0.is_null())
}

/// 这个顶层窗口是不是记事本主窗口（按类名认，不看标题）。
fn is_notepad(hwnd: HWND) -> bool {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    length > 0 && String::from_utf16_lossy(&buffer[..length as usize]) == NOTEPAD_CLASS
}

/// 把文本写进控件的正文区（`WM_SETTEXT` 就是「换掉里面那段文字」）。
fn set_text(edit: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SendMessageW(
            edit,
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(wide.as_ptr().cast::<core::ffi::c_void>() as isize)),
        );
    }
    // `SendMessage` 是同步的：返回时控件已经拷走文本，`wide` 可以释放。
}
