//! **独立窗口模板**（`independent_window_template`）：给「独立窗口」（`docs/notes/crate-notes.md`
//! 「窗口的两类」）共用的那点壳子。
//!
//! 建出来的是**一块空壳窗**：
//!
//! - 注册窗口类（窗口过程由调用方给）并建一块**分层、置顶、不抢焦点**的弹出窗
//!   （`WS_EX_LAYERED | TOOLWINDOW | TOPMOST | NOACTIVATE`；纯展示的再加 `TRANSPARENT` 让鼠标穿透）；
//! - 管**名下的资产**（辅助窗口，见 [`super::auxiliary`]）：独立窗口把自己那块窗建好之后再
//!   [`Self::assets_mut`] 往里加，`Drop` 里**先**放掉资产、**再**销毁自己这块窗 ——
//!   资产的寿命就落在这段区间里。
//!
//! 画什么、怎么摆、什么时候显示，仍由各独立窗口自己定（候选窗口见 `ui/candidates`，
//! 悬浮工具栏见 `ui/status`，状态切换提示见 `ui/status_tip`）。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result};

use super::auxiliary::Assets;
use super::module_handle;
use super::window_class::WindowClass;

/// 独立窗口的窗口过程：每块独立窗口自己实现一个（要按 HWND 找回自己的状态）。
pub(super) type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// 独立窗口模板建出来的那块空壳窗。
pub(super) struct IndependentWindowTemplate {
    /// 自己那块分层窗。
    hwnd: HWND,

    /// 名下的资产（辅助窗口）。
    assets: Assets,
}

impl IndependentWindowTemplate {
    /// 建一块独立窗口：注册 `class`（窗口过程 = `wndproc`），再建一个隐藏的弹出窗。
    ///
    /// `transparent` = 纯展示、鼠标穿透；`cursor` 给要接鼠标的窗口（候选窗口）一个箭头。
    pub(super) fn new(
        class: &'static WindowClass,
        class_name: PCWSTR,
        title: PCWSTR,
        wndproc: WndProc,
        transparent: bool,
        cursor: Option<HCURSOR>,
    ) -> Result<Self> {
        class.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            hInstance: module_handle(),
            hCursor: cursor.unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        })?;
        let mut style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        if transparent {
            style |= WS_EX_TRANSPARENT;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                class_name,
                title,
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module_handle()),
                None,
            )?
        };
        Ok(Self {
            hwnd,
            assets: Assets::default(),
        })
    }

    /// 自己那块窗。
    pub(super) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 名下的资产（辅助窗口）：**建好自己这块窗之后**往里加。
    pub(super) fn assets_mut(&mut self) -> &mut Assets {
        &mut self.assets
    }

    /// 见 [`Self::assets_mut`]。
    pub(super) fn assets(&self) -> &Assets {
        &self.assets
    }
}

impl Drop for IndependentWindowTemplate {
    fn drop(&mut self) {
        // 资产的寿命到宿主销毁**之前**为止：先放掉它们，再销毁自己这块窗
        self.assets.retire();
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
