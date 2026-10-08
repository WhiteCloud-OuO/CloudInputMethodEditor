//! 颜色选择对话框：WinUI 3 的 `ContentDialog` + 一个 `ColorPicker`，按钮「确定 / 取消」。
//!
//! 用法（设置页「主题」页在用）：
//! 1. 状态里放一个 [`ColorDialog`]；
//! 2. `view` 里把 [`ColorDialog::view`] 放进树里（`is_open` 由状态决定）；
//! 3. `update` 里收两个回调消息：
//!    - `on_color` 来的 [`Color`] → [`ColorDialog::set_color`]；
//!    - `on_closed` 来的 [`ContentDialogResult`] → [`ColorDialog::close`]；
//!      `Primary`（确定）时用 [`ColorDialog::color`] 取选中的颜色，`Secondary` / `None` 是取消。
//!
//! 属性说明（`windows-reactor 0.100` 的 `ColorPicker` 只暴露一部分）：
//! `is_alpha_enabled` / `is_color_slider_visible` / `is_hex_input_visible` 显式设成要的值；
//! `IsColorPreviewVisible=true`、`IsMoreButtonVisible=false`、`Orientation=Vertical` 本来就是
//! WinUI 的默认值，不必设；`ColorSpectrumShape=Ring` 没暴露，暂时设不了（用默认的 `Box`）。
use windows_reactor::*;

/// 颜色选择对话框的状态：开着没有、当前挑的颜色。
#[derive(Clone)]
pub(crate) struct ColorDialog {
    open: bool,
    color: Color,
}

impl ColorDialog {
    /// 建一个（先收起）。`initial` 是打开前用的颜色。
    pub(crate) fn new(initial: Color) -> Self {
        Self {
            open: false,
            color: initial,
        }
    }

    /// 打开，并带上要编辑的颜色（一般传当前值）。
    pub(crate) fn open(&mut self, color: Color) {
        self.color = color;
        self.open = true;
    }

    /// 收起（确定与取消之后都要调）。
    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// 当前颜色（「确定」时它就是结果）。
    pub(crate) fn color(&self) -> Color {
        self.color
    }

    /// `ColorPicker` 改动时更新。
    pub(crate) fn set_color(&mut self, color: Color) {
        self.color = color;
    }

    /// 画出来。`on_color` 收 `ColorPicker` 的改动；`on_closed` 收对话框结果 ——
    /// `ContentDialogResult::Primary` 是「确定」，这时用 [`Self::color`] 取选中的颜色。
    pub(crate) fn view(
        &self,
        title: impl Into<String>,
        on_color: Callback<Color>,
        on_closed: Callback<ContentDialogResult>,
    ) -> View {
        ContentDialog::new()
            .title(title)
            .is_open(self.open)
            .primary_button_text("确定")
            .secondary_button_text("取消")
            .close_button_text_optional(None::<String>)
            .on_closed(on_closed)
            .content(
                ColorPicker::new()
                    .color(self.color)
                    .is_alpha_enabled(true)
                    .is_color_slider_visible(true)
                    .is_hex_input_visible(true)
                    .on_color_changed(on_color),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 打开带上初始色 → 改色 → 收起后 `color()` 就是改后的色。
    #[test]
    fn the_dialog_keeps_the_color_it_closed_with() {
        let mut dialog = ColorDialog::new(Color::rgb(0, 0, 0));
        dialog.open(Color::rgb(1, 2, 3));
        dialog.set_color(Color::rgb(4, 5, 6));
        dialog.close();
        assert_eq!(dialog.color(), Color::rgb(4, 5, 6));
    }
}
