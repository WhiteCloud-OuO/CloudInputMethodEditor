//! 窗口壳层的几个小工具：模块句柄、鼠标穿透开关。

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_TRANSPARENT,
};

/// 本进程 exe 的模块句柄（注册窗口类 / 建窗口用）。
pub fn module_handle() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    HINSTANCE(module.0)
}

/// 给窗口开 / 关**鼠标穿透**（`WS_EX_TRANSPARENT` 那一个位）：开着鼠标落到下面的窗口（点不着）。
///
/// 窗口之间「谁接受鼠标」都动这一个位（见 [`crate::auxiliary`] 的焦点）：焦点不在自己身上的一方开着它。
/// 光靠窗口过程回 `HTTRANSPARENT` 不够 —— 那玩意儿只把消息转给**同线程**的下层窗口，而咱底下
/// 通常是别的进程的应用。
pub fn set_mouse_transparent(hwnd: windows::Win32::Foundation::HWND, transparent: bool) {
    let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let style = if transparent {
        current | WS_EX_TRANSPARENT.0
    } else {
        current & !WS_EX_TRANSPARENT.0
    };
    #[cfg(target_pointer_width = "64")]
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize)
    };
    #[cfg(target_pointer_width = "32")]
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as i32)
    };
}

/// 这个窗口这会儿是不是鼠标穿透（窗口过程的 `WM_NCHITTEST` 照它回答，与 [`set_mouse_transparent`]
/// 同一个位）。
pub fn mouse_transparent(hwnd: windows::Win32::Foundation::HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    style & WS_EX_TRANSPARENT.0 != 0
}
