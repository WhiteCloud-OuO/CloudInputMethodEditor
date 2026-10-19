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

/// 缺省滚动条轨道颜色（独立滚动条和 `text_box` 内部滚动条共用）。
const DEFAULT_SCROLLBAR_BACKGROUND: Color = Color::rgb(0xC8, 0xC8, 0xC8);

/// 缺省滚动条滑块颜色。
const DEFAULT_SCROLLBAR_THUMB: Color = Color::rgb(0x88, 0x88, 0x88);

/// 缺省编辑框边框颜色。
const DEFAULT_EDIT_BORDER: Color = Color::rgb(0xC8, 0xC8, 0xC8);

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
    /// 更新静态标签的文字；每个 run 自带颜色，调用方必须给出每段颜色。
    pub fn update_content(&mut self, runs: Vec<LabelRun>) {
        self.runs = runs;
    }

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
    /// 更新静态多行文本内容；不参与命中或编辑。
    pub fn update_content(&mut self, text: &str) {
        self.text = text.to_owned();
    }

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

/// **滚动条**：整条背景胶囊 + 可拖动的滑块胶囊；区域就是背景形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone)]
pub struct ScrollBar {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// 背景颜色；`None` = 缺省 `#FFC8C8C8`。
    pub background: Option<Color>,
    /// 滑块颜色；`None` = 缺省 `#FF888888`。
    pub thumb: Option<Color>,
    pub axis: ScrollAxis,
    pub value: u64,
    pub min: u64,
    pub max: u64,
    /// 滚轮 / 点击轨道时每次改变的步长（至少 1）。
    pub step: u64,
    /// 自动区域名（同一份内容里唯一）。
    pub region_name: String,
}

impl ScrollBar {
    fn track_length(&self) -> f32 {
        match self.axis {
            ScrollAxis::Horizontal => self.width,
            ScrollAxis::Vertical => self.height,
        }
    }

    fn thickness(&self) -> f32 {
        match self.axis {
            ScrollAxis::Horizontal => self.height,
            ScrollAxis::Vertical => self.width,
        }
    }

    /// 滑块长度：轨道 1/4，至少一个厚度，最长整条轨道。
    fn thumb_length(&self) -> f32 {
        let track = self.track_length().max(1.0);
        (track * 0.25).max(self.thickness()).min(track)
    }

    fn ratio(&self) -> f32 {
        if self.max <= self.min {
            0.0
        } else {
            ((self.value - self.min) as f64 / (self.max - self.min) as f64) as f32
        }
    }

    /// 滑块在内容区里的矩形（点）。
    pub fn thumb_rect(&self) -> (f32, f32, f32, f32) {
        let track = self.track_length();
        let thumb = self.thumb_length();
        let travel = (track - thumb).max(0.0);
        let start = travel * self.ratio().clamp(0.0, 1.0);
        match self.axis {
            ScrollAxis::Horizontal => (self.x + start, self.y, thumb, self.height),
            ScrollAxis::Vertical => (self.x, self.y + start, self.width, thumb),
        }
    }

    /// 指针（内容坐标）对应到某个值；`offset` 是按下点相对滑块起点的偏移。
    pub fn value_at(&self, point: (f32, f32), offset: f32) -> u64 {
        let track = self.track_length();
        let thumb = self.thumb_length();
        let travel = (track - thumb).max(0.0);
        if travel <= 0.0 || self.max <= self.min {
            return self.min;
        }
        let start = match self.axis {
            ScrollAxis::Horizontal => self.x,
            ScrollAxis::Vertical => self.y,
        };
        let position = match self.axis {
            ScrollAxis::Horizontal => point.0,
            ScrollAxis::Vertical => point.1,
        };
        let ratio = ((position - start - offset) / travel).clamp(0.0, 1.0);
        self.min
            + ((ratio as f64 * (self.max - self.min) as f64).round() as u64)
                .min(self.max - self.min)
    }

    pub fn hit_shape(&self, origin: (f32, f32)) -> HitShape {
        HitShape::RoundRect {
            x: origin.0 + self.x,
            y: origin.1 + self.y,
            width: self.width,
            height: self.height,
            radius: self.thickness() * 0.5,
        }
    }

    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        _theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let bg = self.background.unwrap_or(DEFAULT_SCROLLBAR_BACKGROUND);
        let thumb = self.thumb.unwrap_or(DEFAULT_SCROLLBAR_THUMB);
        let x = |value: f32| margin + (origin.0 + value) * scale;
        let y = |value: f32| margin + (origin.1 + value) * scale;
        let len = |value: f32| value * scale;
        render.round_rect(
            x(self.x),
            y(self.y),
            len(self.width),
            len(self.height),
            len(self.thickness() * 0.5),
            bg,
        );
        let (tx, ty, tw, th) = self.thumb_rect();
        render.round_rect(
            x(tx),
            y(ty),
            len(tw),
            len(th),
            len(self.thickness() * 0.5),
            thumb,
        );
    }
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

/// 文本编辑框里 SVG 图案靠哪一边。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgAlign {
    Left,
    Right,
}

/// `text_box` 内部滚动条的可见范围（值单位为 RichEdit 的像素）。
/// `max` 是**可滚到的最大位置**（内容总高度 / 宽度减一页），`page` 是视口大小。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextBoxScroll {
    pub min: u64,
    pub max: u64,
    pub value: u64,
    pub page: u64,
    pub visible: bool,
}

impl TextBoxScroll {
    fn ratio(&self) -> f32 {
        let span = self.max.saturating_sub(self.min);
        if span == 0 {
            0.0
        } else {
            ((self.value.saturating_sub(self.min)) as f64 / span as f64) as f32
        }
    }

    /// 滑块长度按可见页占整段内容的比例算，至少一个滚动条厚度。
    fn thumb_length(&self, track: f32, thickness: f32) -> f32 {
        if track <= 0.0 {
            return 0.0;
        }
        let span = self.max.saturating_sub(self.min) as f64;
        let page = self.page.max(1) as f64;
        let length = track as f64 * page / (span + page).max(1.0);
        (length as f32).max(thickness).min(track)
    }
}

/// **多行文本编辑框**：背景圆角矩形 + 文本区 + 内部控制条；文本本身由 Windowless RichEdit 绘制。
#[derive(Clone)]
pub struct TextBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub fill: Option<Color>,
    /// 边框颜色；`None` = 缺省 `#FFC8C8C8`。
    pub border: Option<Color>,
    pub border_width: f32,
    /// 缺省边框按 1 物理像素画，不随缩放；显式 `edge_width` 仍按点缩放。
    pub border_width_fixed: bool,
    pub selection_background: Option<Color>,
    pub selection_color: Option<Color>,
    pub caret_color: Option<Color>,
    pub text_color: Option<Color>,
    pub read_only: bool,
    /// 自动换行；启用时没有水平滚动条。
    pub auto_line_wrap: bool,
    /// 行距（像素）。
    pub line_spacing: u16,
    pub font_name: Option<String>,
    pub font_flag: u8,
    pub handlers: Vec<(String, String)>,
    pub text: String,
    pub font: FontSlot,
    pub size: Option<f32>,
    pub region_name: String,
    /// 运行时滚动状态；由宿主查 RichEdit 后写回。
    pub h_scroll: TextBoxScroll,
    pub v_scroll: TextBoxScroll,
}

impl TextBox {
    const SCROLLBAR_THICKNESS: f32 = 8.0;
    const SCROLLBAR_GAP: f32 = 4.0;

    /// 文本区（相对控件左上角）；滚动条出现时自动让出右侧 / 底部。
    pub fn edit_rect(&self) -> (f32, f32, f32, f32) {
        let inset = self.border_width.max(0.0);
        let padding = 4.0;
        let right = self.width
            - inset
            - padding
            - if self.v_scroll.visible {
                Self::SCROLLBAR_THICKNESS + Self::SCROLLBAR_GAP
            } else {
                0.0
            };
        let bottom = self.height
            - inset
            - padding
            - if self.h_scroll.visible && !self.auto_line_wrap {
                Self::SCROLLBAR_THICKNESS + Self::SCROLLBAR_GAP
            } else {
                0.0
            };
        (
            inset + padding,
            inset + padding,
            (right - inset - padding).max(1.0),
            (bottom - inset - padding).max(1.0),
        )
    }

    /// 滚动条在本控件内的矩形。
    pub fn bar_rect(&self, axis: ScrollAxis) -> (f32, f32, f32, f32) {
        let inset = self.border_width.max(0.0) + 2.0;
        match axis {
            ScrollAxis::Vertical => {
                let x = self.width - inset - Self::SCROLLBAR_THICKNESS;
                let extra = if self.h_scroll.visible && !self.auto_line_wrap {
                    Self::SCROLLBAR_THICKNESS + Self::SCROLLBAR_GAP
                } else {
                    0.0
                };
                let y = inset + Self::SCROLLBAR_GAP;
                (
                    x,
                    y,
                    Self::SCROLLBAR_THICKNESS,
                    (self.height - y - inset - extra).max(1.0),
                )
            }
            ScrollAxis::Horizontal => {
                let y = self.height - inset - Self::SCROLLBAR_THICKNESS;
                let extra = if self.v_scroll.visible {
                    Self::SCROLLBAR_THICKNESS + Self::SCROLLBAR_GAP
                } else {
                    0.0
                };
                let x = inset + Self::SCROLLBAR_GAP;
                (
                    x,
                    y,
                    (self.width - x - inset - extra).max(1.0),
                    Self::SCROLLBAR_THICKNESS,
                )
            }
        }
    }

    pub fn scroll(&self, axis: ScrollAxis) -> TextBoxScroll {
        match axis {
            ScrollAxis::Horizontal => self.h_scroll,
            ScrollAxis::Vertical => self.v_scroll,
        }
    }

    /// 滑块矩形；`axis` 的滚动条不可见时返回空矩形。
    pub fn thumb_rect(&self, axis: ScrollAxis) -> (f32, f32, f32, f32) {
        let (bx, by, bw, bh) = self.bar_rect(axis);
        let scroll = self.scroll(axis);
        if !scroll.visible {
            return (bx, by, 0.0, 0.0);
        }
        match axis {
            ScrollAxis::Horizontal => {
                let length = scroll.thumb_length(bw, bh);
                let travel = (bw - length).max(0.0);
                (bx + travel * scroll.ratio(), by, length, bh)
            }
            ScrollAxis::Vertical => {
                let length = scroll.thumb_length(bh, bw);
                let travel = (bh - length).max(0.0);
                (bx, by + travel * scroll.ratio(), bw, length)
            }
        }
    }

    pub fn hit_shape(&self, origin: (f32, f32)) -> HitShape {
        HitShape::RoundRect {
            x: origin.0 + self.x,
            y: origin.1 + self.y,
            width: self.width,
            height: self.height,
            radius: self.radius,
        }
    }

    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        _theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let x = |value: f32| margin + (origin.0 + value) * scale;
        let y = |value: f32| margin + (origin.1 + value) * scale;
        let len = |value: f32| value * scale;
        if let Some(fill) = self.fill {
            render.round_rect(
                x(self.x),
                y(self.y),
                len(self.width),
                len(self.height),
                len(self.radius),
                fill,
            );
        }
        let border = self.border.unwrap_or(DEFAULT_EDIT_BORDER);
        if self.border_width > 0.0
            && let Some(rect) =
                tiny_skia::Rect::from_xywh(x(self.x), y(self.y), len(self.width), len(self.height))
        {
            let stroke = if self.border_width_fixed {
                1.0
            } else {
                len(self.border_width)
            };
            render.stroke_round_rect(rect, len(self.radius), stroke, border);
        }
        let background = DEFAULT_SCROLLBAR_BACKGROUND;
        let thumb = DEFAULT_SCROLLBAR_THUMB;
        for axis in [ScrollAxis::Horizontal, ScrollAxis::Vertical] {
            let scroll = self.scroll(axis);
            if !scroll.visible {
                continue;
            }
            let (bx, by, bw, bh) = self.bar_rect(axis);
            let radius = Self::SCROLLBAR_THICKNESS * 0.5;
            render.round_rect(
                x(self.x + bx),
                y(self.y + by),
                len(bw),
                len(bh),
                len(radius),
                background,
            );
            let (tx, ty, tw, th) = self.thumb_rect(axis);
            if tw > 0.0 && th > 0.0 {
                render.round_rect(
                    x(self.x + tx),
                    y(self.y + ty),
                    len(tw),
                    len(th),
                    len(radius),
                    thumb,
                );
            }
        }
    }
}

/// **单行文本编辑框**：背景胶囊 / 边框 + 可选 SVG + Windowless RichEdit 承载的文本区。
#[derive(Clone)]
pub struct TextEdit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    /// 内部填充；`None` = 透明。
    pub fill: Option<Color>,
    /// 边框颜色；`None` = 缺省 #FFC8C8C8。
    pub border: Option<Color>,
    pub border_width: f32,
    /// 缺省边框按 1 物理像素画，不随缩放；显式 `edge_width` 仍按点缩放。
    pub border_width_fixed: bool,
    pub svg: Option<String>,
    /// SVG 画板尺寸（点）。
    pub svg_size: (f32, f32),
    pub svg_align: SvgAlign,
    /// SVG 染色；`None` 保留 SVG 自己的颜色。
    pub svg_tint: Option<Color>,
    /// 选中文字的背景；`None` = 主题高亮色。
    pub selection_background: Option<Color>,
    /// 选中文字的颜色；`None` = 主题高亮文字色。
    pub selection_color: Option<Color>,
    /// 插入符颜色；`None` = 正文颜色。
    pub caret_color: Option<Color>,
    /// 正文颜色；`None` = 主题正文色。
    pub text_color: Option<Color>,
    /// 只读。
    pub read_only: bool,
    /// 密码输入：文字按 `*` 显示，真实文本仍由 API 读写。
    pub passkey: bool,
    /// 字体族覆盖；`None` = 字体槽位的主题字体。
    pub font_name: Option<String>,
    /// 字体标志：1 加粗 / 2 斜体 / 4 下划线 / 8 删除线，可相加。
    pub font_flag: u8,
    /// 事件处理器表（`events`）：动作名 → 处理器名，挂在自动区域上。
    pub handlers: Vec<(String, String)>,
    /// 初始文本（真正输入由 Windowless RichEdit 负责）。
    pub text: String,
    /// 字体槽位；缺省候选词字体。
    pub font: FontSlot,
    /// 字号（点）；`None` = 用主题里该槽位的字号。
    pub size: Option<f32>,
    /// 自动区域名。
    pub region_name: String,
}

impl TextEdit {
    /// 文本区（相对按钮左上角）：没有 SVG 时居中；有 SVG 时放在图标旁边。
    pub fn edit_rect(&self) -> (f32, f32, f32, f32) {
        // 让 RichEdit 落在边框里面，别盖住 1 点边框 / 溢出胶囊。
        let inset = self.border_width.max(0.0);
        let height = (self.height - inset * 2.0).max(1.0);
        if self.svg.is_none() {
            return (
                self.height * 0.5,
                inset,
                (self.width - self.height).max(0.0),
                height,
            );
        }
        let svg_width = self.svg_width();
        let gutter = self.height * 0.25;
        let width = (self.width - self.height - svg_width - 4.0).max(0.0);
        match self.svg_align {
            SvgAlign::Left => (gutter + svg_width, inset, width, height),
            SvgAlign::Right => (gutter, inset, width, height),
        }
    }

    /// SVG 按背景高度缩放后的宽度。
    pub fn svg_width(&self) -> f32 {
        if self.svg_size.1 <= 0.0 {
            return self.height;
        }
        self.height * self.svg_size.0 / self.svg_size.1
    }

    pub fn hit_shape(&self, origin: (f32, f32)) -> HitShape {
        HitShape::RoundRect {
            x: origin.0 + self.x,
            y: origin.1 + self.y,
            width: self.width,
            height: self.height,
            radius: self.radius,
        }
    }

    pub(crate) fn draw<R: ControlRender>(
        &self,
        render: &mut R,
        _theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let x = margin + (origin.0 + self.x) * scale;
        let y = margin + (origin.1 + self.y) * scale;
        let width = self.width * scale;
        let height = self.height * scale;
        if let Some(fill) = self.fill {
            render.round_rect(x, y, width, height, self.radius * scale, fill);
        }
        let border = self.border.unwrap_or(DEFAULT_EDIT_BORDER);
        if self.border_width > 0.0
            && let Some(rect) = tiny_skia::Rect::from_xywh(x, y, width, height)
        {
            let stroke = if self.border_width_fixed {
                1.0
            } else {
                self.border_width * scale
            };
            render.stroke_round_rect(rect, self.radius * scale, stroke, border);
        }
        if let Some(svg) = &self.svg {
            let gutter = self.height * 0.25;
            let svg_width = self.svg_width();
            let svg_x = match self.svg_align {
                SvgAlign::Left => gutter,
                SvgAlign::Right => self.width - gutter - svg_width,
            };
            let _ = render.svg(
                svg,
                margin + (origin.0 + self.x + svg_x) * scale,
                margin + (origin.1 + self.y) * scale,
                height,
                self.svg_tint,
            );
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺省滚动条 / 编辑框边框颜色是界面规范里的固定值，不跟主题走。
    #[test]
    fn default_control_colors_are_fixed() {
        assert_eq!(DEFAULT_SCROLLBAR_BACKGROUND, Color::rgb(0xC8, 0xC8, 0xC8));
        assert_eq!(DEFAULT_SCROLLBAR_THUMB, Color::rgb(0x88, 0x88, 0x88));
        assert_eq!(DEFAULT_EDIT_BORDER, Color::rgb(0xC8, 0xC8, 0xC8));
    }
}
