//! 一段文字怎么画：字号、行高（像素）、颜色、删除线 / 下划线。

use crate::color::Color;
use crate::theme::FontSpec;

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// 字号（像素）。
    pub size: f32,

    /// 字号（点）：光学字号与字距表按点查。
    pub points: f32,

    /// 行高（像素）。
    pub line_height: f32,

    /// 字族名；`None` 用系统界面字体。
    pub family: Option<String>,

    pub color: Color,

    /// 画删除线（纠错改掉的拼音）。
    pub strike: bool,

    /// 画下划线（候选右侧辅助码里已经敲上的那几位）。
    pub underline: bool,

    /// 斜体（翻译 Tip 的词性）。
    pub italic: bool,

    /// 字重（100–900，400 为常规）。
    pub weight: u16,

    /// 覆盖率 gamma，见 `Theme::text_gamma`。
    pub gamma: f32,
}

impl TextStyle {
    /// `font` 已按倍数换成像素；`points` 是换算前的字号。
    pub(crate) fn new(font: FontSpec, points: f32, color: Color, gamma: f32) -> Self {
        Self {
            size: font.size,
            points,
            line_height: font.line_height,
            family: None,
            color,
            strike: false,
            underline: false,
            italic: false,
            weight: 400,
            gamma,
        }
    }

    /// 指定字族。
    pub(crate) fn with_family(mut self, family: Option<&String>) -> Self {
        self.family = family.cloned();
        self
    }

    /// 指定字重。
    pub(crate) fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight.clamp(100, 900);
        self
    }

    /// 斜体。
    pub(crate) fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub(crate) fn struck(mut self) -> Self {
        self.strike = true;
        self
    }

    /// 下划线（辅助码里已经敲上的那几位）。
    pub(crate) fn underlined(mut self) -> Self {
        self.underline = true;
        self
    }
}
