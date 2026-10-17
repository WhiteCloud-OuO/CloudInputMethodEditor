//! 脚本窗口 / 面板的内容：一份**声明式**的元素表（display list）。
//!
//! 脚本跑在 Server 的工人线程上，窗口在 UI 线程上：两边不能直接互相调用，所以脚本不「画」，
//! 只交一份数据（坐标一律是**点**、左上角原点），由 UI 线程经 [`crate::Renderer::render_document`]
//! 一次性出图。元素就是渲染器那七类：`rect`（含圆角 / 圆）/ `line` / `text` / `svg` / `img` / `path`。
//!
//! 内容变了就整份替换（没有「只改某一个元素」的口子）—— 与窗口的渲染模型一致：一块内存区域 = 一帧。

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

    /// 图像（已经解码好的预乘 RGBA 位图）。
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        image: Pixmap,
    },

    /// 路径：按**非零环绕**填充。
    Path {
        commands: Vec<PathCommand>,
        color: Color,
    },
}

/// 元素在内容区里的包围盒（**点**）：鼠标命中测试用它（按外框算，不抠形状）。
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

/// 把路径命令换成 tiny-skia 的路径（像素坐标由 `scale` 换算）。空路径 / 画不出来返回 `None`。
pub(crate) fn build_path(commands: &[PathCommand], scale: f32) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let px = |value: f32| value * scale;
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
