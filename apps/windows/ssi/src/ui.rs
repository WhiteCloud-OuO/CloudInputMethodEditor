//! 小部件：面板、标签、按钮、符号格子。全部直接写 `Document`，不经过 `View`。

use cloud_ui::{LabelStyle, Rect, TextEditStyle, Theme};
use cloudime_render::{Color, Element, FontSlot, HitShape, Region};

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
    /// 标签页 / 普通按钮：灰底。
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

    /// 选中的标签页 / 主按钮：强调色底。
    pub fn primary(theme: &Theme) -> Self {
        Self {
            size: 13.0,
            slot: FontSlot::Candidate,
            text: theme.accent_text,
            background: theme.accent,
            hover: theme.accent,
            radius: 6.0,
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

/// 居中文字（结构 / 符号格子用）。
pub fn centered(
    elements: &mut Vec<Element>,
    rect: Rect,
    text: &str,
    size: f32,
    slot: FontSlot,
    color: Color,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let (width, height) = measure(text, slot, size);
    elements.push(Element::Text {
        x: rect.x + (rect.width - width).max(0.0) / 2.0,
        y: rect.y + (rect.height - height).max(0.0) / 2.0,
        text: text.to_owned(),
        size,
        slot,
        color,
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
    centered(
        elements, rect, text, look.size, look.slot, look.text, measure,
    );
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

/// 一个符号格子：底色、悬停描边、符号，并登记命中区域。
#[allow(clippy::too_many_arguments)]
pub fn tile(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    rect: Rect,
    name: &str,
    text: &str,
    slot: FontSlot,
    theme: &Theme,
    pointer: Option<(f32, f32)>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let hovered = pointer.is_some_and(|(x, y)| inside(rect, x, y));
    let radius = 6.0;
    panel(
        elements,
        rect,
        if hovered {
            theme.surface_alt
        } else {
            theme.background
        },
        radius,
    );
    if hovered {
        elements.push(Element::Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius,
            color: theme.accent,
        });
        panel(
            elements,
            Rect {
                x: rect.x + 1.0,
                y: rect.y + 1.0,
                width: (rect.width - 2.0).max(1.0),
                height: (rect.height - 2.0).max(1.0),
            },
            theme.surface_alt,
            radius - 1.0,
        );
    }
    // 单字符按格子宽度的 58% 给字号；「浊゛」这种两字缩小一号。
    let characters = text.chars().count().max(1) as f32;
    let size = if characters > 1.0 {
        (rect.width * 0.38).max(8.0)
    } else {
        (rect.width * 0.58).clamp(10.0, 30.0)
    };
    centered(elements, rect, text, size, slot, theme.text, measure);
    regions.push(Region::new(
        name,
        HitShape::RoundRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius,
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

/// 鼠标是否在矩形里。
pub fn inside(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
}

/// 次要文字样式。
pub fn muted(theme: &Theme, size: f32) -> LabelStyle {
    let mut style = theme.muted();
    style.size = size;
    style
}
