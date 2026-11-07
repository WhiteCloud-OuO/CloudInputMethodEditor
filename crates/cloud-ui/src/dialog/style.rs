//! 对话框实例的可见属性。
//!
//! 所有内置对话框都是类：`new` 建实例，属性在 `show` 之前改，`show(self)` 吃掉实例。
//! 颜色和字号只在这里定义一份，消息对话框与文本输入对话框共用。

use cloudime_render::{Color, FontSlot};

use crate::view::Theme;

/// 对话框实例的可见属性。
///
/// 缺省值都跟随主题；只有显式设过的项才覆盖。`show` 之前改才有效：Rust 的
/// [`show`](crate::MessageDialog::show) 吃掉实例，脚本侧 `:show()` 之后再改属性会报 Lua 错误。
#[derive(Debug, Clone, Default)]
pub struct DialogStyle {
    /// 正文 / 标题 / 按钮用的字体槽。
    pub font: FontSlot,

    /// 正文与按钮字号；`None` = 跟主题缺省（14 点）。
    pub text_size: Option<f32>,

    /// 标题字号；`None` = 标题栏缺省（与 demo 主窗口一致）。
    pub title_size: Option<f32>,

    /// 面板底色；`None` = 主题背景色。
    pub background: Option<Color>,

    /// 标题文字颜色；`None` = 主题次要文字色。
    pub title_color: Option<Color>,

    /// 正文文字颜色；`None` = 主题正文色。
    pub text_color: Option<Color>,

    /// 按钮底色；`None` = 主题强调色。
    pub button_background: Option<Color>,

    /// 按钮文字颜色；`None` = 主题强调文字色。
    pub button_text_color: Option<Color>,
}

impl DialogStyle {
    /// 正文 / 按钮实际字号。
    pub(super) fn text_size(&self) -> f32 {
        self.text_size.unwrap_or(super::layout::BODY_SIZE)
    }

    /// 把颜色覆盖写进主题副本；字号与字体槽由各对话框单独取。
    pub(super) fn theme(&self, theme: &Theme) -> Theme {
        let mut theme = *theme;
        if let Some(color) = self.background {
            theme.background = color;
        }
        if let Some(color) = self.text_color {
            theme.text = color;
        }
        if let Some(color) = self.title_color {
            theme.muted = color;
        }
        if let Some(color) = self.button_background {
            theme.accent = color;
        }
        if let Some(color) = self.button_text_color {
            theme.accent_text = color;
        }
        theme
    }
}

/// 脚本文本与 Rust 实例共用同一组属性；脚本侧 DTO 到这里转一次。
#[cfg(feature = "script")]
impl From<&cloudime_script::DialogStyle> for DialogStyle {
    fn from(style: &cloudime_script::DialogStyle) -> Self {
        Self {
            font: style.font,
            text_size: style.text_size,
            title_size: style.title_size,
            background: style.background,
            title_color: style.title_color,
            text_color: style.text_color,
            button_background: style.button_background,
            button_text_color: style.button_text_color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_properties_keep_the_theme() {
        let base = Theme::dark();
        let theme = DialogStyle::default().theme(&base);
        assert_eq!(theme.background, base.background);
        assert_eq!(theme.text, base.text);
        assert_eq!(theme.muted, base.muted);
        assert_eq!(theme.accent, base.accent);
        assert_eq!(theme.accent_text, base.accent_text);
    }

    #[test]
    fn set_properties_override_the_theme() {
        let base = Theme::dark();
        let style = DialogStyle {
            background: Some(Color::rgb(1, 2, 3)),
            text_color: Some(Color::rgb(4, 5, 6)),
            title_color: Some(Color::rgb(7, 8, 9)),
            button_background: Some(Color::rgb(10, 11, 12)),
            button_text_color: Some(Color::rgb(13, 14, 15)),
            ..Default::default()
        };
        let theme = style.theme(&base);
        assert_eq!(theme.background, Color::rgb(1, 2, 3));
        assert_eq!(theme.text, Color::rgb(4, 5, 6));
        assert_eq!(theme.muted, Color::rgb(7, 8, 9));
        assert_eq!(theme.accent, Color::rgb(10, 11, 12));
        assert_eq!(theme.accent_text, Color::rgb(13, 14, 15));
    }
}
