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
//!
//! **焦点**：每一块独立窗口创建并显示后**总是具备焦点**（这一组的焦点缺省就在它身上），而且它**都**
//! 实现了「指定资产获取焦点」（[`DesignatedAssetToGetFocus`]）—— 可以把焦点指定给名下某件声明了
//! `able_to_get_focus` 的资产，或者收回来。焦点不在自己身上时整块让开：自己那块窗鼠标穿透
//! （`WS_EX_TRANSPARENT`，[`super::set_mouse_transparent`]），窗口过程的 `WM_NCHITTEST` 也按
//! [`Self::focused`] 回 `HTTRANSPARENT`（点不着、滚轮也不是它的）。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result};

use super::auxiliary::{Assets, DesignatedAssetToGetFocus, Focus};
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

    /// 这一组的焦点在不在**自己**（独立窗口）身上。窗口过程里接鼠标的那几处（`WM_NCHITTEST`）
    /// 按它让位：焦点被指定给名下的资产时，所有者整块窗鼠标穿透、不再接受鼠标。
    pub(super) fn focused(&self) -> bool {
        self.assets.focus() == Focus::Owner
    }

    /// 收起名下的资产（宿主收起时调）：资产表会先**自动归还焦点**（§4），之后自己这块窗的鼠标开关
    /// 也跟着回位 —— 否则会留下一块「看不见的窗还占着鼠标穿透」的状态。
    pub(super) fn hide_assets(&self) {
        self.assets.hide();
        self.focusing();
    }

    /// 自己那块窗的**鼠标开关**跟当前焦点对齐：焦点不在自己身上时整块让开（鼠标穿透到下面的窗口）。
    /// 焦点换人（[`DesignatedAssetToGetFocus`]）与资产收起之后都要调一遍。
    fn sync_input(&self) {
        super::set_mouse_transparent(self.hwnd, !self.focused());
    }
}

/// **独立窗口都具备**「指定资产获取焦点」（`designated_asset_to_get_focus`）这个特性 —— 由模板实现，
/// 所以每块独立窗口都有：焦点从自己身上**指定**给名下某件资产，或者**收回来**（见 [`Focus`]）。
impl DesignatedAssetToGetFocus for IndependentWindowTemplate {
    fn designate_focus(&self, name: &str) -> bool {
        let moved = self.assets.designate_focus(name);
        if moved {
            // 焦点交给了名下的资产：自己**失焦**
            self.unfocusing();
        }
        moved
    }

    fn return_focus_to_owner(&self) -> bool {
        let moved = self.assets.return_focus_to_owner();
        if moved {
            // 焦点回到自己身上：**聚焦**
            self.focusing();
        }
        moved
    }

    fn focus(&self) -> Focus {
        self.assets.focus()
    }

    /// **聚焦**：焦点在自己身上，接鼠标。独立窗口创建并显示后就是这个状态。
    fn focusing(&self) {
        self.sync_input();
    }

    /// **失焦**：把焦点交给了名下某件资产 —— 自己那块窗整块让开（鼠标穿透）。
    fn unfocusing(&self) {
        self.sync_input();
    }
}

impl Drop for IndependentWindowTemplate {
    fn drop(&mut self) {
        // 资产的寿命到宿主销毁**之前**为止：先放掉它们，再销毁自己这块窗
        self.assets.retire();
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
