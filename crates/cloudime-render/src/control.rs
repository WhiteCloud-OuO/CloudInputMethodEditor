//! 静态控件：用 [`crate::elements`] 的绘制 traits 组合出来的可复用显示件。
//!
//! 静态控件**没有命中框、不接受交互**：窗口真正能点的还是 `regions` / 元素。面板只负责
//! 分组与背景；子控件用面板内的相对坐标摆放，绘制时整体平移。
//!
//! 坐标与尺寸一律是**点**；绘制时按 `scale` 换成像素。

use std::sync::Arc;

use tiny_skia::{Path, PathBuilder, Pixmap};

use crate::color::Color;
use crate::document::{FontSlot, HitRect, HitShape, PathCommand, family_for};
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

/// 扁平 SVG 按钮的鼠标状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
}

/// 按钮某一状态的两种颜色。
#[derive(Debug, Clone, Copy)]
pub struct ButtonColors {
    pub background: Color,
    pub svg: Color,
}

/// 按钮的背景形状（也是区域的形状）。
#[derive(Debug, Clone)]
pub enum ButtonShape {
    Rect,
    RoundRect {
        radius: f32,
    },
    Circle {
        radius: f32,
    },
    /// 路径命令用按钮内坐标（相对按钮左上角）。
    Path {
        commands: Vec<PathCommand>,
    },
}

/// **扁平 SVG 按钮**：背景形状 + 区域 + SVG 图形；状态由窗口根据鼠标自动切换。
#[derive(Clone)]
pub struct FlatSvgButton {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub shape: ButtonShape,
    pub svg: String,
    /// SVG 画板尺寸（点）；没写背景尺寸时用它。
    pub svg_size: (f32, f32),
    pub normal: ButtonColors,
    pub hovered: ButtonColors,
    pub pressed: ButtonColors,
    pub state: ButtonState,
    /// 自动区域的名字（同一份内容里唯一）。
    pub region_name: String,
}

impl FlatSvgButton {
    pub(crate) fn colors(&self) -> ButtonColors {
        match self.state {
            ButtonState::Normal => self.normal,
            ButtonState::Hovered => self.hovered,
            ButtonState::Pressed => self.pressed,
        }
    }

    /// 背景形状在内容区里的命中区域（区域与背景完全一致）。
    pub fn hit_shape(&self, origin: (f32, f32)) -> HitShape {
        let x = origin.0 + self.x;
        let y = origin.1 + self.y;
        match &self.shape {
            ButtonShape::Rect => HitShape::Rect(HitRect::new(x, y, self.width, self.height)),
            ButtonShape::RoundRect { radius } => HitShape::RoundRect {
                x,
                y,
                width: self.width,
                height: self.height,
                radius: *radius,
            },
            ButtonShape::Circle { radius } => HitShape::Circle {
                cx: x + self.width * 0.5,
                cy: y + self.height * 0.5,
                radius: *radius,
            },
            ButtonShape::Path { commands } => {
                let commands = translate_commands(commands, x, y);
                HitShape::from_path(&commands).unwrap_or(HitShape::Rect(HitRect::new(
                    x,
                    y,
                    self.width,
                    self.height,
                )))
            }
        }
    }

    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let colors = self.colors();
        let x = |value: f32| margin + (origin.0 + value) * scale;
        let y = |value: f32| margin + (origin.1 + value) * scale;
        let len = |value: f32| value * scale;
        match &self.shape {
            ButtonShape::Rect => render.rect(
                x(self.x),
                y(self.y),
                len(self.width),
                len(self.height),
                colors.background,
            ),
            ButtonShape::RoundRect { radius } => render.round_rect(
                x(self.x),
                y(self.y),
                len(self.width),
                len(self.height),
                len(*radius),
                colors.background,
            ),
            ButtonShape::Circle { radius } => render.circle(
                x(self.x + self.width * 0.5),
                y(self.y + self.height * 0.5),
                len(*radius),
                colors.background,
            ),
            ButtonShape::Path { commands } => {
                if let Some(path) =
                    build_path(commands, scale, (origin.0 + self.x, origin.1 + self.y))
                {
                    render.path(&path, colors.background);
                }
            }
        }
        let size = self.width.min(self.height);
        if size > 0.0 {
            let icon_x = x(self.x + (self.width - size) * 0.5);
            let icon_y = y(self.y + (self.height - size) * 0.5);
            let _ = render.svg(&self.svg, icon_x, icon_y, len(size), Some(colors.svg));
        }
    }
}

/// 把按钮内坐标的路径命令平移到内容区坐标。
fn translate_commands(commands: &[PathCommand], dx: f32, dy: f32) -> Vec<PathCommand> {
    commands
        .iter()
        .map(|command| match *command {
            PathCommand::MoveTo(x, y) => PathCommand::MoveTo(x + dx, y + dy),
            PathCommand::LineTo(x, y) => PathCommand::LineTo(x + dx, y + dy),
            PathCommand::QuadTo(cx, cy, x, y) => {
                PathCommand::QuadTo(cx + dx, cy + dy, x + dx, y + dy)
            }
            PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                PathCommand::CubicTo(cx1 + dx, cy1 + dy, cx2 + dx, cy2 + dy, x + dx, y + dy)
            }
            PathCommand::Close => PathCommand::Close,
        })
        .collect()
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
