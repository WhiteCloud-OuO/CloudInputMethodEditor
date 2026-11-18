//! 把切好的槽位画成圆角盒子：边框、部件字、结构角标、命中区域。

use cloud_ui::{Rect, Theme};
use cloudime_render::{Element, FontSlot, HitShape, Region};

use crate::canvas::layout::{Placed, layout};
use crate::composer::{Composer, Node};

/// 槽位区域名前缀。
pub const SLOT_PREFIX: &str = "slot:";

/// 缺字形时的画法。
pub enum FallbackGlyph {
    /// 用内置 SVG 画。
    Svg(String),

    /// 用一段文字（例如 `U+20087`）。
    Text(String),
}

/// 画整个画布。
#[allow(clippy::too_many_arguments)]
pub fn paint(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    composer: &Composer,
    rect: Rect,
    theme: &Theme,
    pointer: Option<(f32, f32)>,
    fallback: &mut dyn FnMut(char) -> Option<FallbackGlyph>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let placed = layout(composer.root(), rect, 8.0);
    let hovered = pointer.and_then(|(x, y)| {
        placed
            .iter()
            .rev()
            .find(|placed| contains(placed.rect, x, y))
            .map(|placed| placed.path.clone())
    });
    for placed in &placed {
        paint_node(
            elements,
            regions,
            composer,
            placed,
            theme,
            hovered.as_deref() == Some(placed.path.as_slice()),
            fallback,
            measure,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_node(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    composer: &Composer,
    placed: &Placed,
    theme: &Theme,
    hovered: bool,
    fallback: &mut dyn FnMut(char) -> Option<FallbackGlyph>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let Some(node) = composer.node_at(&placed.path) else {
        return;
    };
    let rect = placed.rect;
    let active = composer.active() == placed.path.as_slice();
    let border = if active || hovered {
        theme.accent
    } else {
        theme.border
    };
    let fill = match node {
        Node::Op { .. } => theme.surface,
        Node::Empty => theme.background,
        Node::Char(_) | Node::Wildcard => theme.surface_alt,
    };

    // 外圈当边框、内圈当底色：比为每个盒子加描边省一个元素类型。
    let radius = 8.0;
    elements.push(Element::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius,
        color: border,
    });
    let border_width = if active { 2.0 } else { 1.0 };
    elements.push(Element::Rect {
        x: rect.x + border_width,
        y: rect.y + border_width,
        width: (rect.width - border_width * 2.0).max(1.0),
        height: (rect.height - border_width * 2.0).max(1.0),
        radius: (radius - border_width).max(0.0),
        color: fill,
    });

    let side = rect.width.min(rect.height);
    match node {
        Node::Empty => {
            centered_text(
                elements,
                rect,
                "＋",
                (side * 0.38).clamp(8.0, 34.0),
                theme.muted,
                measure,
            );
        }
        Node::Char(character) => match fallback(*character) {
            Some(FallbackGlyph::Svg(source)) => {
                let size = (side * 0.62).clamp(8.0, 96.0);
                elements.push(Element::Svg {
                    x: rect.x + (rect.width - size) / 2.0,
                    y: rect.y + (rect.height - size) / 2.0,
                    size,
                    source,
                    tint: Some(theme.text),
                });
            }
            Some(FallbackGlyph::Text(label)) => {
                centered_text(
                    elements,
                    rect,
                    &label,
                    (side * 0.24).clamp(8.0, 18.0),
                    theme.muted,
                    measure,
                );
            }
            None => {
                centered_text(
                    elements,
                    rect,
                    &character.to_string(),
                    (side * 0.62).clamp(8.0, 96.0),
                    theme.text,
                    measure,
                );
            }
        },
        Node::Wildcard => {
            centered_text(
                elements,
                rect,
                "？",
                (side * 0.55).clamp(8.0, 80.0),
                theme.accent,
                measure,
            );
        }
        Node::Op { op, .. } => {
            if rect.width >= 34.0 && rect.height >= 24.0 {
                elements.push(Element::Text {
                    x: rect.x + 6.0,
                    y: rect.y + 3.0,
                    text: format!("{} {}", op.symbol(), op.abbreviation()),
                    size: 10.0,
                    slot: FontSlot::Candidate,
                    color: theme.muted,
                    italic: false,
                });
            }
        }
    }

    regions.push(Region::new(
        slot_name(&placed.path),
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

fn centered_text(
    elements: &mut Vec<Element>,
    rect: Rect,
    text: &str,
    size: f32,
    color: cloudime_render::Color,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let slot = FontSlot::Candidate;
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

fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height
}

/// 槽位路径 → 区域名：根是 `slot:root`，其余 `slot:0.1.2`。
pub fn slot_name(path: &[usize]) -> String {
    if path.is_empty() {
        return format!("{SLOT_PREFIX}root");
    }
    let parts: Vec<String> = path.iter().map(usize::to_string).collect();
    format!("{SLOT_PREFIX}{}", parts.join("."))
}

/// 区域名 → 槽位路径；不是槽位返回 `None`。
pub fn parse_slot(name: &str) -> Option<Vec<usize>> {
    let rest = name.strip_prefix(SLOT_PREFIX)?;
    if rest == "root" {
        return Some(Vec::new());
    }
    rest.split('.')
        .map(|part| part.parse::<usize>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_names_round_trip() {
        assert_eq!(slot_name(&[]), "slot:root");
        assert_eq!(slot_name(&[0, 2]), "slot:0.2");
        assert_eq!(parse_slot("slot:root"), Some(vec![]));
        assert_eq!(parse_slot("slot:0.2"), Some(vec![0, 2]));
        assert_eq!(parse_slot("slot:x"), None);
        assert_eq!(parse_slot("other:0"), None);
    }
}
