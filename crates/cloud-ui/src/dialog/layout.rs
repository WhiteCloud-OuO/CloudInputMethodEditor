//! 内置对话框共用的尺寸、折行与按钮绘制。
//!
//! 所有对话框都是同一套三段式：标题栏 / 内容区 / 按钮组。尺寸与排版只在这里定义一份，
//! 具体对话框只描述内容区里放什么。

use cloudime_render::{Color, Element, FontSlot, HitShape, Region, TextBlock};

use crate::painter::Painter;
use crate::view::{ButtonStyle, TitleBarStyle};

/// 对话框宽度（点）。
pub(super) const WIDTH: f32 = 380.0;

/// 内容区左右与底部留白。
pub(super) const PADDING: f32 = 20.0;

/// 内容区与按钮组之间的间距。
pub(super) const SECTION_GAP: f32 = 12.0;

/// 正文字号。
pub(super) const BODY_SIZE: f32 = 14.0;

/// 正文行距。
pub(super) const LINE_SPACING: f32 = 4.0;

/// 按钮组尺寸。
pub(super) const BUTTON_HEIGHT: f32 = 32.0;
pub(super) const BUTTON_GAP: f32 = 8.0;
pub(super) const BUTTON_MIN_WIDTH: f32 = 72.0;

/// 标题栏文字：与 `cloud-ui-demo` 主窗口一致（无关闭按钮、下方不画分隔线）。
pub(super) fn title(
    painter: &mut Painter,
    style: &TitleBarStyle,
    text: &str,
    font: FontSlot,
    size: f32,
) -> Element {
    let (_, height) = painter.measure_text("国", font, size);
    Element::Text {
        x: style.padding.left,
        y: (style.height - height).max(0.0) / 2.0,
        text: text.to_owned(),
        size,
        slot: font,
        color: style.title_color,
        italic: false,
    }
}

/// 一段按宽度自动折行的正文；返回元素和它占的高度。
#[allow(clippy::too_many_arguments)]
pub(super) fn body(
    painter: &mut Painter,
    text: &str,
    x: f32,
    y: f32,
    width: f32,
    color: Color,
    font: FontSlot,
    size: f32,
) -> (Element, f32) {
    let (_, line_height) = painter.measure_text("国", font, size);
    let lines = wrap_text(painter, text, width, font, size);
    let height = (lines.len() as f32 * line_height
        + (lines.len().saturating_sub(1)) as f32 * LINE_SPACING)
        .max(line_height);
    let element = Element::TextBlock(TextBlock {
        x,
        y,
        size,
        slot: font,
        line_spacing: LINE_SPACING,
        color,
        text: lines.join("\n"),
    });
    (element, height)
}

/// 按宽度把文本拆行；输入里的 `\n` 是硬换行。
pub(super) fn wrap_text(
    painter: &mut Painter,
    text: &str,
    max_width: f32,
    font: FontSlot,
    size: f32,
) -> Vec<String> {
    wrap_with(text, max_width, |line| {
        painter.measure_text(line, font, size).0
    })
}

/// 按钮宽度：文字宽 + 左右留白，不小于 [`BUTTON_MIN_WIDTH`]。
pub(super) fn button_width(
    painter: &mut Painter,
    style: &ButtonStyle,
    label: &str,
    font: FontSlot,
) -> f32 {
    let (width, _) = painter.measure_text(label, font, style.size);
    (width + style.padding_x * 2.0).max(BUTTON_MIN_WIDTH)
}

/// 在 `x` / `y` 画一个文字按钮，返回它的命中区域。
#[allow(clippy::too_many_arguments)]
pub(super) fn push_button(
    painter: &mut Painter,
    elements: &mut Vec<Element>,
    style: &ButtonStyle,
    x: f32,
    y: f32,
    width: f32,
    label: &str,
    region: String,
    font: FontSlot,
) -> Region {
    let (text_width, text_height) = painter.measure_text(label, font, style.size);
    elements.push(Element::Rect {
        x,
        y,
        width,
        height: BUTTON_HEIGHT,
        radius: style.radius,
        color: style.background,
    });
    elements.push(Element::Text {
        x: x + (width - text_width).max(0.0) / 2.0,
        y: y + (BUTTON_HEIGHT - text_height).max(0.0) / 2.0,
        text: label.to_owned(),
        size: style.size,
        slot: font,
        color: style.color,
        italic: false,
    });
    Region::new(
        region,
        HitShape::RoundRect {
            x,
            y,
            width,
            height: BUTTON_HEIGHT,
            radius: style.radius,
        },
        false,
    )
}

/// [`wrap_text`] 的纯函数部分：量宽由调用方给，便于单测。
fn wrap_with(text: &str, max_width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        for ch in paragraph.chars() {
            let mut candidate = line.clone();
            candidate.push(ch);
            if measure(&candidate) > max_width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line.push(ch);
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_respects_newlines_and_width() {
        let lines = wrap_with("abc\ndef", 100.0, |line| line.chars().count() as f32 * 10.0);
        assert_eq!(lines, ["abc", "def"]);

        let lines = wrap_with("abcdef", 25.0, |line| line.chars().count() as f32 * 10.0);
        assert_eq!(lines, ["ab", "cd", "ef"]);

        let lines = wrap_with("", 25.0, |line| line.chars().count() as f32 * 10.0);
        assert_eq!(lines, [""]);
    }
}
