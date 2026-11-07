//! 自绘标题栏：标题 + 最小化 / 最大化（还原）/ 关闭按钮。
//!
//! 四个图标直接内嵌 `Images/SVG/` 里的 `减号.svg` / `最大化.svg` / `还原窗口.svg` /
//! `打叉.svg`（编译期读入）；形状用 `fill="currentColor"`，运行时把主题色写进根节点
//! `color`，SVG 自带的其他填色保持原样。
//!
//! 标题栏本身只是一行控件：按钮把名字写进 `Document.regions`，应用收到
//! `AppEvent::RegionClicked` 后调 `Context::minimize` / `toggle_maximize` / `close`；
//! 空白处由 `WindowOptions::caption_height` 交给窗口拖动。

use cloud_ui_core::{Edges, Node, NodeId, NodeKind, Rect, Size, SizeSpec};
use cloudime_render::{Color, Element, FontSlot, HitShape, WindowGlyph};

use super::{LabelStyle, View, push_region};

/// 最小化按钮的 `RegionClicked` 名字。
pub const MINIMIZE_ACTION: &str = "window.minimize";

/// 最大化 / 还原按钮的 `RegionClicked` 名字。
pub const MAXIMIZE_ACTION: &str = "window.maximize";

/// 关闭按钮的 `RegionClicked` 名字。
pub const CLOSE_ACTION: &str = "window.close";

/// 标题栏使用的 SVG 源。
#[derive(Debug, Clone, Copy)]
pub struct TitleBarIcons {
    pub minimize: &'static str,
    pub maximize: &'static str,
    pub restore: &'static str,
    pub close: &'static str,
}

/// 还原图标（`Images/SVG/还原窗口.svg`）。
pub const RESTORE_SVG: &str = include_str!("../../../../Images/SVG/还原窗口.svg");

impl Default for TitleBarIcons {
    fn default() -> Self {
        Self {
            minimize: include_str!("../../../../Images/SVG/减号.svg"),
            maximize: include_str!("../../../../Images/SVG/最大化.svg"),
            restore: RESTORE_SVG,
            close: include_str!("../../../../Images/SVG/打叉.svg"),
        }
    }
}

/// 图标按钮样式（标题栏用，也可以单独用）。
#[derive(Debug, Clone, Copy)]
pub struct IconButtonStyle {
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub icon_size: f32,
    /// 普通状态图标色；SVG 里的 `currentColor` 取它，其他 fill 保持源色。
    pub icon: Color,
    pub hover_background: Color,
    /// 悬停时 `currentColor` 的颜色。
    pub hover_icon: Color,
    /// `false` 时给 SVG 根节点加 `shape-rendering="crispEdges"`，关掉形状抗锯齿。
    pub anti_alias: bool,
}

/// 自绘标题栏样式。
#[derive(Debug, Clone, Copy)]
pub struct TitleBarStyle {
    pub height: f32,

    /// 标题栏底色；`None` = 透出窗口背景。
    pub background: Option<Color>,

    pub title_size: f32,
    pub title_color: Color,

    /// 左右留白（上下由行内居中负责）。
    pub padding: Edges,

    /// 标题与按钮之间的空隙。
    pub gap: f32,

    pub icon: IconButtonStyle,

    /// 关闭按钮悬停时的底色 / 图标色（其余按钮用 `icon` 里的悬停色）。
    pub close_hover_background: Color,
    pub close_hover_icon: Color,

    /// 默认 true：最小化 / 最大化 / 还原用设备像素对齐的内置字形；
    /// false 时回到 `icons` 里的 SVG（用于完全自定义图标）。
    pub pixel_icons: bool,

    pub icons: TitleBarIcons,
}

impl Default for TitleBarStyle {
    fn default() -> Self {
        let icon = IconButtonStyle {
            width: 38.0,
            height: 28.0,
            radius: 6.0,
            icon_size: 12.0,
            icon: Color::rgb(236, 236, 242),
            hover_background: Color::rgb(45, 45, 56),
            hover_icon: Color::rgb(255, 255, 255),
            anti_alias: true,
        };
        Self {
            height: 36.0,
            background: None,
            title_size: 13.0,
            title_color: Color::rgb(236, 236, 242),
            padding: Edges {
                top: 4.0,
                right: 6.0,
                bottom: 4.0,
                left: 16.0,
            },
            gap: 4.0,
            icon,
            close_hover_background: Color::rgb(196, 43, 28),
            close_hover_icon: Color::rgb(255, 255, 255),
            pixel_icons: true,
            icons: TitleBarIcons::default(),
        }
    }
}

impl View {
    /// 加一行标题栏：标题 + 最小化 / 最大化（还原）/ 关闭。
    ///
    /// 三个按钮分别派 `MINIMIZE_ACTION` / `MAXIMIZE_ACTION` / `CLOSE_ACTION`；
    /// 空白处不算区域，窗口层按 `WindowOptions::caption_height` 当标题拖动。
    pub fn title_bar(
        &mut self,
        title: impl Into<String>,
        style: TitleBarStyle,
        maximized: bool,
    ) -> NodeId {
        let icons = style.icons;
        let title_style = LabelStyle {
            size: style.title_size,
            slot: FontSlot::Candidate,
            color: style.title_color,
        };
        let close = IconButtonStyle {
            anti_alias: true,
            hover_background: style.close_hover_background,
            hover_icon: style.close_hover_icon,
            ..style.icon
        };
        let crisp = IconButtonStyle {
            anti_alias: false,
            ..style.icon
        };
        let id = self.container(
            NodeKind::Stem,
            cloud_ui_core::LayoutSpec::Flex {
                direction: cloud_ui_core::Axis::Row,
                wrap: false,
                justify: cloud_ui_core::Justify::Start,
                align: cloud_ui_core::Align::Center,
                gap: style.gap,
            },
            style.padding,
            super::Widget::TitleBar { style },
            |ui| {
                ui.label(title, title_style);
                ui.flex_spacer();
                if style.pixel_icons {
                    ui.window_icon_button(MINIMIZE_ACTION, WindowGlyph::Minimize, crisp);
                    ui.window_icon_button(
                        MAXIMIZE_ACTION,
                        if maximized {
                            WindowGlyph::Restore
                        } else {
                            WindowGlyph::Maximize
                        },
                        crisp,
                    );
                } else {
                    ui.icon_button(MINIMIZE_ACTION, icons.minimize, crisp);
                    ui.icon_button(
                        MAXIMIZE_ACTION,
                        if maximized {
                            icons.restore
                        } else {
                            icons.maximize
                        },
                        crisp,
                    );
                }
                ui.icon_button(CLOSE_ACTION, icons.close, close);
            },
        );
        self.tree.nodes[id].width = SizeSpec::Percent(1.0);
        self.tree.nodes[id].height = SizeSpec::Fixed(style.height);
        id
    }

    /// 一个吃满剩余空间的空位（`grow = 1`）。
    pub fn flex_spacer(&mut self) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Fixed(0.0);
        node.height = SizeSpec::Fixed(0.0);
        node.grow = 1.0;
        self.push(node, super::Widget::Spacer)
    }

    /// 一个 SVG 图标按钮；`name` 会出现在 `AppEvent::RegionClicked` 里。
    ///
    /// SVG 里的 `currentColor` 取 `style.icon`（悬停时取 `style.hover_icon`），
    /// 其他 fill / stroke 保持 SVG 源里的颜色。
    pub fn icon_button(
        &mut self,
        name: impl Into<String>,
        svg: &'static str,
        style: IconButtonStyle,
    ) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Fixed(style.width);
        node.height = SizeSpec::Fixed(style.height);
        self.push(
            node,
            super::Widget::IconButton {
                name: name.into(),
                svg,
                glyph: None,
                style,
            },
        )
    }

    /// 设备像素对齐的内置窗口图标按钮；标题栏内部用。
    pub(crate) fn window_icon_button(
        &mut self,
        name: impl Into<String>,
        glyph: WindowGlyph,
        style: IconButtonStyle,
    ) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Fixed(style.width);
        node.height = SizeSpec::Fixed(style.height);
        self.push(
            node,
            super::Widget::IconButton {
                name: name.into(),
                svg: "",
                glyph: Some(glyph),
                style,
            },
        )
    }
}

pub(super) fn measure_icon(
    style: &IconButtonStyle,
    constraints: cloud_ui_core::Constraints,
) -> Size {
    Size::new(
        style.width.min(constraints.max_width),
        style.height.min(constraints.max_height),
    )
}

pub(super) fn add_background(elements: &mut Vec<Element>, rect: Rect, style: &TitleBarStyle) {
    if let Some(color) = style.background {
        elements.push(Element::Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius: 0.0,
            color,
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add_icon_button(
    elements: &mut Vec<Element>,
    regions: &mut Vec<cloudime_render::Region>,
    rect: Rect,
    drawn: (f32, f32),
    clip: Option<Rect>,
    name: &str,
    svg: &'static str,
    glyph: Option<WindowGlyph>,
    style: &IconButtonStyle,
    pointer: Option<(f32, f32)>,
) {
    let hovered = pointer.is_some_and(|(x, y)| {
        let left = drawn.0 + rect.x;
        let top = drawn.1 + rect.y;
        x >= left && x <= left + rect.width && y >= top && y <= top + rect.height
    });
    if hovered {
        elements.push(Element::Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius: style.radius,
            color: style.hover_background,
        });
    }
    let size = style.icon_size.min(rect.width).min(rect.height).max(1.0);
    let icon_color = if hovered {
        style.hover_icon
    } else {
        style.icon
    };
    let icon_x = rect.x + (rect.width - size) * 0.5;
    let icon_y = rect.y + (rect.height - size) * 0.5;
    match glyph {
        Some(glyph) => elements.push(Element::WindowIcon {
            x: icon_x,
            y: icon_y,
            size,
            glyph,
            color: icon_color,
        }),
        None => elements.push(Element::Svg {
            x: icon_x,
            y: icon_y,
            size,
            source: icon_source(svg, style.anti_alias, icon_color),
            tint: None,
        }),
    }
    push_region(
        regions,
        name,
        HitShape::RoundRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            radius: style.radius,
        },
        drawn,
        clip,
    );
}

/// `anti_alias = false` 时靠 SVG 自己的 `shape-rendering` 关掉形状 AA；
/// 同时把当前图标色写进根节点 `color`，让 SVG 里的 `fill="currentColor"` 参与主题配色。
/// resvg 会把这个属性翻译成 `anti_alias = false`，不用事后二值化像素。
fn icon_source(svg: &str, anti_alias: bool, color: Color) -> String {
    let mut attributes = String::new();
    if !anti_alias && !svg.contains("shape-rendering") {
        attributes.push_str(r#" shape-rendering="crispEdges""#);
    }
    if !svg.contains(" color=") {
        attributes.push_str(&format!(
            " color=\"#{:02X}{:02X}{:02X}\"",
            color.r, color.g, color.b
        ));
    }
    if attributes.is_empty() {
        svg.to_owned()
    } else {
        svg.replacen("<svg", &format!("<svg{attributes}"), 1)
    }
}
