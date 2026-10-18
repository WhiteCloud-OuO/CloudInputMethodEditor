//! 静态控件：用 [`crate::elements`] 的绘制 traits 组合出来的可复用显示件。
//!
//! 静态控件**没有命中框、不接受交互**：窗口真正能点的还是 `regions` / 元素。面板只负责
//! 分组与背景；子控件用面板内的相对坐标摆放，绘制时整体平移。
//!
//! 坐标与尺寸一律是**点**；绘制时按 `scale` 换成像素。

use std::sync::Arc;

use tiny_skia::{Path, PathBuilder, Pixmap};

use crate::color::Color;
use crate::document::{FontSlot, PathCommand, family_for};
use crate::elements::ControlRender;
use crate::text::TextStyle;
use crate::theme::{FontSpec, Theme};

/// 标签里的一小段：文字 + 自己的颜色。
#[derive(Debug, Clone)]
pub struct LabelRun {
    pub text: String,
    pub color: Color,
}

/// **标签**：一行文字，每段可以有自己的颜色（「云朵」蓝 + 「输入法」黑）。
#[derive(Debug, Clone)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    /// 字号（点）；行高按 [`FontSpec::with_size`] 算。
    pub size: f32,
    pub slot: FontSlot,
    pub runs: Vec<LabelRun>,
}

impl Label {
    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let mut x = margin + (origin.0 + self.x) * scale;
        let y = margin + (origin.1 + self.y) * scale;
        for run in &self.runs {
            let style = text_style(theme, self.slot, self.size, run.color, scale);
            x += render.text(&run.text, &style, x, y);
        }
    }
}

/// **文本块**：多行文本，行距可调，整块一个颜色（设置页的 TextBlock 那种）。
#[derive(Debug, Clone)]
pub struct TextBlock {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub slot: FontSlot,
    /// 行与行之间额外留多少（点，可为负）。
    pub line_spacing: f32,
    pub color: Color,
    pub text: String,
}

impl TextBlock {
    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let style = text_style(theme, self.slot, self.size, self.color, scale);
        let x = margin + (origin.0 + self.x) * scale;
        let mut y = margin + (origin.1 + self.y) * scale;
        let line_height = FontSpec::with_size(self.size).line_height;
        let spacing = self.line_spacing.max(-line_height + 1.0);
        let step = (line_height + spacing) * scale;
        for line in self.text.split('\n') {
            render.text(line.trim_end_matches('\r'), &style, x, y);
            y += step;
        }
    }
}

/// **静态形状**：直线 / 矩形 / 圆角矩形 / 圆 / 任意闭合路径。
#[derive(Debug, Clone)]
pub enum StaticShape {
    Line {
        from: (f32, f32),
        to: (f32, f32),
        thickness: f32,
        color: Color,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
    },
    RoundRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: Color,
    },
    Circle {
        cx: f32,
        cy: f32,
        radius: f32,
        color: Color,
    },
    Path {
        commands: Vec<PathCommand>,
        color: Color,
    },
}

impl StaticShape {
    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let x = |value: f32| margin + (origin.0 + value) * scale;
        let y = |value: f32| margin + (origin.1 + value) * scale;
        let len = |value: f32| value * scale;
        match self {
            Self::Line {
                from,
                to,
                thickness,
                color,
            } => render.line(
                (x(from.0), y(from.1)),
                (x(to.0), y(to.1)),
                len(*thickness),
                *color,
            ),
            Self::Rect {
                x: left,
                y: top,
                width,
                height,
                color,
            } => render.rect(x(*left), y(*top), len(*width), len(*height), *color),
            Self::RoundRect {
                x: left,
                y: top,
                width,
                height,
                radius,
                color,
            } => render.round_rect(
                x(*left),
                y(*top),
                len(*width),
                len(*height),
                len(*radius),
                *color,
            ),
            Self::Circle {
                cx,
                cy,
                radius,
                color,
            } => render.circle(x(*cx), y(*cy), len(*radius), *color),
            Self::Path { commands, color } => {
                if let Some(path) = build_path(commands, scale, origin) {
                    render.path(&path, *color);
                }
            }
        }
    }
}

/// **静态图像**：已经是内存位图（文件 / 固定句柄在脚本侧解析成 `Arc<Pixmap>`）。
#[derive(Clone)]
pub struct StaticImage {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub image: Arc<Pixmap>,
}

impl StaticImage {
    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        render.image(
            &self.image,
            margin + (origin.0 + self.x) * scale,
            margin + (origin.1 + self.y) * scale,
            self.width * scale,
            self.height * scale,
        );
    }
}

/// **面板**：容器 + 边框；缺省**无填充**，边缘 `#FFE0E0E0`、线宽 1 点。子控件用面板内坐标，绘制时整体平移。
#[derive(Clone)]
pub struct Panel {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    /// 边缘颜色。
    pub color: Color,
    /// 边缘线宽（点）。
    pub border_width: f32,
    /// 内部填充；`None` = 不填充。
    pub fill: Option<Color>,
    pub children: Vec<crate::document::Element>,
}

/// 控件文字样式：与 `draw_element` 的 `text` 元素同一套（字族按槽位取主题）。
fn text_style(theme: &Theme, slot: FontSlot, size: f32, color: Color, scale: f32) -> TextStyle {
    let font = FontSpec::with_size(size).scaled(scale);
    TextStyle::new(font, size, color, theme.text_gamma).with_family(family_for(theme, slot))
}

/// 控件的路径命令 → tiny-skia 路径（控件自己的原点是 `origin`，单位点）。
fn build_path(commands: &[PathCommand], scale: f32, origin: (f32, f32)) -> Option<Path> {
    let ox = origin.0 * scale;
    let oy = origin.1 * scale;
    let mut builder = PathBuilder::new();
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => builder.move_to(ox + x * scale, oy + y * scale),
            PathCommand::LineTo(x, y) => builder.line_to(ox + x * scale, oy + y * scale),
            PathCommand::QuadTo(cx, cy, x, y) => builder.quad_to(
                ox + cx * scale,
                oy + cy * scale,
                ox + x * scale,
                oy + y * scale,
            ),
            PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => builder.cubic_to(
                ox + cx1 * scale,
                oy + cy1 * scale,
                ox + cx2 * scale,
                oy + cy2 * scale,
                ox + x * scale,
                oy + y * scale,
            ),
            PathCommand::Close => builder.close(),
        }
    }
    builder.finish()
}
