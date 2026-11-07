//! 无 Lua 的 view / widget / 布局构建层。
//!
//! 这是独立 exe 的“应用 UI”入口：用 `View` 搭一棵简单的控件树，`build` 时先走
//! [`cloud_ui_core`] 的 measure-arrange，再转成 [`cloudime_render::Document`]。
//! 现有 IME 路径继续用 `cloudime-script` 的页面树 JSON，两者共用同一个布局核心与渲染器。
//!
//! ```ignore
//! let mut view = View::new(width, height);
//! view.column(StackStyle::default(), |ui| {
//!     ui.label("标题", LabelStyle::title());
//!     ui.button("ok", "确定", ButtonStyle::default());
//! });
//! let document = view.build(|text, slot, size| painter.measure_text(text, slot, size));
//! ```

use std::collections::HashMap;

use cloud_ui_core::{
    Align, Axis, Constraints, Edges, Justify, LayoutResult, LayoutSpec, Measure, Node, NodeId,
    NodeKind, PageTree, Rect, Size, SizeSpec, layout,
};
use cloudime_render::{Color, Document, Element, FontSlot, HitShape, Region, WindowGlyph};

mod list_box;
mod scrollable_page;
pub(crate) mod text_edit;
mod theme;
mod title_bar;

pub use list_box::{ListBoxState, ListBoxStyle, ListItem};
pub use scrollable_page::{ScrollablePageMetrics, ScrollablePageState, ScrollablePageStyle};
pub use text_edit::TextEditStyle;
pub use theme::Theme;
pub use title_bar::{
    CLOSE_ACTION, IconButtonStyle, MAXIMIZE_ACTION, MINIMIZE_ACTION, TitleBarIcons, TitleBarStyle,
};

/// 标签样式。
#[derive(Debug, Clone)]
pub struct LabelStyle {
    pub size: f32,
    pub slot: FontSlot,
    pub color: Color,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self {
            size: 16.0,
            slot: FontSlot::Candidate,
            color: Color::rgb(236, 236, 242),
        }
    }
}

impl LabelStyle {
    /// 标题用：大一号、亮一点。
    pub fn title() -> Self {
        Self {
            size: 26.0,
            ..Self::default()
        }
    }

    /// 次要说明用。
    pub fn muted() -> Self {
        Self {
            size: 13.0,
            color: Color::rgb(150, 150, 162),
            ..Self::default()
        }
    }
}

/// 按钮样式。
#[derive(Debug, Clone)]
pub struct ButtonStyle {
    pub size: f32,
    pub slot: FontSlot,
    pub color: Color,
    pub background: Color,
    pub radius: f32,
    /// 文字左右各留多少点。
    pub padding_x: f32,
    /// 按钮高度（点）。
    pub height: f32,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self {
            size: 14.0,
            slot: FontSlot::Candidate,
            color: Color::rgb(255, 255, 255),
            background: Color::rgb(48, 108, 220),
            radius: 8.0,
            padding_x: 16.0,
            height: 34.0,
        }
    }
}

/// 列 / 行容器的间距与内边距。
#[derive(Debug, Clone, Copy, Default)]
pub struct StackStyle {
    pub gap: f32,
    pub padding: Edges,
}

impl StackStyle {
    pub fn gap(gap: f32) -> Self {
        Self {
            gap,
            padding: Edges::default(),
        }
    }

    pub fn padded(gap: f32, padding: f32) -> Self {
        Self {
            gap,
            padding: Edges::all(padding),
        }
    }
}

/// 面板（有背景的容器）样式。
#[derive(Debug, Clone, Copy)]
pub struct PanelStyle {
    pub background: Color,
    pub radius: f32,
    pub padding: Edges,
}

impl Default for PanelStyle {
    fn default() -> Self {
        Self {
            background: Color::rgb(32, 32, 40),
            radius: 12.0,
            padding: Edges::all(16.0),
        }
    }
}

#[derive(Debug, Clone)]
enum Widget {
    Label {
        text: String,
        style: LabelStyle,
    },
    Button {
        name: String,
        text: String,
        style: ButtonStyle,
    },
    Panel {
        style: PanelStyle,
    },
    ScrollablePage {
        name: String,
        style: ScrollablePageStyle,
        state: ScrollablePageState,
    },
    ListBox {
        name: String,
        items: Vec<ListItem>,
        style: ListBoxStyle,
        state: ListBoxState,
    },
    TextEdit {
        name: String,
        text: String,
        style: TextEditStyle,
    },
    TextBox {
        name: String,
        text: String,
        style: TextEditStyle,
    },
    TitleBar {
        style: TitleBarStyle,
    },
    IconButton {
        name: String,
        svg: &'static str,
        glyph: Option<WindowGlyph>,
        style: IconButtonStyle,
    },
    Container,
    Spacer,
}

/// 一棵可布局的控件树。
pub struct View {
    tree: PageTree,
    widgets: HashMap<NodeId, Widget>,
    current: NodeId,
    width: f32,
    height: f32,
    background: Option<(Color, f32)>,
    pointer: Option<(f32, f32)>,
    pages: HashMap<String, ScrollablePageMetrics>,
}

/// 一次 emit 的目标容器。
struct Target<'a> {
    elements: &'a mut Vec<Element>,

    /// 元素坐标要减掉的累计原点（把绝对布局坐标变成当前面板的局部坐标）。
    origin: (f32, f32),

    /// 当前坐标系的 (0, 0) 在文档里的实际位置；区域命中框用它换算成绝对坐标。
    drawn: (f32, f32),

    /// 当前可见区域（绝对坐标）；滚动容器里生成的区域按它裁剪。
    clip: Option<Rect>,
}

impl View {
    /// 建一棵铺满 `width × height`（点）的根页面；布局方向缺省为纵向。
    pub fn new(width: f32, height: f32) -> Self {
        let mut root = Node::new(NodeKind::Page);
        root.layout = LayoutSpec::Flex {
            direction: Axis::Column,
            wrap: false,
            justify: Justify::Start,
            align: Align::Start,
            gap: 0.0,
        };
        root.width = SizeSpec::Fixed(width.max(1.0));
        root.height = SizeSpec::Fixed(height.max(1.0));
        let tree = PageTree::new(root);
        Self {
            tree,
            widgets: HashMap::new(),
            current: 0,
            width,
            height,
            background: None,
            pointer: None,
            pages: HashMap::new(),
        }
    }

    /// 当前指针位置（点）；列表盒用它算行悬停。
    pub fn pointer(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
    }

    /// 整窗背景色（画在所有内容之前）。
    pub fn background(&mut self, color: Color) {
        self.background = Some((color, 0.0));
    }

    /// 整窗圆角背景；无边框窗口用它做可见的圆角面板。
    pub fn background_rounded(&mut self, color: Color, radius: f32) {
        self.background = Some((color, radius.max(0.0)));
    }

    /// 纵向排列。
    pub fn column(&mut self, style: StackStyle, add: impl FnOnce(&mut Self)) -> NodeId {
        self.container(
            NodeKind::Stem,
            LayoutSpec::Flex {
                direction: Axis::Column,
                wrap: false,
                justify: Justify::Start,
                align: Align::Start,
                gap: style.gap,
            },
            style.padding,
            Widget::Container,
            add,
        )
    }

    /// 横向排列。
    ///
    /// 单行 flex：**不换行、不压缩子项、也不裁剪**。子项按固有宽度排，总宽超过父容器时会画到容器外；
    /// 内容可能超宽时请拆成多行，或把长文本放进 `TextBlock` / 可滚动页面。
    pub fn row(&mut self, style: StackStyle, add: impl FnOnce(&mut Self)) -> NodeId {
        self.container(
            NodeKind::Stem,
            LayoutSpec::Flex {
                direction: Axis::Row,
                wrap: false,
                justify: Justify::Start,
                align: Align::Center,
                gap: style.gap,
            },
            style.padding,
            Widget::Container,
            add,
        )
    }

    /// 有背景的容器。
    pub fn panel(&mut self, style: PanelStyle, add: impl FnOnce(&mut Self)) -> NodeId {
        self.container(
            NodeKind::Container,
            LayoutSpec::Flex {
                direction: Axis::Column,
                wrap: false,
                justify: Justify::Start,
                align: Align::Start,
                gap: 8.0,
            },
            style.padding,
            Widget::Panel { style },
            add,
        )
    }

    /// 固定高度、可垂直滚动的控件容器（可滚动页面）；内容超出视口时裁剪并画滚动条。
    ///
    /// 控件容器与子页面容器并列；这里只放普通控件。滚动偏移由应用保存；
    /// 布局后可以读 [`View::scrollable_page_metrics`] 拿到可滚范围。
    pub fn scrollable_page(
        &mut self,
        name: impl Into<String>,
        style: ScrollablePageStyle,
        state: ScrollablePageState,
        add: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut node = Node::new(NodeKind::Container);
        node.layout = LayoutSpec::Flex {
            direction: Axis::Column,
            wrap: false,
            justify: Justify::Start,
            align: Align::Start,
            gap: style.gap,
        };
        node.padding = style.padding;
        node.width = SizeSpec::Percent(1.0);
        node.height = SizeSpec::Fixed(style.height);
        let id = self.push(
            node,
            Widget::ScrollablePage {
                name: name.into(),
                style,
                state,
            },
        );
        let previous = self.current;
        self.current = id;
        add(self);
        self.current = previous;
        id
    }

    /// 一行文字。
    pub fn label(&mut self, text: impl Into<String>, style: LabelStyle) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Auto;
        node.height = SizeSpec::Auto;
        self.push(
            node,
            Widget::Label {
                text: text.into(),
                style,
            },
        )
    }

    /// 一个可点击按钮；`name` 会出现在 `AppEvent::RegionClicked` 里。
    pub fn button(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
        style: ButtonStyle,
    ) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Auto;
        node.height = SizeSpec::Fixed(style.height);
        self.push(
            node,
            Widget::Button {
                name: name.into(),
                text: text.into(),
                style,
            },
        )
    }

    /// 固定尺寸的空位。
    pub fn spacer(&mut self, width: f32, height: f32) -> NodeId {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Fixed(width);
        node.height = SizeSpec::Fixed(height);
        self.push(node, Widget::Spacer)
    }

    /// 上一次 `build` 时某个可滚动页面容器的可滚范围。
    pub fn scrollable_page_metrics(&self, name: &str) -> Option<ScrollablePageMetrics> {
        self.pages.get(name).copied()
    }

    /// 量文字并生成 Document。`measure` 一般传 `Painter::measure_text` 的闭包。
    pub fn build<F>(&mut self, mut measure: F) -> Document
    where
        F: FnMut(&str, FontSlot, f32) -> (f32, f32),
    {
        self.pages.clear();
        let constraints = Constraints {
            min_width: 0.0,
            max_width: self.width,
            min_height: 0.0,
            max_height: self.height,
        };
        let result = {
            let mut adapter = MeasureAdapter {
                widgets: &self.widgets,
                measure: &mut measure,
            };
            layout(&self.tree, constraints, &mut adapter)
        };

        let mut document = Document::empty(self.width, self.height);
        if let Some((color, radius)) = self.background {
            document.elements.push(Element::Rect {
                x: 0.0,
                y: 0.0,
                width: self.width,
                height: self.height,
                radius,
                color,
            });
        }
        let mut pages = HashMap::new();
        {
            let mut target = Target {
                elements: &mut document.elements,
                origin: (0.0, 0.0),
                drawn: (0.0, 0.0),
                clip: None,
            };
            self.emit(
                0,
                &result,
                &mut target,
                &mut document.regions,
                &mut measure,
                &mut pages,
            );
        }
        self.pages = pages;
        document
    }

    fn container(
        &mut self,
        kind: NodeKind,
        layout_spec: LayoutSpec,
        padding: Edges,
        widget: Widget,
        add: impl FnOnce(&mut Self),
    ) -> NodeId {
        let mut node = Node::new(kind);
        node.layout = layout_spec;
        node.padding = padding;
        let id = self.push(node, widget);
        let previous = self.current;
        self.current = id;
        add(self);
        self.current = previous;
        id
    }

    fn push(&mut self, node: Node, widget: Widget) -> NodeId {
        let parent = self.current;
        let id = self.tree.push(node);
        self.tree.nodes[parent].children.push(id);
        self.widgets.insert(id, widget);
        id
    }

    /// 递归生成元素；可滚动页面会另起一层坐标并把子元素塞进裁剪面板。
    fn emit<F>(
        &self,
        id: NodeId,
        result: &LayoutResult,
        target: &mut Target<'_>,
        regions: &mut Vec<Region>,
        measure: &mut F,
        pages: &mut HashMap<String, ScrollablePageMetrics>,
    ) where
        F: FnMut(&str, FontSlot, f32) -> (f32, f32),
    {
        let Some(rect) = result.rect(id) else {
            return;
        };
        let local = Rect::new(
            rect.x - target.origin.0,
            rect.y - target.origin.1,
            rect.width,
            rect.height,
        );
        let drawn = target.drawn;
        let clip = target.clip;

        match self.widgets.get(&id) {
            Some(Widget::ScrollablePage { name, style, state }) => {
                let content_top = rect.y + style.padding.top;
                let mut content_bottom = content_top;
                for child in &self.tree.nodes[id].children {
                    content_bottom = content_bottom.max(self.subtree_bottom(*child, result));
                }
                let viewport = (rect.height - style.padding.vertical()).max(0.0);
                let content_height =
                    (content_bottom - content_top + style.padding.bottom).max(viewport);
                let metrics = ScrollablePageMetrics {
                    rect: Rect::new(
                        drawn.0 + local.x,
                        drawn.1 + local.y,
                        rect.width,
                        rect.height,
                    ),
                    content_height,
                    viewport_height: viewport,
                };
                let offset = metrics.clamp(state.offset);
                let mut panel = scrollable_page::panel(style, local);
                let panel_clip = intersect_rect(clip, rect);
                {
                    let mut child_target = Target {
                        elements: &mut panel.children,
                        origin: (rect.x, rect.y + offset),
                        drawn: (drawn.0 + local.x, drawn.1 + local.y),
                        clip: panel_clip,
                    };
                    for child in &self.tree.nodes[id].children {
                        self.emit(*child, result, &mut child_target, regions, measure, pages);
                    }
                }
                scrollable_page::push_scrollbar(&mut panel, style, metrics, offset);
                target.elements.push(Element::SelfDrawingPanel(panel));
                pages.insert(name.clone(), metrics);
                return;
            }
            Some(Widget::Panel { style }) => add_panel(target.elements, local, style),
            Some(Widget::Label { text, style }) => add_label(target.elements, local, text, style),
            Some(Widget::Button { name, text, style }) => add_button(
                target.elements,
                regions,
                local,
                drawn,
                clip,
                name,
                text,
                style,
                measure,
            ),
            Some(Widget::ListBox {
                name,
                items,
                style,
                state,
            }) => {
                let box_drawn = (drawn.0 + local.x, drawn.1 + local.y);
                let pointer = self
                    .pointer
                    .map(|(x, y)| (x - box_drawn.0, y - box_drawn.1));
                list_box::build(
                    target.elements,
                    regions,
                    local,
                    drawn,
                    clip,
                    list_box::ListBoxInput {
                        name,
                        items,
                        style: *style,
                    },
                    pointer,
                    *state,
                    measure,
                );
            }
            Some(Widget::TextEdit { name, text, style }) => {
                text_edit::build(target.elements, local, name, text, style)
            }
            Some(Widget::TextBox { name, text, style }) => {
                text_edit::build_box(target.elements, local, name, text, style)
            }
            Some(Widget::TitleBar { style }) => {
                title_bar::add_background(target.elements, local, style)
            }
            Some(Widget::IconButton {
                name,
                svg,
                glyph,
                style,
            }) => title_bar::add_icon_button(
                target.elements,
                regions,
                local,
                drawn,
                clip,
                name,
                svg,
                *glyph,
                style,
                self.pointer,
            ),
            Some(Widget::Container | Widget::Spacer) | None => {}
        }

        for child in &self.tree.nodes[id].children {
            self.emit(*child, result, target, regions, measure, pages);
        }
    }

    /// 子树里最靠下的矩形底边（绝对坐标）。
    fn subtree_bottom(&self, id: NodeId, result: &LayoutResult) -> f32 {
        let mut bottom = result.rect(id).map_or(0.0, Rect::bottom);
        for child in &self.tree.nodes[id].children {
            bottom = bottom.max(self.subtree_bottom(*child, result));
        }
        bottom
    }
}

struct MeasureAdapter<'a, F> {
    widgets: &'a HashMap<NodeId, Widget>,
    measure: &'a mut F,
}

impl<F> Measure for MeasureAdapter<'_, F>
where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    fn measure(&mut self, node: NodeId, constraints: Constraints) -> Size {
        let Some(widget) = self.widgets.get(&node) else {
            return Size::ZERO;
        };
        match widget {
            Widget::Label { text, style } => {
                let (width, height) = (self.measure)(text, style.slot, style.size);
                Size::new(
                    width.min(constraints.max_width),
                    height.min(constraints.max_height),
                )
            }
            Widget::Button { text, style, .. } => {
                let (width, height) = (self.measure)(text, style.slot, style.size);
                Size::new(
                    (width + style.padding_x * 2.0).min(constraints.max_width),
                    style.height.min(constraints.max_height).max(height),
                )
            }
            Widget::ListBox { style, .. } => list_box::measure(style, constraints),
            Widget::TextEdit { style, .. } | Widget::TextBox { style, .. } => {
                text_edit::measure(style, constraints)
            }
            Widget::IconButton { style, .. } => title_bar::measure_icon(style, constraints),
            Widget::ScrollablePage { style, .. } => Size::new(
                constraints.max_width,
                style.height.min(constraints.max_height),
            ),
            Widget::TitleBar { .. } | Widget::Panel { .. } | Widget::Container | Widget::Spacer => {
                Size::ZERO
            }
        }
    }
}

fn add_panel(elements: &mut Vec<Element>, rect: Rect, style: &PanelStyle) {
    elements.push(Element::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: style.radius,
        color: style.background,
    });
}

fn add_label(elements: &mut Vec<Element>, rect: Rect, text: &str, style: &LabelStyle) {
    elements.push(Element::Text {
        x: rect.x,
        y: rect.y,
        text: text.to_owned(),
        size: style.size,
        slot: style.slot,
        color: style.color,
        italic: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn add_button<F>(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    rect: Rect,
    drawn: (f32, f32),
    clip: Option<Rect>,
    name: &str,
    text: &str,
    style: &ButtonStyle,
    measure: &mut F,
) where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    elements.push(Element::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: style.radius,
        color: style.background,
    });
    let (text_width, text_height) = measure(text, style.slot, style.size);
    elements.push(Element::Text {
        x: rect.x + (rect.width - text_width).max(0.0) / 2.0,
        y: rect.y + (rect.height - text_height).max(0.0) / 2.0,
        text: text.to_owned(),
        size: style.size,
        slot: style.slot,
        color: style.color,
        italic: false,
    });
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

/// 把局部命中形状平移到文档坐标、按可见区域裁剪后写进 regions。
fn push_region(
    regions: &mut Vec<Region>,
    name: &str,
    shape: HitShape,
    drawn: (f32, f32),
    clip: Option<Rect>,
) {
    let shape = shift_shape(shape, drawn);
    if let Some(shape) = clip_shape(shape, clip) {
        regions.push(Region::new(name, shape, false));
    }
}

fn shift_shape(shape: HitShape, (dx, dy): (f32, f32)) -> HitShape {
    match shape {
        HitShape::Rect(rect) => HitShape::Rect(cloudime_render::HitRect::new(
            rect.x + dx,
            rect.y + dy,
            rect.width,
            rect.height,
        )),
        HitShape::RoundRect {
            x,
            y,
            width,
            height,
            radius,
        } => HitShape::RoundRect {
            x: x + dx,
            y: y + dy,
            width,
            height,
            radius,
        },
        HitShape::Circle { cx, cy, radius } => HitShape::Circle {
            cx: cx + dx,
            cy: cy + dy,
            radius,
        },
        HitShape::Segment {
            from,
            to,
            thickness,
        } => HitShape::Segment {
            from: (from.0 + dx, from.1 + dy),
            to: (to.0 + dx, to.1 + dy),
            thickness,
        },
        HitShape::Path { contours } => HitShape::Path {
            contours: contours
                .into_iter()
                .map(|contour| contour.into_iter().map(|(x, y)| (x + dx, y + dy)).collect())
                .collect(),
        },
    }
}

fn clip_shape(shape: HitShape, clip: Option<Rect>) -> Option<HitShape> {
    let Some(clip) = clip else {
        return Some(shape);
    };
    match shape {
        HitShape::Rect(rect) => {
            let x = rect.x.max(clip.x);
            let y = rect.y.max(clip.y);
            let right = (rect.x + rect.width).min(clip.right());
            let bottom = (rect.y + rect.height).min(clip.bottom());
            (right > x && bottom > y)
                .then(|| HitShape::Rect(cloudime_render::HitRect::new(x, y, right - x, bottom - y)))
        }
        HitShape::RoundRect {
            x,
            y,
            width,
            height,
            radius,
        } => {
            let left = x.max(clip.x);
            let top = y.max(clip.y);
            let right = (x + width).min(clip.right());
            let bottom = (y + height).min(clip.bottom());
            let width = right - left;
            let height = bottom - top;
            (width > 0.0 && height > 0.0).then(|| HitShape::RoundRect {
                x: left,
                y: top,
                width,
                height,
                radius: radius.min(width * 0.5).min(height * 0.5),
            })
        }
        other => Some(other),
    }
}

fn intersect_rect(outer: Option<Rect>, inner: Rect) -> Option<Rect> {
    let Some(outer) = outer else {
        return Some(inner);
    };
    let x = outer.x.max(inner.x);
    let y = outer.y.max(inner.y);
    let right = outer.right().min(inner.right());
    let bottom = outer.bottom().min(inner.bottom());
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(text: &str, _slot: FontSlot, size: f32) -> (f32, f32) {
        (text.chars().count() as f32 * size * 0.6, size * 1.25)
    }

    #[test]
    fn stacks_labels_and_buttons_in_a_column() {
        let mut view = View::new(400.0, 300.0);
        view.column(StackStyle::padded(8.0, 16.0), |ui| {
            ui.label("标题", LabelStyle::title());
            ui.button("ok", "确定", ButtonStyle::default());
        });
        let document = view.build(measure);
        assert_eq!(
            document.elements.len(),
            3,
            "label + button rect + button text"
        );
        assert_eq!(document.regions.len(), 1);
        assert_eq!(document.regions[0].name, "ok");
        let label = &document.elements[0];
        let Element::Text { x, y, text, .. } = label else {
            panic!("第一项应是标题文字");
        };
        assert_eq!(text, "标题");
        assert!((*x - 16.0).abs() < 0.01);
        assert!((*y - 16.0).abs() < 0.01);
    }

    #[test]
    fn panels_draw_before_their_children() {
        let mut view = View::new(200.0, 120.0);
        view.panel(PanelStyle::default(), |ui| {
            ui.label("里面", LabelStyle::default());
        });
        let document = view.build(measure);
        assert!(matches!(document.elements[0], Element::Rect { .. }));
        assert!(matches!(document.elements[1], Element::Text { .. }));
    }

    #[test]
    fn list_box_emits_rows_and_row_regions() {
        let mut view = View::new(300.0, 200.0);
        let items = [
            ListItem::new("a").action("row-a"),
            ListItem::new("b").detail("x").action("row-b"),
        ];
        view.list_box(
            "list",
            &items,
            ListBoxStyle::with_height(120.0),
            ListBoxState {
                selected: Some(1),
                scroll_offset: 0.0,
            },
        );
        let document = view.build(measure);
        let Some(Element::ListBox(list)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::ListBox(_)))
        else {
            panic!("应生成列表盒元素");
        };
        assert_eq!(list.region_name, "list");
        assert_eq!(list.row_count, 2);
        assert_eq!(list.selected_index, 1);
        let names: Vec<&str> = document
            .regions
            .iter()
            .map(|region| region.name.as_str())
            .collect();
        assert_eq!(names, ["row-a", "row-b"]);
    }

    #[test]
    fn list_box_hover_uses_local_pointer() {
        let items = [ListItem::new("a"), ListItem::new("b"), ListItem::new("c")];
        let mut view = View::new(300.0, 240.0);
        view.spacer(0.0, 50.0);
        view.list_box(
            "list",
            &items,
            ListBoxStyle::with_height(120.0),
            ListBoxState {
                selected: None,
                scroll_offset: 0.0,
            },
        );
        view.pointer(10.0, 60.0);
        let document = view.build(measure);
        let Some(Element::ListBox(list)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::ListBox(_)))
        else {
            panic!("应生成列表盒元素");
        };
        assert_eq!(list.hovered_index, Some(0));
    }

    #[test]
    fn list_box_hover_outside_is_none() {
        let items = [ListItem::new("a"), ListItem::new("b")];
        let mut view = View::new(300.0, 240.0);
        view.spacer(0.0, 50.0);
        view.list_box(
            "list",
            &items,
            ListBoxStyle::with_height(120.0),
            ListBoxState {
                selected: None,
                scroll_offset: 0.0,
            },
        );
        view.pointer(10.0, 200.0);
        let document = view.build(measure);
        let Some(Element::ListBox(list)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::ListBox(_)))
        else {
            panic!("应生成列表盒元素");
        };
        assert_eq!(list.hovered_index, None);
    }

    #[test]
    fn text_edit_carries_name_and_text() {
        let mut view = View::new(300.0, 100.0);
        view.text_edit("input", "hi", TextEditStyle::default());
        let document = view.build(measure);
        let Some(Element::TextEdit(edit)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::TextEdit(_)))
        else {
            panic!("应生成文本编辑元素");
        };
        assert_eq!(edit.region_name, "input");
        assert_eq!(edit.text, "hi");
    }

    #[test]
    fn theme_styles_share_colors() {
        let theme = Theme::light();
        assert_eq!(theme.text_edit().fill, Some(theme.surface));
        assert_eq!(theme.button().background, theme.accent);
        assert_eq!(theme.list_box(100.0).selection, theme.accent);
        assert_eq!(theme.label().color, theme.text);
    }

    #[test]
    fn scrollable_page_clips_children_and_reports_metrics() {
        let mut view = View::new(200.0, 100.0);
        view.scrollable_page(
            "log",
            ScrollablePageStyle::with_height(50.0),
            ScrollablePageState { offset: 20.0 },
            |ui| {
                for index in 0..4 {
                    ui.label(format!("line {index}"), LabelStyle::default());
                }
            },
        );
        let document = view.build(measure);
        let Some(Element::SelfDrawingPanel(panel)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::SelfDrawingPanel(panel) if panel.clip))
        else {
            panic!("应生成裁剪面板");
        };
        let Element::Text { y, .. } = &panel.children[0] else {
            panic!("第一个子元素应是文字");
        };
        assert!((*y + 20.0).abs() < 0.01, "滚动偏移应作用到子元素");
        let metrics = view.scrollable_page_metrics("log").expect("应记录滚动范围");
        assert_eq!(metrics.viewport_height, 50.0);
        assert!(metrics.content_height > metrics.viewport_height);
        assert!(metrics.max_scroll() > 0.0);
    }

    #[test]
    fn scrollable_page_moves_and_clips_regions() {
        let mut view = View::new(200.0, 100.0);
        view.scrollable_page(
            "page",
            ScrollablePageStyle::with_height(80.0),
            ScrollablePageState { offset: 10.0 },
            |ui| {
                ui.spacer(0.0, 50.0);
                ui.button("ok", "确定", ButtonStyle::default());
                ui.spacer(0.0, 100.0);
            },
        );
        let document = view.build(measure);
        let region = document
            .regions
            .iter()
            .find(|region| region.name == "ok")
            .expect("按钮区域应保留");
        let HitShape::RoundRect { y, .. } = region.shape else {
            panic!("按钮区域应是圆角矩形");
        };
        assert!((y - 48.0).abs() < 0.01, "区域应按滚动偏移上移：y={y}");
    }

    #[test]
    fn scrollable_page_drops_regions_outside_the_viewport() {
        let mut view = View::new(200.0, 100.0);
        view.scrollable_page(
            "page",
            ScrollablePageStyle::with_height(40.0),
            ScrollablePageState { offset: 60.0 },
            |ui| {
                ui.button("hidden", "上面", ButtonStyle::default());
                ui.spacer(0.0, 40.0);
                ui.button("visible", "下面", ButtonStyle::default());
            },
        );
        let document = view.build(measure);
        assert!(
            !document
                .regions
                .iter()
                .any(|region| region.name == "hidden")
        );
        assert!(
            document
                .regions
                .iter()
                .any(|region| region.name == "visible")
        );
    }

    #[test]
    fn list_box_scroll_keeps_rows_complete_and_gaps_even() {
        let mut view = View::new(300.0, 160.0);
        let items: Vec<ListItem> = (0..12)
            .map(|index| ListItem::new(format!("行 {index}")))
            .collect();
        view.list_box(
            "log",
            &items,
            ListBoxStyle::with_height(110.0),
            ListBoxState {
                selected: None,
                scroll_offset: 40.0,
            },
        );
        let document = view.build(measure);
        let Some(Element::ListBox(list)) = document
            .elements
            .iter()
            .find(|element| matches!(element, Element::ListBox(_)))
        else {
            panic!("应生成列表盒元素");
        };
        assert!(list.auto_item_gap_averaging, "可见行应平均分布剩余高度");
        let viewport = list.viewport_height();
        let visible: Vec<(f32, f32)> = list
            .rows
            .iter()
            .filter_map(|row| list.row_view_top(row).map(|top| (top, row.height)))
            .collect();
        assert!(visible.len() >= 2, "至少应显示两整行");
        for (top, height) in &visible {
            assert!(*top >= -0.01, "行顶被裁：{top}");
            assert!(top + height <= viewport + 0.01, "行底被裁：{top}+{height}");
        }
        let gap = visible[1].0 - (visible[0].0 + visible[0].1);
        assert!(gap >= list.gap, "间隙不应小于样式 gap：{gap}");
    }

    #[test]
    fn title_bar_buttons_are_regions() {
        let mut view = View::new(410.0, 100.0);
        view.title_bar("t", TitleBarStyle::default(), false);
        let document = view.build(measure);
        let names: Vec<&str> = document
            .regions
            .iter()
            .map(|region| region.name.as_str())
            .collect();
        assert_eq!(names, [MINIMIZE_ACTION, MAXIMIZE_ACTION, CLOSE_ACTION]);
        let close = &document.regions[2];
        let HitShape::RoundRect { x, y, .. } = close.shape else {
            panic!("图标按钮区域应是圆角矩形");
        };
        assert!((x - 410.0 + 6.0 + 38.0).abs() < 0.01, "close x={x}");
        assert!((y - 4.0).abs() < 0.01, "close y={y}");
    }

    #[test]
    fn title_bar_uses_pixel_glyphs_and_svg_close() {
        let mut view = View::new(410.0, 100.0);
        view.title_bar("t", TitleBarStyle::default(), false);
        let document = view.build(measure);
        let mut glyphs = Vec::new();
        let mut svgs = Vec::new();
        for element in &document.elements {
            match element {
                Element::WindowIcon { glyph, color, .. } => glyphs.push((*glyph, *color)),
                Element::Svg { source, tint, .. } => svgs.push((source.as_str(), *tint)),
                _ => {}
            }
        }
        assert_eq!(
            glyphs.iter().map(|(glyph, _)| *glyph).collect::<Vec<_>>(),
            [WindowGlyph::Minimize, WindowGlyph::Maximize]
        );
        assert_eq!(svgs.len(), 1);
        let (close, tint) = svgs[0];
        assert!(close.contains("fill=\"currentColor\""));
        assert!(close.contains("color=\"#"));
        assert!(!close.contains("shape-rendering=\"crispEdges\""));
        assert!(tint.is_none());

        let mut view = View::new(410.0, 100.0);
        view.title_bar("t", TitleBarStyle::default(), true);
        let document = view.build(measure);
        let glyphs: Vec<WindowGlyph> = document
            .elements
            .iter()
            .filter_map(|element| match element {
                Element::WindowIcon { glyph, .. } => Some(*glyph),
                _ => None,
            })
            .collect();
        assert_eq!(glyphs, [WindowGlyph::Minimize, WindowGlyph::Restore]);
    }

    #[test]
    fn title_bar_svg_fallback_keeps_crisp_and_current_color() {
        let style = TitleBarStyle {
            pixel_icons: false,
            ..TitleBarStyle::default()
        };
        let mut view = View::new(410.0, 100.0);
        view.title_bar("t", style, false);
        let document = view.build(measure);
        let icons: Vec<(&str, Option<Color>)> = document
            .elements
            .iter()
            .filter_map(|element| match element {
                Element::Svg { source, tint, .. } => Some((source.as_str(), *tint)),
                _ => None,
            })
            .collect();
        assert_eq!(icons.len(), 3);
        assert!(icons[0].0.contains("shape-rendering=\"crispEdges\""));
        assert!(icons[1].0.contains("shape-rendering=\"crispEdges\""));
        assert!(!icons[2].0.contains("shape-rendering=\"crispEdges\""));
        assert!(icons.iter().all(|(source, tint)| {
            source.contains("fill=\"currentColor\"")
                && source.contains("color=\"#")
                && tint.is_none()
        }));
    }

    #[test]
    fn title_bar_icon_color_follows_theme() {
        let icon_color = |style: TitleBarStyle| {
            let mut view = View::new(410.0, 100.0);
            view.title_bar("t", style, false);
            let document = view.build(measure);
            document
                .elements
                .iter()
                .find_map(|element| match element {
                    Element::WindowIcon { color, .. } => Some(*color),
                    _ => None,
                })
                .unwrap()
        };
        assert_ne!(
            icon_color(Theme::light().title_bar()),
            icon_color(Theme::dark().title_bar())
        );
    }
}
