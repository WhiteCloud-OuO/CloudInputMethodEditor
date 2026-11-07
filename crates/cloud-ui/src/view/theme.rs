//! `View` 的主题：一组颜色、圆角与字号，外加从它派生的控件样式。
//!
//! 主题只影响 `View` 生成的控件；IME 那套候选窗 / 状态条主题仍是
//! `cloudime-platform` 的 `ThemeFile`，两者互不替换。

use cloudime_render::{Color, FontSlot};

use super::{
    ButtonStyle, IconButtonStyle, LabelStyle, ListBoxStyle, PanelStyle, ScrollablePageStyle,
    TextEditStyle, TitleBarStyle,
};

/// 独立应用的颜色与尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// 整窗背景。
    pub background: Color,

    /// 面板 / 列表盒底色。
    pub surface: Color,

    /// 次要底色（分隔、条纹）。
    pub surface_alt: Color,

    /// 边框。
    pub border: Color,

    /// 正文。
    pub text: Color,

    /// 次要文字。
    pub muted: Color,

    /// 强调色（按钮、选中行）。
    pub accent: Color,

    /// 强调色上的文字。
    pub accent_text: Color,

    /// 控件通用圆角。
    pub radius: f32,

    /// 正文 / 标签字号。
    pub font_size: f32,

    /// 标题字号。
    pub title_size: f32,

    /// 次要文字字号。
    pub muted_size: f32,

    /// 按钮高度。
    pub button_height: f32,

    /// 列表行高。
    pub row_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// 深色（与 IME 候选窗接近的一套灰蓝）。
    pub const fn dark() -> Self {
        Self {
            background: Color::rgb(24, 24, 30),
            surface: Color::rgb(30, 30, 38),
            surface_alt: Color::rgb(40, 40, 50),
            border: Color::rgb(58, 58, 70),
            text: Color::rgb(236, 236, 242),
            muted: Color::rgb(150, 150, 162),
            accent: Color::rgb(48, 108, 220),
            accent_text: Color::rgb(255, 255, 255),
            radius: 12.0,
            font_size: 16.0,
            title_size: 26.0,
            muted_size: 13.0,
            button_height: 34.0,
            row_height: 30.0,
        }
    }

    /// 浅色。
    pub const fn light() -> Self {
        Self {
            background: Color::rgb(245, 245, 247),
            surface: Color::rgb(255, 255, 255),
            surface_alt: Color::rgb(236, 236, 240),
            border: Color::rgb(208, 208, 213),
            text: Color::rgb(28, 28, 30),
            muted: Color::rgb(110, 110, 118),
            accent: Color::rgb(10, 102, 216),
            accent_text: Color::rgb(255, 255, 255),
            radius: 12.0,
            font_size: 16.0,
            title_size: 26.0,
            muted_size: 13.0,
            button_height: 34.0,
            row_height: 30.0,
        }
    }

    /// 标题标签。
    pub fn title(&self) -> LabelStyle {
        LabelStyle {
            size: self.title_size,
            slot: FontSlot::Candidate,
            color: self.text,
        }
    }

    /// 正文标签。
    pub fn label(&self) -> LabelStyle {
        LabelStyle {
            size: self.font_size,
            slot: FontSlot::Candidate,
            color: self.text,
        }
    }

    /// 次要说明。
    pub fn muted(&self) -> LabelStyle {
        LabelStyle {
            size: self.muted_size,
            slot: FontSlot::Candidate,
            color: self.muted,
        }
    }

    /// 强调按钮。
    pub fn button(&self) -> ButtonStyle {
        ButtonStyle {
            size: 14.0,
            slot: FontSlot::Candidate,
            color: self.accent_text,
            background: self.accent,
            radius: self.radius.min(8.0),
            padding_x: 16.0,
            height: self.button_height,
        }
    }

    /// 面板。
    pub fn panel(&self) -> PanelStyle {
        PanelStyle {
            background: self.surface,
            radius: self.radius,
            padding: cloud_ui_core::Edges::all(16.0),
        }
    }

    /// 列表盒（固定视口高度）。
    pub fn list_box(&self, height: f32) -> ListBoxStyle {
        ListBoxStyle {
            height,
            row_height: self.row_height,
            gap: 2.0,
            auto_item_gap_averaging: true,
            radius: self.radius.min(10.0),
            background: self.surface,
            border: self.border,
            border_width: 1.0,
            content_gap: 4.0,
            padding_x: 10.0,
            row_radius: 6.0,
            size: 14.0,
            muted_size: 12.0,
            font: FontSlot::Candidate,
            text: self.text,
            muted: self.muted,
            selection: self.accent,
            selection_text: self.accent_text,
            hover: self.surface_alt,
        }
    }

    /// 单行文本编辑框。
    pub fn text_edit(&self) -> TextEditStyle {
        TextEditStyle {
            height: 34.0,
            radius: self.radius.min(8.0),
            fill: Some(self.surface),
            border: Some(self.border),
            border_width: 1.0,
            text_color: Some(self.text),
            selection_background: Some(self.accent),
            selection_color: Some(self.accent_text),
            caret_color: Some(self.text),
            size: 14.0,
            font: FontSlot::Candidate,
            read_only: false,
        }
    }

    /// 多行文本编辑框（固定高度）。
    pub fn text_box(&self, height: f32) -> TextEditStyle {
        TextEditStyle {
            height,
            ..self.text_edit()
        }
    }

    /// 可滚动页面（控件容器）的样式（固定视口高度）。
    pub fn scrollable_page(&self, height: f32) -> ScrollablePageStyle {
        ScrollablePageStyle {
            height,
            background: Some(self.surface),
            radius: self.radius.min(10.0),
            gap: 8.0,
            scrollbar_width: 6.0,
            track: Color::rgba(self.muted.r, self.muted.g, self.muted.b, 70),
            thumb: Color::rgba(self.muted.r, self.muted.g, self.muted.b, 170),
            ..ScrollablePageStyle::with_height(height)
        }
    }

    /// 自绘标题栏（36 点高，按钮用内嵌 SVG）。
    pub fn title_bar(&self) -> TitleBarStyle {
        TitleBarStyle {
            height: 36.0,
            background: None,
            title_size: 13.0,
            title_color: self.muted,
            gap: 4.0,
            icon: IconButtonStyle {
                width: 38.0,
                height: 28.0,
                radius: 6.0,
                icon_size: 12.0,
                icon: self.text,
                hover_background: self.surface_alt,
                hover_icon: self.text,
                anti_alias: true,
            },
            close_hover_background: Color::rgb(196, 43, 28),
            close_hover_icon: Color::rgb(255, 255, 255),
            ..TitleBarStyle::default()
        }
    }
}
