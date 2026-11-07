//! 独立窗口蒙版：盖在客户区上的一层逐像素 alpha 分层窗口。
//!
//! 蒙版不是控件，也不进 `Document`：它是一块单独的顶层窗口，位于主窗口与其
//! RichEdit 宿主之上，`WM_NCHITTEST` 回 `HTTRANSPARENT`，鼠标 / 键盘仍落到下面的
//! 控件。蒙版本身用和窗口背景同一套圆角几何生成位图，再经 `UpdateLayeredWindow`
//! 贴上去，所以圆角边缘的 alpha 与窗口背景对齐，不会漏出未覆盖的像素。

use std::cell::{Cell, RefCell};

use cloudime_render::{Color, Pixmap, rounded_rect_pixmap};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, HWND_TOP, HWND_TOPMOST,
    MA_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowPos, ShowWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSEXW, WS_DISABLED,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Result, w};

use crate::layered;
use crate::window::module_handle;
use crate::window_class::WindowClass;

/// 窗口类只在首次创建蒙版时注册。
static CLASS: WindowClass = WindowClass::new();

/// 独立窗口的内容蒙版。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowMask {
    /// 蒙版颜色；RGB 是颜色，`a` 是整体不透明度（0 全透明，255 完全不透明）。
    pub color: Color,
}

impl WindowMask {
    /// 用任意颜色做蒙版。
    pub const fn new(color: Color) -> Self {
        Self { color }
    }

    /// 黑色蒙版；`strength` 从 0 到 1，越大越暗。
    pub fn dim(strength: f32) -> Self {
        let alpha = (strength.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(Color::rgba(0, 0, 0, alpha))
    }
}

/// 一张已生成的蒙版位图；尺寸 / 圆角 / 颜色没变时复用。
struct MaskContent {
    width: u32,
    height: u32,
    radius: f32,
    color: Color,
    pixmap: Pixmap,
}

/// 真正的蒙版窗口。字段只在创建它的 UI 线程上使用。
pub(crate) struct MaskWindow {
    hwnd: HWND,
    color: Color,
    content: RefCell<Option<MaskContent>>,
    position: Cell<Option<(i32, i32)>>,
}

impl MaskWindow {
    /// 在 `owner` 的内容区上建一块隐藏的蒙版窗。
    pub(crate) fn new(owner: HWND, top_most: bool, color: Color) -> Result<Self> {
        CLASS.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(mask_proc),
            hInstance: module_handle(),
            lpszClassName: w!("CloudUIWindowMask"),
            ..Default::default()
        })?;

        let mut ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        if top_most {
            ex_style |= WS_EX_TOPMOST;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                w!("CloudUIWindowMask"),
                w!(""),
                WS_POPUP | WS_DISABLED,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(module_handle()),
                None,
            )?
        };
        Ok(Self {
            hwnd,
            color,
            content: RefCell::new(None),
            position: Cell::new(None),
        })
    }

    /// 蒙版窗口句柄（独立顶层窗口；所有者移动时由 dialog 带着走）。
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 更新颜色与整体不透明度；下一帧 `set_bounds` 会重新生成位图。
    pub(crate) fn set_color(&mut self, color: Color) {
        self.color = color;
    }

    /// 摆到屏幕坐标 `(x, y)`、`width × height`，并按 `radius` 生成位图。
    pub(crate) fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32, radius: f32) {
        let width = width.max(1) as u32;
        let height = height.max(1) as u32;
        let mut content = self.content.borrow_mut();
        let needs_content = content.as_ref().is_none_or(|current| {
            current.width != width
                || current.height != height
                || current.radius != radius
                || current.color != self.color
        });
        if needs_content {
            let Some(pixmap) = rounded_rect_pixmap(width, height, radius, self.color) else {
                return;
            };
            *content = Some(MaskContent {
                width,
                height,
                radius,
                color: self.color,
                pixmap,
            });
        }
        if !needs_content && self.position.get() == Some((x, y)) {
            return;
        }
        if let Some(content) = content.as_ref() {
            if layered::present(self.hwnd, &content.pixmap, (x, y), 255).is_ok() {
                self.position.set(Some((x, y)));
            } else {
                // 下一次调用强制重试，避免内容换了但位置没变时一直跳过。
                self.position.set(None);
            }
        }
    }

    /// 提到同层窗口的最前面（主窗口的 RichEdit 宿主会反复 `HWND_TOP`）。
    pub(crate) fn raise(&self, top_most: bool) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(insert_after(top_most)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// 显示 / 隐藏；只由主窗口的可见性驱动。
    pub(crate) fn show(&self, show: bool) {
        unsafe {
            let _ = ShowWindow(self.hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
}

impl Drop for MaskWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn insert_after(top_most: bool) -> HWND {
    if top_most { HWND_TOPMOST } else { HWND_TOP }
}

unsafe extern "system" fn mask_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // 点击穿到下面的 RichEdit 宿主或主窗口，键盘也永远不会到这块窗口。
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_maps_strength_to_alpha() {
        assert_eq!(WindowMask::dim(0.0).color.a, 0);
        assert_eq!(WindowMask::dim(1.0).color.a, 255);
        assert_eq!(WindowMask::dim(0.5).color.a, 128);
        assert_eq!(WindowMask::dim(-1.0).color.a, 0);
        assert_eq!(WindowMask::dim(2.0).color.a, 255);
    }

    #[test]
    fn dim_is_black() {
        let color = WindowMask::dim(0.5).color;
        assert_eq!((color.r, color.g, color.b), (0, 0, 0));
    }
}
