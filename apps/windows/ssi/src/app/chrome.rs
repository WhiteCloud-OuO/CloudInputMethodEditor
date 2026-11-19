//! 自绘标题栏：标题 + 关闭按钮（原版没有最小化）。
//!
//! 窗口用 `Decorations::None` + `caption_height`，这一条 36 点高的区域由窗口壳负责拖动；
//! 关闭按钮自己的命中区域不可拖动，所以点按钮不会变成拖窗口。

use cloud_ui::{CLOSE_ACTION, Theme, TitleBarIcons};
use cloudime_render::{Element, FontSlot, HitShape, Region};

use crate::ui;

/// 标题栏高度（与 `WindowOptions.caption_height` 一致）。
pub const HEIGHT: f32 = 36.0;

/// 画标题栏并登记关闭按钮区域。
pub fn paint(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    width: f32,
    title: &str,
    theme: &Theme,
    pointer: Option<(f32, f32)>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let reference = if crate::theme::is_light(theme) {
        Theme::light()
    } else {
        Theme::dark()
    };
    let style = reference.title_bar();

    let mut label = theme.label();
    label.size = style.title_size;
    label.color = style.title_color;
    ui::label(
        elements,
        cloud_ui::Rect {
            x: style.padding.left,
            y: 0.0,
            width: (width - style.padding.left - style.icon.width - 8.0).max(1.0),
            height: HEIGHT,
        },
        title,
        &label,
        measure,
    );

    let icons = TitleBarIcons::default();
    let rect = cloud_ui::Rect {
        x: width - style.padding.right - style.icon.width,
        y: (HEIGHT - style.icon.height) / 2.0,
        width: style.icon.width,
        height: style.icon.height,
    };
    let hovered = pointer.is_some_and(|(x, y)| ui::inside(rect, x, y));
    if hovered {
        ui::panel(
            elements,
            rect,
            style.close_hover_background,
            style.icon.radius,
        );
    }
    let color = if hovered {
        style.close_hover_icon
    } else {
        style.icon.icon
    };
    let size = style.icon.icon_size;
    elements.push(Element::Svg {
        x: rect.x + (rect.width - size) / 2.0,
        y: rect.y + (rect.height - size) / 2.0,
        size,
        source: icons.close.to_owned(),
        tint: Some(color),
    });
    regions.push(Region::new(
        CLOSE_ACTION,
        HitShape::RoundRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius: style.icon.radius,
        },
        false,
    ));
}
