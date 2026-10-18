//! 脚本窗口 / 面板的内容：一份**声明式**的元素表（display list）。
//!
//! 脚本跑在 Server 的工人线程上，窗口在 UI 线程上：两边不能直接互相调用，所以脚本不「画」，
//! 只交一份数据（坐标一律是**点**、左上角原点），由 UI 线程经 [`crate::Renderer::render_document`]
//! 一次性出图。元素就是渲染器那七类：`rect`（含圆角 / 圆）/ `line` / `text` / `svg` / `img` / `path`。
//!
//! 内容变了就整份替换（没有「只改某一个元素」的口子）—— 与窗口的渲染模型一致：一块内存区域 = 一帧。

use std::sync::Arc;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use tiny_skia::{PathBuilder, Pixmap};

use crate::color::Color;
use crate::control::{
    FlatSvgButton, Label, Panel, ScrollBar, StaticImage, StaticShape, TextBlock, TextBox, TextEdit,
};

/// 一份内容：内容区尺寸（点）+ 元素表（按顺序画，后面的盖前面的）。
#[derive(Clone)]
pub struct Document {
    /// 内容区宽（点）。
    pub width: f32,

    /// 内容区高（点）。
    pub height: f32,

    /// 要画的元素；空表就是一块空面板。
    pub elements: Vec<Element>,

    /// 可交互的**区域**（点击 / 拖拽的命中范围）；空表 = 没有区域（元素仍可单独命中）。
    pub regions: Vec<Region>,

    /// JSON 布局里的控件名：名字 → 元素路径；脚本 / 事件按它认控件。
    pub control_names: Vec<ControlName>,
}

/// 一个控件的名字与它在内容树里的位置（根元素从 0 开始，`children` 继续往下）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlName {
    /// 布局文件里的 `name`，同一份布局内唯一。
    pub name: String,

    /// 元素路径：`[根元素下标]` 或 `[根元素下标, 子元素下标, …]`（0 起）。
    pub path: Vec<usize>,
}

/// 一个 `text_edit` 在内容区里的位置（控件左上角，点）；嵌套在面板里时由所有父面板偏移累加。
#[derive(Debug, Clone, PartialEq)]
pub struct TextEditPlacement {
    /// 元素路径（0 起），可交给 [`Document::element_at_path`] 取回控件。
    pub path: Vec<usize>,

    /// 控件左上角的绝对内容坐标（点）。
    pub x: f32,
    pub y: f32,
}

impl Document {
    /// 内容里所有 `text_edit`（含面板里的）的绝对位置。
    pub fn text_edits(&self) -> Vec<TextEditPlacement> {
        let mut placements = Vec::new();
        collect_text_edits(&self.elements, (0.0, 0.0), &mut Vec::new(), &mut placements);
        placements
    }

    /// 内容里所有 `text_box` 的绝对位置。
    pub fn text_boxes(&self) -> Vec<TextEditPlacement> {
        let mut placements = Vec::new();
        collect_text_boxes(&self.elements, (0.0, 0.0), &mut Vec::new(), &mut placements);
        placements
    }

    /// 按元素路径取可变元素。
    pub fn element_at_path_mut(&mut self, path: &[usize]) -> Option<&mut Element> {
        let (first, rest) = path.split_first()?;
        let mut element = self.elements.get_mut(*first)?;
        for index in rest {
            let Element::Panel(panel) = element else {
                return None;
            };
            element = panel.children.get_mut(*index)?;
        }
        Some(element)
    }

    /// 按元素路径取回一个元素。
    pub fn element_at_path(&self, path: &[usize]) -> Option<&Element> {
        let (first, rest) = path.split_first()?;
        let mut element = self.elements.get(*first)?;
        for index in rest {
            let Element::Panel(panel) = element else {
                return None;
            };
            element = panel.children.get(*index)?;
        }
        Some(element)
    }

    /// 按元素路径找控件名（JSON 布局里每个控件都有）。
    pub fn control_name_at_path(&self, path: &[usize]) -> Option<&str> {
        self.control_names
            .iter()
            .find(|control| control.path == path)
            .map(|control| control.name.as_str())
    }
}

fn collect_text_edits(
    elements: &[Element],
    origin: (f32, f32),
    path: &mut Vec<usize>,
    placements: &mut Vec<TextEditPlacement>,
) {
    for (index, element) in elements.iter().enumerate() {
        path.push(index);
        match element {
            Element::TextEdit(edit) => placements.push(TextEditPlacement {
                path: path.clone(),
                x: origin.0 + edit.x,
                y: origin.1 + edit.y,
            }),
            Element::Panel(panel) => collect_text_edits(
                &panel.children,
                (origin.0 + panel.x, origin.1 + panel.y),
                path,
                placements,
            ),
            _ => {}
        }
        path.pop();
    }
}

fn collect_text_boxes(
    elements: &[Element],
    origin: (f32, f32),
    path: &mut Vec<usize>,
    placements: &mut Vec<TextEditPlacement>,
) {
    for (index, element) in elements.iter().enumerate() {
        path.push(index);
        match element {
            Element::TextBox(edit) => placements.push(TextEditPlacement {
                path: path.clone(),
                x: origin.0 + edit.x,
                y: origin.1 + edit.y,
            }),
            Element::Panel(panel) => collect_text_boxes(
                &panel.children,
                (origin.0 + panel.x, origin.1 + panel.y),
                path,
                placements,
            ),
            _ => {}
        }
        path.pop();
    }
}

impl Document {
    /// 只要一块 `width` × `height` 的空面板。
    pub fn empty(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            elements: Vec::new(),
            regions: Vec::new(),
            control_names: Vec::new(),
        }
    }

    /// 命中的根元素（事件里的 `element`，1 起）对应的控件名。
    pub fn control_name(&self, element: usize) -> Option<&str> {
        let index = element.checked_sub(1)?;
        self.control_names
            .iter()
            .find(|control| control.path.len() == 1 && control.path[0] == index)
            .map(|control| control.name.as_str())
    }
}

/// 窗口里的一个**区域**（region）：一块可以被鼠标点击 / 拖拽的范围。
///
/// 坐标单位跟内容一致（脚本窗口是点、悬浮工具栏是像素），`shape` 与元素的命中形状同一套。
/// 命中范围可以再**合并**（并集）与**剪除**（差集）：`combined` 里的形状只要命中就算命中，
/// `pruned` 里的形状命中就一律不算 —— 与「绿框并上蓝框」「红框减掉黑框」的几何结果一致，
/// 只是命中检测不需要把轮廓真的算出来。
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// 名字：同一窗口里唯一，鼠标事件里带它（`event.region`）。
    pub name: String,

    /// 命中范围（与元素命中同一套形状：矩形 / 圆角 / 圆 / 线段 / 路径）。
    pub shape: HitShape,

    /// **合并**进来的形状（并集）：任一命中就命中。
    pub combined: Vec<HitShape>,

    /// **剪除**掉的形状（差集）：任一命中就不命中（优先于原始形状与 `combined`）。
    pub pruned: Vec<HitShape>,

    /// 事件响应表：事件名（`mouse_clicked` 等）→ 处理器名；窗口事件里带 `handler`。
    pub handlers: Vec<(String, String)>,

    /// 拖这块区域时移动整块窗口；`false` = 按住它不拖窗口（留给以后元素自己处理，比如滑块）。
    pub draggable: bool,
}

impl Region {
    /// 一个只有基础形状的区域（还没有合并 / 剪除）。
    pub fn new(name: impl Into<String>, shape: HitShape, draggable: bool) -> Self {
        Self {
            name: name.into(),
            shape,
            combined: Vec::new(),
            pruned: Vec::new(),
            handlers: Vec::new(),
            draggable,
        }
    }

    /// 这个区域对某个事件声明的处理器名。扁平按钮的 `mouse_clicked` 也接受 `click` 别名，
    /// 双击接受 `double_click`；真正派发的动作名仍是 `mouse_clicked` / `mouse_double_clicked`。
    pub fn handler(&self, action: &str) -> Option<&str> {
        if let Some(handler) = self
            .handlers
            .iter()
            .find(|(name, _)| name == action)
            .map(|(_, handler)| handler.as_str())
        {
            return Some(handler);
        }
        let alias = match action {
            "mouse_clicked" => "click",
            "mouse_double_clicked" => "double_click",
            _ => return None,
        };
        self.handlers
            .iter()
            .find(|(name, _)| name == alias)
            .map(|(_, handler)| handler.as_str())
    }

    /// 点是不是落在这个区域里：先看剪除（剪掉的优先），再看原始形状与合并进来的。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        if self.pruned.iter().any(|shape| shape.contains(x, y)) {
            return false;
        }
        self.shape.contains(x, y) || self.combined.iter().any(|shape| shape.contains(x, y))
    }

    /// **合并**：把 `shape` 并进这个区域（命中范围取并集）。
    pub fn combine(&mut self, shape: HitShape) {
        self.combined.push(shape);
    }

    /// **剪除**：把这个区域里的 `shape` 挖掉（命中范围取差集）。
    pub fn prune(&mut self, shape: HitShape) {
        self.pruned.push(shape);
    }

    /// **内含**：`self` 完全覆盖 `other`（`other` 减掉 `self` 之后没有面积）；空区域不算。
    pub fn contains_region(&self, other: &Region) -> bool {
        let a = self.effective_contours();
        let b = other.effective_contours();
        if a.is_empty() || b.is_empty() {
            return false;
        }
        let rest = b.overlay(&a, OverlayRule::Difference, FillRule::NonZero);
        shapes_area(&rest) <= AREA_EPSILON
    }

    /// **相交**：两块区域有**面积**上的重叠（只贴一条边不算相交，算相离）。
    pub fn intersects(&self, other: &Region) -> bool {
        let a = self.effective_contours();
        let b = other.effective_contours();
        if a.is_empty() || b.is_empty() {
            return false;
        }
        let overlap = a.overlay(&b, OverlayRule::Intersect, FillRule::NonZero);
        shapes_area(&overlap) > AREA_EPSILON
    }

    /// 实际命中轮廓：原始形状 ∪ `combined` − `pruned`（用多边形布尔运算算出来）。
    fn effective_contours(&self) -> Vec<Contour> {
        let mut subject = shape_contours(&self.shape);
        for shape in &self.combined {
            let clip = shape_contours(shape);
            subject = flatten(subject.overlay(&clip, OverlayRule::Union, FillRule::NonZero));
        }
        for shape in &self.pruned {
            let clip = shape_contours(shape);
            subject = flatten(subject.overlay(&clip, OverlayRule::Difference, FillRule::NonZero));
        }
        subject
    }
}

/// 多边形布尔运算用的轮廓（点集，自动闭合）。
type Contour = Vec<[f64; 2]>;

/// 一块有孔洞的形状（第一个轮廓是外圈，后面的圈是洞）。
type PolygonShape = Vec<Contour>;

/// 关系判断里认为「有面积」的下限（点²）。
const AREA_EPSILON: f64 = 1e-3;

/// 元素 / 区域的形状 → 多边形轮廓（圆角 / 圆 / 线段都按折线近似；路径用它自己的扁平化轮廓）。
fn shape_contours(shape: &HitShape) -> Vec<Contour> {
    match shape {
        HitShape::Rect(rect) => vec![vec![
            [f64::from(rect.x), f64::from(rect.y)],
            [f64::from(rect.x + rect.width), f64::from(rect.y)],
            [
                f64::from(rect.x + rect.width),
                f64::from(rect.y + rect.height),
            ],
            [f64::from(rect.x), f64::from(rect.y + rect.height)],
        ]],
        HitShape::RoundRect {
            x,
            y,
            width,
            height,
            radius,
        } => vec![round_rect_contour(*x, *y, *width, *height, *radius)],
        HitShape::Circle { cx, cy, radius } => vec![circle_contour(*cx, *cy, *radius)],
        HitShape::Segment {
            from,
            to,
            thickness,
        } => vec![segment_contour(*from, *to, *thickness)],
        HitShape::Path { contours } => contours
            .iter()
            .map(|contour| {
                contour
                    .iter()
                    .map(|&(x, y)| [f64::from(x), f64::from(y)])
                    .collect()
            })
            .collect(),
    }
}

/// 圆角矩形的轮廓（每个角 16 段折线）。
fn round_rect_contour(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Contour {
    const ARC_SEGMENTS: usize = 16;
    let radius = f64::from(radius.min(width.min(height) * 0.5).max(0.0));
    let (x, y, width, height) = (
        f64::from(x),
        f64::from(y),
        f64::from(width),
        f64::from(height),
    );
    if radius <= 0.0 {
        return vec![
            [x, y],
            [x + width, y],
            [x + width, y + height],
            [x, y + height],
        ];
    }
    let mut points = Vec::with_capacity(ARC_SEGMENTS * 4 + 4);
    let arc = |points: &mut Contour, cx: f64, cy: f64, from: f64, to: f64| {
        for step in 1..=ARC_SEGMENTS {
            let angle = from + (to - from) * step as f64 / ARC_SEGMENTS as f64;
            points.push([cx + radius * angle.cos(), cy + radius * angle.sin()]);
        }
    };
    points.push([x + radius, y]);
    points.push([x + width - radius, y]);
    arc(
        &mut points,
        x + width - radius,
        y + radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    points.push([x + width, y + height - radius]);
    arc(
        &mut points,
        x + width - radius,
        y + height - radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    points.push([x + radius, y + height]);
    arc(
        &mut points,
        x + radius,
        y + height - radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    points.push([x, y + radius]);
    arc(
        &mut points,
        x + radius,
        y + radius,
        std::f64::consts::PI,
        std::f64::consts::PI + std::f64::consts::FRAC_PI_2,
    );
    // 最后一段圆弧的终点就是起点，去掉重复点
    points.pop();
    points
}

/// 圆的轮廓（64 段折线）。
fn circle_contour(cx: f32, cy: f32, radius: f32) -> Contour {
    const SEGMENTS: usize = 64;
    let (cx, cy, radius) = (f64::from(cx), f64::from(cy), f64::from(radius.max(0.0)));
    (0..SEGMENTS)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / SEGMENTS as f64;
            [cx + radius * angle.cos(), cy + radius * angle.sin()]
        })
        .collect()
}

/// 线段按「细长矩形」算（区域关系用；命中仍按点到线段的距离）。
fn segment_contour(from: (f32, f32), to: (f32, f32), thickness: f32) -> Contour {
    let (x1, y1) = (f64::from(from.0), f64::from(from.1));
    let (x2, y2) = (f64::from(to.0), f64::from(to.1));
    let (dx, dy) = (x2 - x1, y2 - y1);
    let length = (dx * dx + dy * dy).sqrt();
    let half = f64::from(thickness.max(0.1)) * 0.5;
    if length <= f64::EPSILON {
        return circle_contour(from.0, from.1, thickness.max(0.1) * 0.5);
    }
    let (nx, ny) = (-dy / length * half, dx / length * half);
    vec![
        [x1 + nx, y1 + ny],
        [x2 + nx, y2 + ny],
        [x2 - nx, y2 - ny],
        [x1 - nx, y1 - ny],
    ]
}

/// `Vec<Shape>`（布尔运算的结果）→ 扁平轮廓表。
fn flatten(shapes: Vec<PolygonShape>) -> Vec<Contour> {
    shapes.into_iter().flatten().collect()
}

/// 一组轮廓的净面积（外圈正、洞负；i_overlay 的输出方向就是外圈逆时针、洞顺时针）。
fn shapes_area(shapes: &[PolygonShape]) -> f64 {
    let mut total = 0.0;
    for shape in shapes {
        for contour in shape {
            total += signed_area(contour);
        }
    }
    total.abs()
}

/// 鞋带公式：逆时针为正、顺时针为负。
fn signed_area(contour: &Contour) -> f64 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for index in 0..contour.len() {
        let [x1, y1] = contour[index];
        let [x2, y2] = contour[(index + 1) % contour.len()];
        sum += x1 * y2 - x2 * y1;
    }
    sum * 0.5
}

/// 字体槽：与 `cloudime.ui.measure` 的字体名同一套（候选 / 拼音 / 序号 / 译文）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontSlot {
    /// 候选词字体（缺省）。
    #[default]
    Candidate,

    /// 拼音行字体。
    Pinyin,

    /// 序号字体。
    ItemNumber,

    /// 译文 / 提示那一行的字体。
    Translate,
}

impl FontSlot {
    /// 脚本写的名字 → 字体槽；写别的返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "candidate" => Some(Self::Candidate),
            "pinyin" => Some(Self::Pinyin),
            "item_number" => Some(Self::ItemNumber),
            "translate" => Some(Self::Translate),
            _ => None,
        }
    }
}

/// 一条路径命令（`tiny_skia::PathBuilder` 那几笔）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    /// 起一笔。
    MoveTo(f32, f32),

    /// 直线到。
    LineTo(f32, f32),

    /// 二次贝塞尔：控制点 + 终点。
    QuadTo(f32, f32, f32, f32),

    /// 三次贝塞尔：两个控制点 + 终点。
    CubicTo(f32, f32, f32, f32, f32, f32),

    /// 闭合当前子路径。
    Close,
}

/// 一类元素（坐标 / 尺寸都是**点**）。
#[derive(Clone)]
pub enum Element {
    /// 矩形；`radius > 0` 是圆角矩形，`radius` 超过半边长按半边长算。
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: Color,
    },

    /// 线条：`from` → `to`，粗 `thickness`。
    Line {
        from: (f32, f32),
        to: (f32, f32),
        thickness: f32,
        color: Color,
    },

    /// 圆：圆心 + 半径。
    Circle {
        cx: f32,
        cy: f32,
        radius: f32,
        color: Color,
    },

    /// 一段文字；`(x, y)` 是行框左上角（不是基线），`size` 是字号（点）。
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        slot: FontSlot,
        color: Color,
        italic: bool,
    },

    /// SVG：按自身宽高比等比塞进 `size` 见方的框；`tint` 给 `Some` 时整张按该颜色染。
    Svg {
        x: f32,
        y: f32,
        size: f32,
        source: String,
        tint: Option<Color>,
    },

    /// 图像（已经解码好的预乘 RGBA 位图；`Arc` 让同一张图在多份内容里共享，不复制像素）。
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        image: Arc<Pixmap>,
    },

    /// 路径：按**非零环绕**填充。
    Path {
        commands: Vec<PathCommand>,
        color: Color,
    },

    /// 静态控件：标签（每段可单独着色）。
    Label(Label),

    /// 静态控件：多行文本块（统一颜色、可调行距）。
    TextBlock(TextBlock),

    /// 静态控件：直线 / 矩形 / 圆角矩形 / 圆 / 路径。
    StaticShape(StaticShape),

    /// 静态控件：图像（文件 / 内存句柄在脚本侧解析成位图）。
    StaticImage(StaticImage),

    /// 静态控件：面板容器（背景 + 相对坐标的子控件）。
    Panel(Panel),

    /// 动态控件：扁平 SVG 按钮（背景形状 / 区域 / SVG 三件套，状态由窗口自动切换）。
    FlatSvgButton(FlatSvgButton),

    /// 动态控件：水平 / 垂直滚动条（背景胶囊 + 滑块胶囊）。
    ScrollBar(ScrollBar),

    /// 动态控件：单行文本编辑框（背景 / SVG 由渲染器画，文本区由 Windowless RichEdit 承载）。
    TextBox(TextBox),

    TextEdit(TextEdit),
}

/// 元素在内容区里的命中区域（**点**）。
///
/// 早期只有外框（[`HitRect`]），2026-10-17 起按形状算：圆角矩形的四角、圆的边界、
/// 线段的粗细、路径的填充（非零环绕）都抠出来了 —— 只有文字 / 图像 / SVG 仍是外框。
#[derive(Debug, Clone, PartialEq)]
pub enum HitShape {
    /// 直角矩形（文字、图像、SVG 也用它）。
    Rect(HitRect),

    /// 圆角矩形：落在四个角的圆外面就不算命中。
    RoundRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },

    /// 圆。
    Circle { cx: f32, cy: f32, radius: f32 },

    /// 线段：到线段的距离不超过半个线宽。
    Segment {
        from: (f32, f32),
        to: (f32, f32),
        thickness: f32,
    },

    /// 路径填充（非零环绕）：`contours` 是扁平化后的闭合轮廓（内容区坐标）。
    Path { contours: Vec<Vec<(f32, f32)>> },
}

impl From<HitRect> for HitShape {
    fn from(rect: HitRect) -> Self {
        Self::Rect(rect)
    }
}

impl HitShape {
    /// 由路径命令造一个**闭合填充**形状（与元素的 `path` 同一套扁平化 + 非零环绕）。
    pub fn from_path(commands: &[PathCommand]) -> Option<Self> {
        let contours = path_contours(commands);
        (!contours.is_empty()).then_some(Self::Path { contours })
    }

    /// 点是不是落在这块区域里（含边界）。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Rect(rect) => rect.contains(x, y),
            Self::RoundRect {
                x: left,
                y: top,
                width,
                height,
                radius,
            } => round_rect_contains(*left, *top, *width, *height, *radius, x, y),
            Self::Circle { cx, cy, radius } => {
                let (dx, dy) = (x - cx, y - cy);
                dx * dx + dy * dy <= radius * radius
            }
            Self::Segment {
                from,
                to,
                thickness,
            } => segment_distance(*from, *to, x, y) <= thickness * 0.5,
            Self::Path { contours } => winding_number(contours, x, y) != 0,
        }
    }
}

/// 圆角矩形：先落在矩形里，再对四个角做一次圆判定。`radius` 超过半边长按半边长算。
fn round_rect_contains(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    px: f32,
    py: f32,
) -> bool {
    if px < x || px > x + width || py < y || py > y + height {
        return false;
    }
    let radius = radius.clamp(0.0, width.min(height) * 0.5);
    if radius <= 0.0 {
        return true;
    }
    // 落在哪个角的那块正方形里？只有那里才可能出圆
    let cx = if px < x + radius {
        x + radius
    } else if px > x + width - radius {
        x + width - radius
    } else {
        return true;
    };
    let cy = if py < y + radius {
        y + radius
    } else if py > y + height - radius {
        y + height - radius
    } else {
        return true;
    };
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= radius * radius
}

/// 点到线段的最短距离。
fn segment_distance(from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return ((x - from.0).powi(2) + (y - from.1).powi(2)).sqrt();
    }
    let t = (((x - from.0) * dx + (y - from.1) * dy) / length_squared).clamp(0.0, 1.0);
    let (px, py) = (from.0 + t * dx, from.1 + t * dy);
    ((x - px).powi(2) + (y - py).powi(2)).sqrt()
}

/// 非零环绕规则：点被几条轮廓按同一方向绕了几圈（与 tiny-skia 的默认填充一致）。
fn winding_number(contours: &[Vec<(f32, f32)>], x: f32, y: f32) -> i32 {
    let mut winding = 0;
    for contour in contours {
        if contour.len() < 3 {
            continue;
        }
        for index in 0..contour.len() {
            let (x1, y1) = contour[index];
            let (x2, y2) = contour[(index + 1) % contour.len()];
            let side = (x2 - x1) * (y - y1) - (x - x1) * (y2 - y1);
            if y1 <= y {
                if y2 > y && side > 0.0 {
                    winding += 1;
                }
            } else if y2 <= y && side < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

/// 把路径命令**扁平化**成闭合轮廓（内容区坐标，点）：二次 / 三次贝塞尔按 `TOLERANCE` 递归细分。
///
/// 命中测试用非零环绕判定填充，与渲染时的 `fill_path` 同一套规则。开放子路径按「填充分会
/// 隐式闭合」处理（首尾相连）。
pub(crate) fn path_contours(commands: &[PathCommand]) -> Vec<Vec<(f32, f32)>> {
    /// 扁平化误差（点）：小于半个像素，肉眼与命中都看不出差别。
    const TOLERANCE: f32 = 0.25;

    let mut contours: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut cursor = (0.0_f32, 0.0_f32);
    let mut start = cursor;
    let flush = |current: &mut Vec<(f32, f32)>, contours: &mut Vec<Vec<(f32, f32)>>| {
        if current.len() >= 2 {
            contours.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => {
                flush(&mut current, &mut contours);
                cursor = (x, y);
                start = cursor;
                current.push(cursor);
            }
            PathCommand::LineTo(x, y) => {
                cursor = (x, y);
                current.push(cursor);
            }
            PathCommand::QuadTo(cx, cy, x, y) => {
                flatten_quad(cursor, (cx, cy), (x, y), 0, &mut current, TOLERANCE);
                cursor = (x, y);
            }
            PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                flatten_cubic(
                    cursor,
                    (cx1, cy1),
                    (cx2, cy2),
                    (x, y),
                    0,
                    &mut current,
                    TOLERANCE,
                );
                cursor = (x, y);
            }
            PathCommand::Close => {
                flush(&mut current, &mut contours);
                // 闭合之后再画线从子路径起点算：tiny-skia 也是这个语义
                cursor = start;
                current.push(cursor);
            }
        }
    }
    flush(&mut current, &mut contours);
    contours
}

/// 递归细分的深度上限：极端控制点也不会把栈打穿。
const MAX_FLATTEN_DEPTH: u8 = 10;

fn flatten_quad(
    from: (f32, f32),
    control: (f32, f32),
    to: (f32, f32),
    depth: u8,
    out: &mut Vec<(f32, f32)>,
    tolerance: f32,
) {
    if depth >= MAX_FLATTEN_DEPTH || quad_is_flat(from, control, to, tolerance) {
        out.push(to);
        return;
    }
    let mid = midpoint(from, control);
    let mid2 = midpoint(control, to);
    let center = midpoint(mid, mid2);
    flatten_quad(from, mid, center, depth + 1, out, tolerance);
    flatten_quad(center, mid2, to, depth + 1, out, tolerance);
}

fn flatten_cubic(
    from: (f32, f32),
    control1: (f32, f32),
    control2: (f32, f32),
    to: (f32, f32),
    depth: u8,
    out: &mut Vec<(f32, f32)>,
    tolerance: f32,
) {
    if depth >= MAX_FLATTEN_DEPTH || cubic_is_flat(from, control1, control2, to, tolerance) {
        out.push(to);
        return;
    }
    let p01 = midpoint(from, control1);
    let p12 = midpoint(control1, control2);
    let p23 = midpoint(control2, to);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let center = midpoint(p012, p123);
    flatten_cubic(from, p01, p012, center, depth + 1, out, tolerance);
    flatten_cubic(center, p123, p23, to, depth + 1, out, tolerance);
}

fn midpoint(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// 控制点到弦的距离够近就算平了（二次贝塞尔）。
fn quad_is_flat(from: (f32, f32), control: (f32, f32), to: (f32, f32), tolerance: f32) -> bool {
    segment_distance(from, to, control.0, control.1) <= tolerance
}

/// 两个控制点到弦的距离都够近就算平了（三次贝塞尔）。
fn cubic_is_flat(
    from: (f32, f32),
    control1: (f32, f32),
    control2: (f32, f32),
    to: (f32, f32),
    tolerance: f32,
) -> bool {
    segment_distance(from, to, control1.0, control1.1) <= tolerance
        && segment_distance(from, to, control2.0, control2.1) <= tolerance
}

/// 元素在内容区里的包围盒（**点**）：文字 / 图像 / SVG 的命中区域用它（这些不抠形状）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl HitRect {
    /// 由左上角与尺寸造一个；尺寸非正也算一个零面积的框（命中不到）。
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 点是不是落在框里（含边界）。
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// 把路径命令换成 tiny-skia 的路径：坐标乘 `scale` 换成像素，再整体加 `offset`（阴影留白）。
/// 空路径 / 画不出来返回 `None`。
pub(crate) fn build_path(
    commands: &[PathCommand],
    scale: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let px = |value: f32| offset_x + value * scale;
    let py = |value: f32| offset_y + value * scale;
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => builder.move_to(px(x), py(y)),
            PathCommand::LineTo(x, y) => builder.line_to(px(x), py(y)),
            PathCommand::QuadTo(cx, cy, x, y) => builder.quad_to(px(cx), py(cy), px(x), py(y)),
            PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                builder.cubic_to(px(cx1), py(cy1), px(cx2), py(cy2), px(x), py(y));
            }
            PathCommand::Close => builder.close(),
        }
    }
    builder.finish()
}

/// 主题里的四项字体 → 字族名（`None` = 系统界面字体）。字号由脚本给，行高按 [`FontSpec::with_size`] 算。
pub(crate) fn family_for(theme: &crate::theme::Theme, slot: FontSlot) -> Option<&String> {
    match slot {
        FontSlot::Candidate => theme.text_family.as_ref(),
        FontSlot::Pinyin => theme.pinyin_family.as_ref(),
        FontSlot::ItemNumber => theme.index_family.as_ref(),
        FontSlot::Translate => theme.translate_family.as_ref(),
    }
}

/// 手写 `Debug`：图像只报尺寸，不把整块像素倒出来（日志 / 测试里看着舒服）。
impl std::fmt::Debug for Element {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rect {
                x,
                y,
                width,
                height,
                radius,
                color,
            } => f
                .debug_struct("Rect")
                .field("x", x)
                .field("y", y)
                .field("width", width)
                .field("height", height)
                .field("radius", radius)
                .field("color", color)
                .finish(),
            Self::Line {
                from,
                to,
                thickness,
                color,
            } => f
                .debug_struct("Line")
                .field("from", from)
                .field("to", to)
                .field("thickness", thickness)
                .field("color", color)
                .finish(),
            Self::Circle {
                cx,
                cy,
                radius,
                color,
            } => f
                .debug_struct("Circle")
                .field("cx", cx)
                .field("cy", cy)
                .field("radius", radius)
                .field("color", color)
                .finish(),
            Self::Text {
                x,
                y,
                text,
                size,
                slot,
                color,
                italic,
            } => f
                .debug_struct("Text")
                .field("x", x)
                .field("y", y)
                .field("text", text)
                .field("size", size)
                .field("slot", slot)
                .field("color", color)
                .field("italic", italic)
                .finish(),
            Self::Svg {
                x,
                y,
                size,
                source,
                tint,
            } => f
                .debug_struct("Svg")
                .field("x", x)
                .field("y", y)
                .field("size", size)
                .field("source_len", &source.len())
                .field("tint", tint)
                .finish(),
            Self::Image {
                x,
                y,
                width,
                height,
                image,
            } => f
                .debug_struct("Image")
                .field("x", x)
                .field("y", y)
                .field("width", width)
                .field("height", height)
                .field("pixels", &(image.width(), image.height()))
                .finish(),
            Self::Path { commands, color } => f
                .debug_struct("Path")
                .field("commands", &commands.len())
                .field("color", color)
                .finish(),
            Self::Label(label) => f
                .debug_struct("Label")
                .field("runs", &label.runs.len())
                .finish(),
            Self::TextBlock(text) => f
                .debug_struct("TextBlock")
                .field("chars", &text.text.chars().count())
                .finish(),
            Self::StaticShape(_) => f.write_str("StaticShape"),
            Self::StaticImage(image) => f
                .debug_struct("StaticImage")
                .field("pixels", &(image.image.width(), image.image.height()))
                .finish(),
            Self::Panel(panel) => f
                .debug_struct("Panel")
                .field("children", &panel.children.len())
                .finish(),
            Self::FlatSvgButton(button) => f
                .debug_struct("FlatSvgButton")
                .field("state", &button.state)
                .finish(),
            Self::ScrollBar(bar) => f
                .debug_struct("ScrollBar")
                .field("axis", &bar.axis)
                .field("value", &bar.value)
                .field("min", &bar.min)
                .field("max", &bar.max)
                .finish(),
            Self::TextBox(edit) => f.debug_struct("TextBox").field("text", &edit.text).finish(),
            Self::TextEdit(edit) => f
                .debug_struct("TextEdit")
                .field("text", &edit.text)
                .finish(),
        }
    }
}

/// 手写 `Debug`：只报尺寸与元素个数。
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("elements", &self.elements.len())
            .field("regions", &self.regions.len())
            .field("control_names", &self.control_names.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 圆角矩形：四个角切掉的那块不算命中，边与中间算。
    #[test]
    fn the_round_rect_cuts_its_corners_out() {
        let shape = HitShape::RoundRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
            radius: 10.0,
        };
        assert!(shape.contains(1.0, 25.0), "左边中部");
        assert!(shape.contains(50.0, 50.0), "底边中部");
        assert!(shape.contains(50.0, 25.0), "正中间");
        assert!(!shape.contains(0.5, 0.5), "左上角在圆外");
        assert!(!shape.contains(99.5, 49.5), "右下角在圆外");
        assert!(shape.contains(10.0, 10.0), "圆心那一点算命中");
        assert!(!shape.contains(-1.0, 25.0), "矩形外");
    }

    /// 圆不是它的外框：四个角的空白点不着。
    #[test]
    fn a_circle_is_not_its_bounding_box() {
        let shape = HitShape::Circle {
            cx: 50.0,
            cy: 50.0,
            radius: 20.0,
        };
        assert!(shape.contains(50.0, 50.0));
        assert!(shape.contains(69.0, 50.0));
        assert!(!shape.contains(31.0, 31.0), "外框的角落");
        assert!(!shape.contains(50.0, 71.0));
    }

    /// 线段按粗细算：贴着线的点命中，离开半个线宽就不算。
    #[test]
    fn a_line_hits_within_half_its_thickness() {
        let shape = HitShape::Segment {
            from: (0.0, 0.0),
            to: (100.0, 0.0),
            thickness: 4.0,
        };
        assert!(shape.contains(50.0, 1.5));
        assert!(shape.contains(50.0, -1.9));
        assert!(!shape.contains(50.0, 2.5));
        assert!(!shape.contains(103.0, 0.0), "线段两端之外（超过半个线宽）");
    }

    /// 路径按**非零环绕**判填充：外圈顺时针、内圈逆时针 → 中间的洞点不着。
    #[test]
    fn a_path_fills_with_the_nonzero_rule_and_keeps_holes() {
        let commands = vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(100.0, 0.0),
            PathCommand::LineTo(100.0, 100.0),
            PathCommand::LineTo(0.0, 100.0),
            PathCommand::Close,
            PathCommand::MoveTo(25.0, 25.0),
            PathCommand::LineTo(25.0, 75.0),
            PathCommand::LineTo(75.0, 75.0),
            PathCommand::LineTo(75.0, 25.0),
            PathCommand::Close,
        ];
        let shape = HitShape::Path {
            contours: path_contours(&commands),
        };
        assert!(shape.contains(10.0, 50.0), "外圈与内圈之间");
        assert!(!shape.contains(50.0, 50.0), "洞的中间");
        assert!(!shape.contains(120.0, 50.0), "路径外");
    }

    /// 曲线按扁平化算：二次贝塞尔的拱形里算命中、拱外不算。
    #[test]
    fn a_curved_path_is_flattened_for_the_hit_test() {
        let commands = vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::QuadTo(50.0, 100.0, 100.0, 0.0),
            PathCommand::Close,
        ];
        let shape = HitShape::Path {
            contours: path_contours(&commands),
        };
        assert!(shape.contains(50.0, 20.0), "拱形里面");
        assert!(!shape.contains(50.0, 60.0), "拱形外面（曲线下方）");
    }

    /// 区域的合并（并集）与剪除（差集）：与「绿框并蓝框」「红框减黑框」的几何结果一致。
    #[test]
    fn combining_and_pruning_change_the_hit_area() {
        let rect = |x, y, width, height| HitShape::Rect(HitRect::new(x, y, width, height));

        // 合并：绿框 ∪ 蓝框（蓝框在右下，重叠一块）
        let mut union = Region::new("union", rect(0.0, 0.0, 100.0, 100.0), true);
        union.combine(rect(50.0, 50.0, 100.0, 100.0));
        assert!(union.contains(25.0, 25.0), "绿框里");
        assert!(union.contains(60.0, 60.0), "重叠那块");
        assert!(union.contains(120.0, 60.0), "蓝框露出来的那一块");
        assert!(union.contains(60.0, 120.0), "蓝框下面那块");
        assert!(!union.contains(120.0, 25.0), "两个框都没盖住");
        assert!(!union.contains(160.0, 160.0));

        // 剪除：红框 − 黑框（黑框压在右下角）
        let mut cut = Region::new("cut", rect(0.0, 0.0, 100.0, 100.0), true);
        cut.prune(rect(50.0, 50.0, 50.0, 50.0));
        assert!(cut.contains(25.0, 25.0));
        assert!(cut.contains(80.0, 25.0), "黑框上边还在");
        assert!(!cut.contains(80.0, 80.0), "挖掉的角");
        assert!(!cut.contains(120.0, 25.0));

        // 剪除优先于合并：合并进来的那块也能挖掉
        let mut both = Region::new("both", rect(0.0, 0.0, 100.0, 100.0), true);
        both.combine(rect(100.0, 0.0, 50.0, 50.0));
        both.prune(rect(120.0, 10.0, 10.0, 10.0));
        assert!(both.contains(140.0, 40.0), "合并进来的");
        assert!(!both.contains(125.0, 15.0), "合并之后又挖掉");
    }

    /// 两个区域的关系：相离 / 相交 / 内含 / 重合；只贴边算相离。
    #[test]
    fn region_relations_cover_the_four_cases() {
        let rect = |x, y, width, height| {
            Region::new("r", HitShape::Rect(HitRect::new(x, y, width, height)), true)
        };
        let a = rect(0.0, 0.0, 100.0, 100.0);

        // 重合
        let same = rect(0.0, 0.0, 100.0, 100.0);
        assert!(a.contains_region(&same) && same.contains_region(&a));
        assert!(a.intersects(&same));

        // 内含（大包小）
        let inner = rect(20.0, 20.0, 40.0, 40.0);
        assert!(a.contains_region(&inner));
        assert!(!inner.contains_region(&a));
        assert!(a.intersects(&inner));

        // 相交（部分重叠）
        let overlap = rect(80.0, 80.0, 50.0, 50.0);
        assert!(!a.contains_region(&overlap));
        assert!(!overlap.contains_region(&a));
        assert!(a.intersects(&overlap));

        // 相离
        let far = rect(200.0, 200.0, 20.0, 20.0);
        assert!(!a.contains_region(&far));
        assert!(!a.intersects(&far));

        // 只贴边：不算相交（没有面积重叠），也不算内含
        let touching = rect(100.0, 0.0, 50.0, 50.0);
        assert!(!a.intersects(&touching));
        assert!(!a.contains_region(&touching));
    }

    /// 圆 / 圆角 / 带洞的区域也能判关系（多边形近似）。
    #[test]
    fn region_relations_work_with_circles_and_holes() {
        let circle = |cx, cy, radius| Region::new("c", HitShape::Circle { cx, cy, radius }, true);
        let big = circle(50.0, 50.0, 40.0);
        let small = circle(50.0, 50.0, 10.0);
        assert!(big.contains_region(&small));
        assert!(big.intersects(&small));
        let next = circle(85.0, 50.0, 10.0);
        assert!(big.intersects(&next), "两个圆部分重叠");
        assert!(!big.contains_region(&next));

        // 挖了洞的区域：洞里的小块与它相离，环上的小块与它相交
        let mut ring = Region::new(
            "ring",
            HitShape::Rect(HitRect::new(0.0, 0.0, 100.0, 100.0)),
            true,
        );
        ring.prune(HitShape::Rect(HitRect::new(25.0, 25.0, 50.0, 50.0)));
        let in_hole = Region::new(
            "hole",
            HitShape::Rect(HitRect::new(40.0, 40.0, 10.0, 10.0)),
            true,
        );
        assert!(!ring.intersects(&in_hole), "洞里的块不碰环");
        assert!(!ring.contains_region(&in_hole));
        let on_ring = Region::new(
            "edge",
            HitShape::Rect(HitRect::new(10.0, 10.0, 10.0, 10.0)),
            true,
        );
        assert!(ring.intersects(&on_ring));
        assert!(ring.contains_region(&on_ring));
    }
}
