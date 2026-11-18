//! 自绘标题栏：标题、最小化 / 关闭按钮。
//!
//! 窗口用 `Decorations::None` + `caption_height`，这一条 36 点高的区域由窗口壳负责拖动与双击最大化；
//! 按钮自己的命中区域是不可拖动的，所以点按钮不会变成拖窗口。
//!
//! 按钮尺寸与配色对齐 `cloud-ui-demo` 的浅色 / 深色主题：38×28、圆角 6、图标 12，
//! 悬停用主题 `surface_alt`，关闭按钮悬停为红色。

use cloud_ui::{CLOSE_ACTION, MINIMIZE_ACTION, Theme, TitleBarIcons};
use cloudime_render::{Element, FontSlot, HitShape, Region, WindowGlyph};

use crate::ui;

/// 标题栏高度（与 `WindowOptions.caption_height` 一致）。
pub const HEIGHT: f32 = 36.0;

/// 画标题栏并登记两个按钮区域。
#[allow(clippy::too_many_arguments)]
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
            width: (width - style.padding.left - style.padding.right - 38.0 * 2.0 - style.gap)
                .max(1.0),
            height: HEIGHT,
        },
        title,
        &label,
        measure,
    );

    let icons = TitleBarIcons::default();
    let buttons = [
        (MINIMIZE_ACTION, WindowGlyph::Minimize),
        (CLOSE_ACTION, WindowGlyph::Minimize),
    ];
    for (index, (name, glyph)) in buttons.into_iter().enumerate() {
        let x = width
            - style.padding.right
            - style.icon.width * (buttons.len() - index) as f32
            - style.gap * (buttons.len() - index - 1) as f32;
        let rect = cloud_ui::Rect {
            x,
            y: (HEIGHT - style.icon.height) / 2.0,
            width: style.icon.width,
            height: style.icon.height,
        };
        let hovered = pointer.is_some_and(|(px, py)| ui::inside(rect, px, py));
        let is_close = name == CLOSE_ACTION;
        if hovered {
            ui::panel(
                elements,
                rect,
                if is_close {
                    style.close_hover_background
                } else {
                    style.icon.hover_background
                },
                style.icon.radius,
            );
        }
        let color = if hovered {
            if is_close {
                style.close_hover_icon
            } else {
                style.icon.hover_icon
            }
        } else {
            style.icon.icon
        };
        let size = style.icon.icon_size;
        if is_close {
            elements.push(Element::Svg {
                x: rect.x + (rect.width - size) / 2.0,
                y: rect.y + (rect.height - size) / 2.0,
                size,
                source: icons.close.to_owned(),
                tint: Some(color),
            });
        } else {
            elements.push(Element::WindowIcon {
                x: rect.x + (rect.width - size) / 2.0,
                y: rect.y + (rect.height - size) / 2.0,
                size,
                glyph,
                color,
            });
        }
        regions.push(Region::new(
            name,
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
}
