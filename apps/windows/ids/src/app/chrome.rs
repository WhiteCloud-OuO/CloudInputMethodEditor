//! 自绘标题栏：标题、最小化 / 最大化 / 关闭按钮。
//!
//! 窗口用 `Decorations::None` + `caption_height`，这一条 36 点高的区域由窗口壳负责拖动与双击最大化；
//! 按钮自己的命中区域是不可拖动的，所以点按钮不会变成拖窗口。

use cloud_ui::{CLOSE_ACTION, MAXIMIZE_ACTION, MINIMIZE_ACTION, Theme, TitleBarIcons};
use cloudime_render::{Element, FontSlot, HitShape, Region};

use crate::ui;

/// 标题栏高度（与 `WindowOptions.caption_height` 一致）。
pub const HEIGHT: f32 = 36.0;

const BUTTON_WIDTH: f32 = 36.0;
const BUTTON_HEIGHT: f32 = 26.0;
const ICON_SIZE: f32 = 12.0;

/// 画标题栏并登记三个按钮区域。
#[allow(clippy::too_many_arguments)]
pub fn paint(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    width: f32,
    title: &str,
    theme: &Theme,
    pointer: Option<(f32, f32)>,
    maximized: bool,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let icons = TitleBarIcons::default();
    let mut label = theme.label();
    label.size = 13.0;
    ui::label(
        elements,
        cloud_ui::Rect {
            x: 16.0,
            y: 0.0,
            width: (width - BUTTON_WIDTH * 3.0 - 24.0).max(1.0),
            height: HEIGHT,
        },
        title,
        &label,
        measure,
    );

    let buttons = [
        (MINIMIZE_ACTION, icons.minimize),
        (
            MAXIMIZE_ACTION,
            if maximized {
                icons.restore
            } else {
                icons.maximize
            },
        ),
        (CLOSE_ACTION, icons.close),
    ];
    for (index, (name, svg)) in buttons.into_iter().enumerate() {
        let x = width - BUTTON_WIDTH * (3 - index) as f32;
        let rect = cloud_ui::Rect {
            x,
            y: (HEIGHT - BUTTON_HEIGHT) / 2.0,
            width: BUTTON_WIDTH,
            height: BUTTON_HEIGHT,
        };
        let hovered = pointer.is_some_and(|(px, py)| ui::inside(rect, px, py));
        if hovered {
            ui::panel(elements, rect, theme.surface_alt, 6.0);
        }
        elements.push(Element::Svg {
            x: rect.x + (BUTTON_WIDTH - ICON_SIZE) / 2.0,
            y: rect.y + (BUTTON_HEIGHT - ICON_SIZE) / 2.0,
            size: ICON_SIZE,
            source: svg.to_owned(),
            tint: Some(if name == CLOSE_ACTION && hovered {
                cloudime_render::Color::rgb(255, 120, 120)
            } else {
                theme.text
            }),
        });
        regions.push(Region::new(
            name,
            HitShape::RoundRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                radius: 6.0,
            },
            false,
        ));
    }
}
