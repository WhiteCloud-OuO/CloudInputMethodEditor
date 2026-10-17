//! 脚本窗口 / 面板的内容：一份**声明式**的元素表（display list）。
//!
//! 脚本跑在 Server 的工人线程上，窗口在 UI 线程上：两边不能直接互相调用，所以脚本不「画」，
//! 只交一份数据（坐标一律是**点**、左上角原点），由 UI 线程经 [`crate::Renderer::render_document`]
//! 一次性出图。元素就是渲染器那七类：`rect`（含圆角 / 圆）/ `line` / `text` / `svg` / `img` / `path`。
//!
//! 内容变了就整份替换（没有「只改某一个元素」的口子）—— 与窗口的渲染模型一致：一块内存区域 = 一帧。

use std::sync::Arc;

use tiny_skia::{PathBuilder, Pixmap};

use crate::color::Color;

/// 一份内容：内容区尺寸（点）+ 元素表（按顺序画，后面的盖前面的）。
#[derive(Clone)]
pub struct Document {
    /// 内容区宽（点）。
    pub width: f32,

    /// 内容区高（点）。
    pub height: f32,

    /// 要画的元素；空表就是一块空面板。
    pub elements: Vec<Element>,
}

impl Document {
    /// 只要一块 `width` × `height` 的空面板。
    pub fn empty(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            elements: Vec::new(),
        }
    }
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
    offset: f32,
) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let px = |value: f32| offset + value * scale;
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => builder.move_to(px(x), px(y)),
            PathCommand::LineTo(x, y) => builder.line_to(px(x), px(y)),
            PathCommand::QuadTo(cx, cy, x, y) => builder.quad_to(px(cx), px(cy), px(x), px(y)),
            PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                builder.cubic_to(px(cx1), px(cy1), px(cx2), px(cy2), px(x), px(y));
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
}
