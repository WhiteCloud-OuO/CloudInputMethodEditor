//! 结果字形预览：把命中的 IDS 画成结构示意图。
//!
//! 系统字体没有这个字的字形时，文字列表只能显示一个方块；悬停结果行时在旁边画一张
//! 动态示意图（运算符切分 + 部件文字 / 缩写），任何已编码结构与 `{n}` 未编码部件都能看见。

use std::collections::HashMap;

use cloud_ui::{Rect, Theme};
use cloudime_ids::{IdsNode, parse};
use cloudime_render::{Element, FontSlot};

use crate::canvas;
use crate::glyph::{GlyphProbe, abbreviation};

/// 预览面板尺寸（点）。
pub const WIDTH: f32 = 210.0;
pub const HEIGHT: f32 = 240.0;

/// 画预览；IDS 解析失败时不画。
#[allow(clippy::too_many_arguments)]
pub fn paint(
    elements: &mut Vec<Element>,
    character: char,
    ids: &str,
    anchor: (f32, f32),
    bounds: Rect,
    theme: &Theme,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    component_ids: &HashMap<char, String>,
    glyphs: &mut GlyphProbe,
) {
    let Ok(node) = parse(ids) else {
        return;
    };
    let mut x = anchor.0 + 18.0;
    let mut y = anchor.1 + 18.0;
    if x + WIDTH > bounds.x + bounds.width {
        x = anchor.0 - WIDTH - 18.0;
    }
    x = x.clamp(bounds.x, (bounds.x + bounds.width - WIDTH).max(bounds.x));
    y = y.clamp(bounds.y, (bounds.y + bounds.height - HEIGHT).max(bounds.y));
    let panel = Rect {
        x,
        y,
        width: WIDTH,
        height: HEIGHT,
    };

    elements.push(Element::Rect {
        x: panel.x,
        y: panel.y,
        width: panel.width,
        height: panel.height,
        radius: 10.0,
        color: theme.border,
    });
    elements.push(Element::Rect {
        x: panel.x + 1.0,
        y: panel.y + 1.0,
        width: panel.width - 2.0,
        height: panel.height - 2.0,
        radius: 9.0,
        color: theme.surface,
    });

    let drawing = Rect {
        x: panel.x + 12.0,
        y: panel.y + 12.0,
        width: panel.width - 24.0,
        height: panel.height - 58.0,
    };
    draw_node(
        elements,
        &node,
        drawing,
        0,
        theme,
        measure,
        component_ids,
        glyphs,
    );

    let mut code = theme.muted();
    code.size = 12.0;
    elements.push(Element::Text {
        x: panel.x + 12.0,
        y: panel.y + panel.height - 40.0,
        text: format!("U+{:04X}", character as u32),
        size: 12.0,
        slot: FontSlot::Candidate,
        color: code.color,
        italic: false,
    });
    let ids_text: String = ids.chars().take(24).collect();
    elements.push(Element::Text {
        x: panel.x + 12.0,
        y: panel.y + panel.height - 22.0,
        text: ids_text,
        size: 12.0,
        slot: FontSlot::Candidate,
        color: theme.text,
        italic: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    elements: &mut Vec<Element>,
    node: &IdsNode,
    rect: Rect,
    depth: usize,
    theme: &Theme,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    component_ids: &HashMap<char, String>,
    glyphs: &mut GlyphProbe,
) {
    if rect.width < 6.0 || rect.height < 6.0 {
        return;
    }
    elements.push(Element::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: 4.0,
        color: theme.border,
    });
    elements.push(Element::Rect {
        x: rect.x + 1.0,
        y: rect.y + 1.0,
        width: rect.width - 2.0,
        height: rect.height - 2.0,
        radius: 3.0,
        color: match node {
            IdsNode::Leaf(_) => theme.surface_alt,
            _ => theme.background,
        },
    });

    match node {
        IdsNode::Leaf(character) => {
            let size = label_size(rect, 0.55, 7.0, 44.0);
            let text = if glyphs.has_glyph(*character) {
                character.to_string()
            } else {
                component_ids
                    .get(character)
                    .map(|ids| {
                        let mut has_glyph = |candidate| glyphs.has_glyph(candidate);
                        abbreviation(ids, 2, component_ids, &mut has_glyph)
                    })
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| format!("{:X}", *character as u32))
            };
            let color = if glyphs.has_glyph(*character) {
                theme.text
            } else {
                theme.muted
            };
            centered(elements, rect, &text, size, color, measure);
        }
        IdsNode::Unencoded(number) => {
            centered(
                elements,
                rect,
                &format!("{{{number}}}"),
                label_size(rect, 0.35, 6.0, 18.0),
                theme.accent,
                measure,
            );
        }
        IdsNode::Wildcard => {
            centered(
                elements,
                rect,
                "？",
                label_size(rect, 0.45, 7.0, 30.0),
                theme.accent,
                measure,
            );
        }
        IdsNode::Op { op, children } => {
            if depth >= 6 {
                centered(
                    elements,
                    rect,
                    "…",
                    label_size(rect, 0.4, 6.0, 16.0),
                    theme.muted,
                    measure,
                );
                return;
            }
            if rect.width >= 30.0 && rect.height >= 22.0 {
                elements.push(Element::Text {
                    x: rect.x + 3.0,
                    y: rect.y + 1.0,
                    text: op.abbreviation().to_owned(),
                    size: 8.0,
                    slot: FontSlot::Candidate,
                    color: theme.muted,
                    italic: false,
                });
            }
            for (index, child) in children.iter().enumerate() {
                let child_rect = canvas::child_rect(*op, index, inset(rect, 2.0), 3.0);
                draw_node(
                    elements,
                    child,
                    child_rect,
                    depth + 1,
                    theme,
                    measure,
                    component_ids,
                    glyphs,
                );
            }
        }
    }
}

fn label_size(rect: Rect, ratio: f32, min: f32, max: f32) -> f32 {
    (rect.width.min(rect.height) * ratio).clamp(min, max)
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - by * 2.0).max(1.0),
        height: (rect.height - by * 2.0).max(1.0),
    }
}

fn centered(
    elements: &mut Vec<Element>,
    rect: Rect,
    text: &str,
    size: f32,
    color: cloudime_render::Color,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let (width, height) = measure(text, FontSlot::Candidate, size);
    elements.push(Element::Text {
        x: rect.x + (rect.width - width).max(0.0) / 2.0,
        y: rect.y + (rect.height - height).max(0.0) / 2.0,
        text: text.to_owned(),
        size,
        slot: FontSlot::Candidate,
        color,
        italic: false,
    });
}
