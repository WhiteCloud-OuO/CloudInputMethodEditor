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
    Disabled,
}

/// 按钮某一状态的两种颜色。
#[derive(Debug, Clone, Copy)]
pub struct ButtonColors {
    pub background: Color,
    pub svg: Color,
}

/// 文本按钮里的一段文本样式。
#[derive(Debug, Clone)]
pub struct ButtonText {
    pub text: String,
    pub color: Color,
    /// `None` = 主题正文 / 候选字体。
    pub family: Option<String>,
    /// 字号（点）。
    pub size: f32,
    /// 字重（100–900）。
    pub weight: u16,
    pub italic: bool,
}

/// 文本按钮自己的阴影；模糊半径沿用浮层阴影。
#[derive(Debug, Clone, Copy)]
pub struct ButtonShadow {
    pub color: Color,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// 页面默认键绑定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultButton {
    Escape,
    Enter,
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
    /// 禁用状态的背景 / 文本颜色。
    pub disabled: ButtonColors,
    pub state: ButtonState,
    /// 是否可用；`false` 时不响应鼠标 / 键盘，绘制 disabled 颜色。
    pub enabled: bool,
    /// 文本按钮的内容；`None` = 普通扁平 SVG 按钮。
    pub text: Option<ButtonText>,
    /// 文本按钮自己的阴影；`None` = 无阴影。
    pub shadow: Option<ButtonShadow>,
    /// 只在为 `true` 时绘制阴影；缺省 `false`。
    pub show_shadow: bool,
    /// 页面默认键（同一个页面最多一个按钮绑定）。
    pub default_button: Option<DefaultButton>,
    /// 自动区域的名字（同一份内容里唯一）。
    pub region_name: String,
}

impl FlatSvgButton {
    pub(crate) fn colors(&self) -> ButtonColors {
        if !self.enabled {
            return self.disabled;
        }
        match self.state {
            ButtonState::Normal => self.normal,
            ButtonState::Hovered => self.hovered,
            ButtonState::Pressed => self.pressed,
            ButtonState::Disabled => self.disabled,
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
        theme: &Theme,
        scale: f32,
        origin: (f32, f32),
        margin: f32,
    ) {
        let colors = self.colors();
        let x = |value: f32| margin + (origin.0 + value) * scale;
        let y = |value: f32| margin + (origin.1 + value) * scale;
        let len = |value: f32| value * scale;
        if self.show_shadow
            && let Some(shadow) = &self.shadow
            && let Some(content) =
                tiny_skia::Rect::from_xywh(x(self.x), y(self.y), len(self.width), len(self.height))
        {
            let shadow = crate::Shadow {
                blur: 12.0,
                offset_x: shadow.offset_x,
                offset_y: shadow.offset_y,
                color: shadow.color,
            };
            render.shadow(&shadow, content, len(self.shadow_radius()), scale);
        }
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
        if self.text.is_some() {
            let layout = ButtonTextBox {
                x: x(self.x),
                y: y(self.y),
                width: len(self.width),
                height: len(self.height),
                scale,
            };
            self.draw_text(render, theme, colors.svg, layout);
        } else {
            let size = self.width.min(self.height);
            if !self.svg.is_empty() && size > 0.0 {
                let icon_x = x(self.x + (self.width - size) * 0.5);
                let icon_y = y(self.y + (self.height - size) * 0.5);
                let _ = render.svg(&self.svg, icon_x, icon_y, len(size), Some(colors.svg));
            }
        }
    }

    fn shadow_radius(&self) -> f32 {
        match &self.shape {
            ButtonShape::Rect => 0.0,
            ButtonShape::RoundRect { radius } => *radius,
            ButtonShape::Circle { radius } => *radius,
            ButtonShape::Path { .. } => 0.0,
        }
    }

    fn draw_text<R: ControlRender>(
        &self,
        render: &mut R,
        theme: &Theme,
        color: Color,
        layout: ButtonTextBox,
    ) {
        let Some(text) = &self.text else {
            return;
        };
        let style = button_text_style(theme, text, color, layout.scale);
        let line_height = FontSpec::with_size(text.size).line_height * layout.scale;
        let lines: Vec<&str> = text.text.split('\n').collect();
        let block_height = line_height * lines.len() as f32;
        let mut line_y = layout.y + (layout.height - block_height) * 0.5;
        for line in lines {
            let width = render.measure(line, &style).width;
            render.text(
                line,
                &style,
                layout.x + (layout.width - width) * 0.5,
                line_y,
            );
            line_y += line_height;
        }
    }
}

/// 文本按钮在画布里的像素矩形 + 当前缩放。
#[derive(Clone, Copy)]
struct ButtonTextBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: f32,
}

/// 列表盒的一行内容。元素坐标相对行左上角。
#[derive(Clone, Debug)]
pub struct ListBoxRow {
    /// 逻辑行号（0 起）；虚拟化时不一定等于 `rows` 的下标。
    pub index: usize,
    /// 相对内容区顶部的偏移（点）。
    pub top: f32,
    pub height: f32,
    /// 行内容自然宽度；`0` 表示铺满内容区。
    pub width: f32,
    /// 稳定 key（数据 `key` 字段）；没有 key 时由页面树给实例名。
    pub key: Option<String>,
    pub elements: Vec<crate::document::Element>,
    /// 选中态模板展开结果；空时回退到 `elements`。
    pub selected_elements: Vec<crate::document::Element>,
    /// 悬停态模板展开结果；空时回退到 `elements`。
    pub hovered_elements: Vec<crate::document::Element>,
    /// 行内可交互控件的矩形与事件处理器。
    pub hits: Vec<ListBoxHit>,
}

/// 虚拟化列表的一次可见窗口。
#[derive(Clone, Debug)]
pub struct ListBoxWindow {
    pub rows: Vec<ListBoxRow>,
    pub row_count: usize,
    pub content_height: f32,
    pub scroll_offset: f32,
}

impl ListBoxRow {
    pub fn active_elements(&self, selected: bool, hovered: bool) -> &[crate::document::Element] {
        if selected && !self.selected_elements.is_empty() {
            &self.selected_elements
        } else if hovered && !self.hovered_elements.is_empty() {
            &self.hovered_elements
        } else {
            &self.elements
        }
    }
}

/// 列表盒行内的一个可交互控件。
#[derive(Clone, Debug)]
pub struct ListBoxHit {
    pub shape: HitShape,
    pub control: String,
    pub handlers: Vec<(String, String)>,
    /// 在 `ListBoxRow::elements` 里的下标；按钮状态切换要用。
    pub element: usize,
}

/// `list_box` 行宽小于内容区时的水平对齐。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListBoxAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// **单列列表盒**：背景 + 内部行内容 + 内部垂直滚动条。
///
/// 行的布局、子页面实例和选择状态由上层页面树负责；这里只保存已经展开的行和滚动参数。
#[derive(Clone)]
pub struct ListBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub fill: Option<Color>,
    pub edge_color: Color,
    pub edge_width: f32,
    /// 边框内侧到内容区的间隙（点）。
    pub content_gap: f32,
    pub gap: f32,
    /// 绘制时把内容区剩余高度平均到可见项间隙里，缺省 `true`。
    pub auto_item_gap_averaging: bool,
    pub align: ListBoxAlign,
    pub rows: Vec<ListBoxRow>,
    /// 逻辑行总数；虚拟化时可能大于 `rows.len()`。
    pub row_count: usize,
    /// 滚动偏移（点）。
    pub scroll_offset: f32,
    /// 逻辑内容总高度（点）。
    pub content_height: f32,
    pub allow_select: bool,
    pub selected_index: i32,
    /// 选中项变化时把它垂直居中；缺省 `false`（只保证滚进可见区）。
    pub auto_center_selected: bool,
    /// 当前悬停行（0 起）；没有悬停为 `None`。
    pub hovered_index: Option<i32>,
    /// 自动区域名（同一份内容里唯一）。
    pub region_name: String,
}

impl ListBox {
    pub fn viewport_height(&self) -> f32 {
        (self.height - self.edge_width * 2.0 - self.content_gap * 2.0).max(0.0)
    }

    pub fn content_width(&self, scrollbar: bool) -> f32 {
        (self.width
            - self.edge_width * 2.0
            - self.content_gap * 2.0
            - if scrollbar { 8.0 + 4.0 } else { 0.0 })
        .max(0.0)
    }

    pub fn scrollbar_visible(&self) -> bool {
        self.content_height > self.viewport_height() + 0.01
    }

    /// 滚动条轨道（相对控件左上角）：右侧内缩 2、宽 8、上下贴边框内沿。
    pub fn track_rect(&self) -> (f32, f32, f32, f32) {
        // 轨道和内容区使用同一对上下内缩；否则滑块会顶到圆角边框，
        // 和列表内容之间看起来多出一截。
        let top = self.edge_width + self.content_gap;
        (
            self.width - self.edge_width - 2.0 - 8.0,
            top,
            8.0,
            self.viewport_height().max(1.0),
        )
    }

    pub fn thumb_rect(&self) -> (f32, f32, f32, f32) {
        let (tx, ty, tw, th) = self.track_rect();
        let content = self.content_height;
        let view = self.viewport_height();
        if content <= view || content <= 0.0 {
            return (tx, ty, tw, 0.0);
        }
        let length = (th * view / content).max(1.0).min(th);
        let travel = (th - length).max(0.0);
        let max_scroll = (content - view).max(0.0);
        let offset = self.scroll_offset.clamp(0.0, max_scroll);
        let ratio = if max_scroll <= 0.0 {
            0.0
        } else {
            offset / max_scroll
        };
        (tx, ty + travel * ratio, tw, length)
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

    pub fn max_scroll_px(&self) -> f32 {
        (self.content_height - self.viewport_height()).max(0.0)
    }

    pub fn clamp_scroll(&self) -> f32 {
        self.scroll_offset.clamp(0.0, self.max_scroll_px())
    }

    /// 绘制和命中使用的滚动偏移：中间位置吸附到行首，尽量让上下边缘都落在整行上。
    ///
    /// 贴底时保留原始偏移，让最后一行完整贴在内容区下沿；此时顶部可能有一行被裁掉，
    /// 由 [`Self::row_visible`] 过滤。当前窗口还没覆盖目标偏移时也先返回原值，
    /// 等 Server 的下一窗到达后再吸附。
    pub fn paint_scroll(&self) -> f32 {
        let offset = self.clamp_scroll();
        let view = self.viewport_height();
        if view <= 0.0 || offset + view >= self.content_height - 0.5 {
            return offset;
        }
        let (Some(first), Some(last)) = (self.rows.first(), self.rows.last()) else {
            return offset;
        };
        let end = last.top + last.height;
        if offset < first.top - 0.5 || offset > end + 0.5 {
            return offset;
        }
        self.rows
            .iter()
            .filter(|row| row.top <= offset + 0.5)
            .max_by(|a, b| a.top.total_cmp(&b.top))
            .map(|row| row.top)
            .unwrap_or(offset)
    }

    /// 这一行是否应该完整画出来。单行比视口还高时无法完整显示，退化为部分可见。
    pub fn row_visible(&self, row: &ListBoxRow) -> bool {
        let view = self.viewport_height();
        if view <= 0.0 {
            return false;
        }
        let top = row.top - self.paint_scroll();
        let bottom = top + row.height;
        if row.height > view + 0.5 {
            top < view - 0.5 && bottom > 0.5
        } else {
            top >= -0.5 && bottom <= view + 0.5
        }
    }

    /// 行在内容区里的绘制顶部。开启 `auto_item_gap_averaging` 且列表需要滚动时，
    /// 把剩余高度平均分给当前可见行之间的间隙；否则沿用 `gap` 的原始位置。
    pub fn row_view_top(&self, row: &ListBoxRow) -> Option<f32> {
        if !self.row_visible(row) {
            return None;
        }
        let base = row.top - self.paint_scroll();
        if !self.auto_item_gap_averaging || !self.scrollbar_visible() {
            return Some(base);
        }
        let visible: Vec<&ListBoxRow> = self
            .rows
            .iter()
            .filter(|candidate| self.row_visible(candidate))
            .collect();
        if visible.len() < 2 {
            return Some(base);
        }
        let total_height = visible
            .iter()
            .map(|candidate| candidate.height)
            .sum::<f32>();
        let view = self.viewport_height();
        if total_height >= view - 0.5 {
            return Some(base);
        }
        let averaged_gap = (view - total_height) / (visible.len() - 1) as f32;
        let mut top = 0.0;
        for candidate in visible {
            if candidate.index == row.index {
                return Some(top);
            }
            top += candidate.height + averaged_gap;
        }
        Some(base)
    }

    pub fn row(&self, index: usize) -> Option<&ListBoxRow> {
        self.rows.iter().find(|row| row.index == index)
    }

    pub fn row_mut(&mut self, index: usize) -> Option<&mut ListBoxRow> {
        self.rows.iter_mut().find(|row| row.index == index)
    }

    /// 命中哪一行（`y` 是相对 list_box 左上角的点坐标），空白返回 `None`。
    pub fn row_at(&self, y: f32) -> Option<usize> {
        let local = y - self.edge_width - self.content_gap;
        if local < 0.0 {
            return None;
        }
        self.rows.iter().find_map(|row| {
            let top = self.row_view_top(row)?;
            (local >= top && local < top + row.height).then_some(row.index)
        })
    }

    pub fn row_origin(&self, row: usize) -> Option<(f32, f32)> {
        let row_ref = self.row(row)?;
        // 隐藏行也返回原始位置：文本宿主先建好并隐藏，滚到它时直接显示，
        // 不再等下一次 SetListWindow 才补建窗口（否则底部新露出的行会空白）。
        let top = self
            .row_view_top(row_ref)
            .unwrap_or_else(|| row_ref.top - self.paint_scroll());
        let available = self.content_width(self.scrollbar_visible());
        let row_width = if row_ref.width > 0.0 {
            row_ref.width
        } else {
            available
        };
        let x = match self.align {
            ListBoxAlign::Left => 0.0,
            ListBoxAlign::Center => (available - row_width).max(0.0) * 0.5,
            ListBoxAlign::Right => (available - row_width).max(0.0),
        };
        Some((
            self.edge_width + self.content_gap + x,
            self.edge_width + self.content_gap + top,
        ))
    }

    pub fn hit_at(&self, row: usize, x: f32, y: f32) -> Option<usize> {
        let row_ref = self.row(row)?;
        let available = self.content_width(self.scrollbar_visible());
        let row_width = if row_ref.width > 0.0 {
            row_ref.width
        } else {
            available
        };
        let row_x = match self.align {
            ListBoxAlign::Left => 0.0,
            ListBoxAlign::Center => (available - row_width).max(0.0) * 0.5,
            ListBoxAlign::Right => (available - row_width).max(0.0),
        };
        let local_x = x - self.edge_width - self.content_gap - row_x;
        let local_y = y - self.edge_width - self.content_gap - self.row_view_top(row_ref)?;
        row_ref
            .hits
            .iter()
            .position(|hit| hit.shape.contains(local_x, local_y))
    }

    pub fn first_visible_row(&self) -> usize {
        let offset = self.clamp_scroll();
        if let Some(row) = self.rows.iter().find(|row| self.row_visible(row)) {
            return row.index;
        }
        // 点击轨道会直接跳到当前窗口之外的偏移；这时不能回退到“最后一行”，
        // 也不能拿当前窗口第一行凑数，否则 Server 会按错误行重建窗口：
        // 往下跳表现为跳到底部，往上跳表现为只挪到窗口边缘。
        let Some(first) = self.rows.first() else {
            return self.row_count.saturating_sub(1);
        };
        let average = self
            .rows
            .iter()
            .map(|row| row.height + self.gap)
            .sum::<f32>()
            / self.rows.len() as f32;
        if average <= 0.0 {
            return first.index;
        }
        let estimated = if offset < first.top {
            first.index as f32 - ((first.top - offset) / average).ceil()
        } else if let Some(last) = self.rows.last() {
            let end = last.top + last.height;
            if offset >= end {
                last.index as f32 + ((offset - end) / average).floor() + 1.0
            } else {
                first.index as f32 + ((offset - first.top) / average).floor()
            }
        } else {
            first.index as f32 + ((offset - first.top) / average).floor()
        };
        estimated
            .max(0.0)
            .min(self.row_count.saturating_sub(1) as f32) as usize
    }

    pub fn row_top(&self, index: usize) -> f32 {
        if let Some(row) = self.row(index) {
            return row.top;
        }
        // 只有虚拟化目标行不在当前窗口时才会走到这里；固定行高可直接估算。
        self.rows
            .first()
            .map(|row| index as f32 * (row.height + self.gap))
            .unwrap_or(0.0)
    }

    pub fn scroll_to_row(&self, index: usize) -> f32 {
        let height = self
            .row(index)
            .map(|row| row.height)
            .unwrap_or_else(|| self.rows.first().map(|row| row.height).unwrap_or(0.0));
        (self.row_top(index) + height * 0.5 - self.viewport_height() * 0.5)
            .clamp(0.0, self.max_scroll_px())
    }

    /// 只保证目标行完整可见；已经完整可见时不改变滚动位置。
    pub fn scroll_to_visible(&self, index: usize) -> f32 {
        let height = self
            .row(index)
            .map(|row| row.height)
            .unwrap_or_else(|| self.rows.first().map(|row| row.height).unwrap_or(0.0));
        let top = self.row_top(index);
        let current = self.clamp_scroll();
        let viewport = self.viewport_height();
        if top < current {
            top
        } else if top + height > current + viewport {
            (top + height - viewport).max(0.0)
        } else {
            current
        }
        .clamp(0.0, self.max_scroll_px())
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
    /// 边框内侧到内容 / 滚动条区域的间隙（点）。
    pub content_gap: f32,
    pub selection_background: Option<Color>,
    pub selection_color: Option<Color>,
    pub caret_color: Option<Color>,
    pub text_color: Option<Color>,
    pub read_only: bool,
    /// 失焦时取消文本选区，缺省 `true`。
    pub deselect_on_blur: bool,
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
        let inset = self.border_width.max(0.0) + self.content_gap.max(0.0);
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
        // 滚动条也贴内容区：上下左右先让出边框 + content_gap；两条滚动条之间
        // 再留 SCROLLBAR_GAP，避免交叉处互相盖住。
        let inset = self.border_width.max(0.0) + self.content_gap.max(0.0);
        match axis {
            ScrollAxis::Vertical => {
                let x = self.width - inset - Self::SCROLLBAR_THICKNESS;
                let extra = if self.h_scroll.visible && !self.auto_line_wrap {
                    Self::SCROLLBAR_THICKNESS + Self::SCROLLBAR_GAP
                } else {
                    0.0
                };
                let y = inset;
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
                let x = inset;
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
    /// 失焦时取消文本选区，缺省 `true`。
    pub deselect_on_blur: bool,
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

/// 文本按钮自己的字体：字族可直接给，缺省用候选字体。
fn button_text_style(theme: &Theme, text: &ButtonText, color: Color, scale: f32) -> TextStyle {
    let font = FontSpec::with_size(text.size).scaled(scale);
    let family = text
        .family
        .clone()
        .or_else(|| family_for(theme, FontSlot::Candidate).cloned());
    let style = TextStyle::new(font, text.size, color, theme.text_gamma)
        .with_family(family.as_ref())
        .with_weight(text.weight);
    if text.italic { style.italic() } else { style }
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

    fn test_row(index: usize, top: f32, height: f32) -> ListBoxRow {
        ListBoxRow {
            index,
            top,
            height,
            width: 40.0,
            key: None,
            hits: Vec::new(),
            selected_elements: Vec::new(),
            hovered_elements: Vec::new(),
            elements: Vec::new(),
        }
    }

    fn test_list(scroll_offset: f32) -> ListBox {
        ListBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
            radius: 0.0,
            fill: None,
            edge_color: Color::rgb(0, 0, 0),
            edge_width: 1.0,
            content_gap: 2.0,
            gap: 0.0,
            auto_item_gap_averaging: true,
            align: ListBoxAlign::Left,
            rows: vec![test_row(0, 0.0, 40.0), test_row(1, 40.0, 40.0)],
            row_count: 2,
            scroll_offset,
            content_height: 80.0,
            allow_select: false,
            selected_index: -1,
            auto_center_selected: false,
            hovered_index: None,
            region_name: "list".to_owned(),
        }
    }

    /// 绘制时只留下完整落在内容区里的行；贴底时优先保证最后一行完整。
    #[test]
    fn list_box_only_draws_complete_rows() {
        let mut list = test_list(10.0);
        assert_eq!(list.paint_scroll(), 0.0);
        assert!(list.row_visible(&list.rows[0]));
        assert!(!list.row_visible(&list.rows[1]));
        assert_eq!(list.first_visible_row(), 0);

        list.scroll_offset = 26.0;
        assert_eq!(list.paint_scroll(), 26.0);
        assert!(!list.row_visible(&list.rows[0]));
        assert!(list.row_visible(&list.rows[1]));
        assert_eq!(list.first_visible_row(), 1);
        assert_eq!(list.row_at(20.0), Some(1));
        assert_eq!(list.row_at(5.0), None);
    }

    /// 默认把剩余高度平均分到可见行之间，填满内容区；关闭后保留原来的 gap。
    #[test]
    fn list_box_averages_visible_item_gaps() {
        let mut list = test_list(0.0);
        list.rows = vec![
            test_row(0, 0.0, 20.0),
            test_row(1, 20.0, 20.0),
            test_row(2, 40.0, 20.0),
        ];
        list.row_count = 3;
        list.content_height = 60.0;

        assert_eq!(list.row_view_top(&list.rows[0]), Some(0.0));
        assert_eq!(list.row_view_top(&list.rows[1]), Some(34.0));
        assert_eq!(list.row_view_top(&list.rows[2]), None);
        assert_eq!(list.row_origin(1), Some((3.0, 37.0)));
        assert_eq!(list.row_at(30.0), None);
        assert_eq!(list.row_at(37.0), Some(1));

        list.auto_item_gap_averaging = false;
        assert_eq!(list.row_view_top(&list.rows[1]), Some(20.0));
        assert_eq!(list.row_origin(1), Some((3.0, 23.0)));
    }
}
