//! 单行 / 多行文本编辑框：`View` 里的外观参数。
//!
//! 文字本身由窗口层的 windowless RichEdit 宿主绘制与编辑（见 `cloud_ui::text_edit`）；
//! 这里只生成 [`cloudime_render::Element::TextEdit`] / [`cloudime_render::Element::TextBox`]，
//! 宿主按 `region_name` 与它对齐。文本内容仍由应用保存，输入变化通过 `AppEvent::TextChanged` 交回。

use cloudime_render::{
    Color, Element, FontSlot, SvgAlign, TextBox as TextBoxElement, TextBoxScroll,
    TextEdit as TextEditElement,
};

/// 文本编辑框的绘制参数（点）。
#[derive(Debug, Clone)]
pub struct TextEditStyle {
    pub height: f32,
    pub radius: f32,

    /// 内部填充；`None` = 透明。
    pub fill: Option<Color>,

    /// 边框颜色；`None` = 渲染器缺省边框色。
    pub border: Option<Color>,

    pub border_width: f32,

    /// 正文颜色；`None` = 主题正文色。
    pub text_color: Option<Color>,

    /// 选区背景；`None` = 主题高亮色。
    pub selection_background: Option<Color>,

    /// 选区文字颜色；`None` = 主题高亮文字色。
    pub selection_color: Option<Color>,

    /// 插入符颜色；`None` = 正文颜色。
    pub caret_color: Option<Color>,

    pub size: f32,
    pub font: FontSlot,
    pub read_only: bool,
}

impl Default for TextEditStyle {
    fn default() -> Self {
        Self {
            height: 34.0,
            radius: 8.0,
            fill: Some(Color::rgb(24, 24, 30)),
            border: Some(Color::rgb(58, 58, 70)),
            border_width: 1.0,
            text_color: Some(Color::rgb(236, 236, 242)),
            selection_background: None,
            selection_color: None,
            caret_color: None,
            size: 14.0,
            font: FontSlot::Candidate,
            read_only: false,
        }
    }
}

impl TextEditStyle {
    /// 固定高度 / 圆角的快捷构造。
    pub fn with_height(height: f32) -> Self {
        Self {
            height,
            ..Self::default()
        }
    }
}

pub(crate) fn measure(
    style: &TextEditStyle,
    constraints: cloud_ui_core::Constraints,
) -> cloud_ui_core::Size {
    cloud_ui_core::Size::new(
        constraints.max_width,
        style.height.min(constraints.max_height),
    )
}

pub(crate) fn build(
    elements: &mut Vec<Element>,
    rect: cloud_ui_core::Rect,
    name: &str,
    text: &str,
    style: &TextEditStyle,
) {
    elements.push(Element::TextEdit(TextEditElement {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: style.radius,
        fill: style.fill,
        border: style.border,
        border_width: style.border_width,
        border_width_fixed: true,
        svg: None,
        svg_size: (0.0, 0.0),
        svg_align: SvgAlign::Left,
        svg_tint: None,
        selection_background: style.selection_background,
        selection_color: style.selection_color,
        caret_color: style.caret_color,
        text_color: style.text_color,
        read_only: style.read_only,
        deselect_on_blur: true,
        passkey: false,
        font_name: None,
        font_flag: 0,
        handlers: Vec::new(),
        text: text.to_owned(),
        font: style.font,
        size: Some(style.size),
        region_name: name.to_owned(),
    }));
}

pub(crate) fn build_box(
    elements: &mut Vec<Element>,
    rect: cloud_ui_core::Rect,
    name: &str,
    text: &str,
    style: &TextEditStyle,
) {
    elements.push(Element::TextBox(TextBoxElement {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: style.radius,
        fill: style.fill,
        border: style.border,
        border_width: style.border_width,
        border_width_fixed: true,
        content_gap: 2.0,
        selection_background: style.selection_background,
        selection_color: style.selection_color,
        caret_color: style.caret_color,
        text_color: style.text_color,
        read_only: style.read_only,
        deselect_on_blur: true,
        auto_line_wrap: true,
        line_spacing: 0,
        font_name: None,
        font_flag: 0,
        handlers: Vec::new(),
        text: text.to_owned(),
        font: style.font,
        size: Some(style.size),
        region_name: name.to_owned(),
        h_scroll: TextBoxScroll::default(),
        v_scroll: TextBoxScroll::default(),
    }));
}

impl super::View {
    /// 加一个单行文本编辑框；输入结果通过 `AppEvent::TextChanged { name, .. }` 交回。
    ///
    /// `text` 是当前文本（应用状态）；宿主会按它同步，用户输入后应用先更新状态再重建视图。
    pub fn text_edit(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
        style: TextEditStyle,
    ) -> cloud_ui_core::NodeId {
        let mut node = cloud_ui_core::Node::new(cloud_ui_core::NodeKind::Control);
        node.width = cloud_ui_core::SizeSpec::Auto;
        node.height = cloud_ui_core::SizeSpec::Fixed(style.height);
        self.push(
            node,
            super::Widget::TextEdit {
                name: name.into(),
                text: text.into(),
                style,
            },
        )
    }

    /// 加一个多行文本编辑框（自动换行、内部滚动条由宿主查询后回写）。
    pub fn text_box(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
        style: TextEditStyle,
    ) -> cloud_ui_core::NodeId {
        let mut node = cloud_ui_core::Node::new(cloud_ui_core::NodeKind::Control);
        node.width = cloud_ui_core::SizeSpec::Auto;
        node.height = cloud_ui_core::SizeSpec::Fixed(style.height);
        self.push(
            node,
            super::Widget::TextBox {
                name: name.into(),
                text: text.into(),
                style,
            },
        )
    }
}
