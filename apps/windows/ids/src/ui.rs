//! 小部件：面板、标签、按钮、文本编辑框。全部直接写 `Document`，不经过 `View`。

use cloud_ui::{LabelStyle, Rect, TextEditStyle, Theme};
use cloudime_render::{Color, Element, FontSlot, HitShape, Region};

use crate::composer::Composer;

/// 按钮配色。
#[derive(Debug, Clone, Copy)]
pub struct ButtonLook {
    pub size: f32,
    pub slot: FontSlot,
    pub text: Color,
    pub background: Color,
    pub hover: Color,
    pub radius: f32,
}

impl ButtonLook {
    /// 工具行 / 普通按钮：灰底。
    pub fn normal(theme: &Theme) -> Self {
        Self {
            size: 13.0,
            slot: FontSlot::Candidate,
            text: theme.text,
            background: theme.surface_alt,
            hover: theme.border,
            radius: 6.0,
        }
    }

    /// 主按钮 / 选中态：背景取主题的强调色，`theme.rs` 把它映射成候选窗口的 `extra`。
    pub fn primary(theme: &Theme) -> Self {
        Self {
            size: 14.0,
            slot: FontSlot::Candidate,
            text: theme.accent_text,
            background: theme.accent,
            hover: theme.accent,
            radius: 6.0,
        }
    }

    /// 结构按钮：运算符与整颗按钮的文字都用结构字体。
    ///
    /// 组字工具把「序号」槽借给结构符号（窗口里没有序号），`main.rs` 把这一槽的字族
    /// 配成霞鹜文楷；本机没装时那一槽就是配置里的普通字体，外观与 [`Self::normal`] 一致。
    pub fn structure(theme: &Theme) -> Self {
        Self {
            slot: FontSlot::ItemNumber,
            ..Self::normal(theme)
        }
    }
}

/// 一块圆角底色。
pub fn panel(elements: &mut Vec<Element>, rect: Rect, color: Color, radius: f32) {
    elements.push(Element::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius,
        color,
    });
}

/// 左对齐、垂直居中的一行文字。
pub fn label(
    elements: &mut Vec<Element>,
    rect: Rect,
    text: &str,
    style: &LabelStyle,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let (_, height) = measure(text, style.slot, style.size);
    elements.push(Element::Text {
        x: rect.x,
        y: rect.y + (rect.height - height).max(0.0) / 2.0,
        text: text.to_owned(),
        size: style.size,
        slot: style.slot,
        color: style.color,
        italic: false,
    });
}

/// 一个按钮：底色 + 居中文字 + 命中区域。
#[allow(clippy::too_many_arguments)]
pub fn button(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    rect: Rect,
    name: &str,
    text: &str,
    look: &ButtonLook,
    pointer: Option<(f32, f32)>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let hovered = pointer.is_some_and(|(x, y)| inside(rect, x, y));
    panel(
        elements,
        rect,
        if hovered { look.hover } else { look.background },
        look.radius,
    );
    let (width, height) = measure(text, look.slot, look.size);
    elements.push(Element::Text {
        x: rect.x + (rect.width - width).max(0.0) / 2.0,
        y: rect.y + (rect.height - height).max(0.0) / 2.0,
        text: text.to_owned(),
        size: look.size,
        slot: look.slot,
        color: look.text,
        italic: false,
    });
    regions.push(Region::new(
        name,
        HitShape::RoundRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius: look.radius,
        },
        false,
    ));
}

/// 单行文本编辑框；宿主由窗口壳按元素自动创建。
pub fn text_edit(
    elements: &mut Vec<Element>,
    rect: Rect,
    name: &str,
    text: &str,
    style: &TextEditStyle,
) {
    cloud_ui::view::text_edit::build(elements, rect, name, text, style);
}

/// IDS 表达式行：完整就显示表达式，否则显示第一条错误。
pub fn expression_text(composer: &Composer) -> (String, bool) {
    match composer.to_ids() {
        Ok(ids) => (format!("表达式：{ids}"), false),
        Err(error) => (format!("表达式不完整：{error}"), true),
    }
}

/// 鼠标是否在矩形里。
pub fn inside(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
}

/// 构造一个标签样式（标题 / 状态行）。
pub fn muted_style(theme: &Theme) -> LabelStyle {
    let mut style = theme.muted();
    style.size = 13.0;
    style
}
