//! 渲染器：一帧 + 排布 + 主题 → 位图。排版逻辑：顶部拼音行，竖排一行一个候选、横排排成一行。
//!
//! 内部全用像素：主题里的点数进来先乘缩放倍数。文字的 y 都指行框顶边，字形在行高里垂直居中。

mod columns;
mod horizontal;
mod item;
mod matrix;
mod rendered;
mod status;
mod top_line;
mod vertical;

use crate::canvas::Canvas;
use crate::color::Color;
use crate::error::RenderError;
use crate::fonts::FontLibrary;
use crate::frame::{Frame, Row, Tone};
use crate::layout::Layout;
use crate::shadow::Shadow;
use crate::text::{TextPainter, TextSize, TextStyle};
use crate::theme::{FontSpec, Theme};

pub use rendered::Rendered;
pub use status::{RenderedStatus, StatusCell};

/// preedit 光标的宽度（点）。
const CARET_WIDTH: f32 = 1.5;

/// preedit 与右侧状态文字之间的间距（点）。
const STATUS_GAP: f32 = 16.0;

/// 横排时序号与候选词之间的间距（点）。
const INDEX_GAP: f32 = 3.0;

/// 横排时高亮底色在候选两侧多出的宽度（点）。
const HIGHLIGHT_INSET: f32 = 5.0;

/// 候选右侧来源角标与候选格右边缘之间的间距（点）。
const BADGE_GAP: f32 = 4.0;

/// 光学字号（点）：20 pt 以下系统给字体用的就是这一档（调研期在 macOS 上量的）。
const OPTICAL_SIZE: f32 = 17.0;

pub struct Renderer {
    /// 文字测绘。
    text: TextPainter,
}

/// 一次渲染期间的上下文：主题按倍数换算后的像素值。
pub(super) struct Metrics<'a> {
    pub(super) theme: &'a Theme,
    pub(super) scale: f32,
}

impl Metrics<'_> {
    pub(super) fn px(&self, points: f32) -> f32 {
        points * self.scale
    }

    pub(super) fn padding(&self) -> f32 {
        self.px(self.theme.padding)
    }

    fn row_padding(&self) -> f32 {
        self.px(self.theme.row_padding)
    }

    fn column_gap(&self) -> f32 {
        self.px(self.theme.column_gap)
    }

    pub(super) fn corner_radius(&self) -> f32 {
        self.px(self.theme.corner_radius)
    }

    pub(super) fn style(&self, font: FontSpec, color: Color) -> TextStyle {
        TextStyle::new(
            font.scaled(self.scale),
            font.size,
            color,
            self.theme.text_gamma,
        )
    }

    /// 候选窗口顶部那一行拼音用的样式。
    pub(super) fn pinyin_style(&self, color: Color) -> TextStyle {
        self.style(self.theme.pinyin_font, color)
            .with_family(self.theme.pinyin_family.as_ref())
    }

    pub(super) fn text_style(&self) -> TextStyle {
        let color = self.theme.colors.text;
        self.style(self.theme.text_font, color)
            .with_family(self.theme.text_family.as_ref())
    }

    fn annotation_style(&self, color: Color) -> TextStyle {
        self.style(self.theme.annotation_font, color)
    }

    fn index_style(&self) -> TextStyle {
        self.style(self.theme.index_font, self.theme.colors.index)
            .with_family(self.theme.index_family.as_ref())
    }

    /// 页码等页脚小字：与序号同一套字体，但保持弱化的颜色（序号改纯黑后别把页码也带亮）。
    fn footer_style(&self) -> TextStyle {
        self.style(self.theme.index_font, self.theme.colors.footer)
            .with_family(self.theme.index_family.as_ref())
    }

    /// 候选右侧来源角标：与序号同一套字体，固定浅灰。
    fn badge_style(&self) -> TextStyle {
        self.style(self.theme.index_font, self.theme.colors.badge)
            .with_family(self.theme.index_family.as_ref())
    }

    fn tone_color(&self, tone: Tone) -> Color {
        match tone {
            Tone::Gloss => self.theme.colors.gloss,
            Tone::Fresh => self.theme.colors.fresh,
            Tone::Faint => self.theme.colors.pos,
        }
    }

    /// 小字相对候选词往下挪多少，让两者底部对齐。
    fn small_offset(&self, text_height: f32) -> f32 {
        (text_height - self.px(self.theme.annotation_font.line_height)).max(0.0)
    }

    /// 序号相对候选词往下挪多少：两者**行框垂直居中**（序号字体比候选词小，居中的挪动量是差值的一半）。
    /// 以前沿用译文的底部对齐，序号看起来偏高、与候选词不在一条中线上。
    fn index_offset(&self, text_height: f32) -> f32 {
        (text_height - self.px(self.theme.index_font.line_height)).max(0.0) / 2.0
    }
}

impl Renderer {
    pub fn new(library: FontLibrary) -> Self {
        let mut text = TextPainter::new(library);
        text.set_optical_size(Some(OPTICAL_SIZE));
        Self { text }
    }

    /// 画一帧。`scale` 是点 → 像素的倍数（Retina 为 2）；带 `shadow` 时位图四周留出阴影的边。
    pub fn render(
        &mut self,
        frame: &Frame,
        layout: Layout,
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<Rendered, RenderError> {
        let metrics = Metrics { theme, scale };
        let (content_width, content_height) = self.preferred_size(frame, layout, &metrics);
        let margin = shadow.map_or(0.0, |s| metrics.px(s.margin()));
        let width = (content_width + margin * 2.0).ceil();
        let height = (content_height + margin * 2.0).ceil();
        let mut canvas = Canvas::new(width as u32, height as u32)?;
        let radius = metrics.corner_radius();
        if let Some(shadow) = shadow
            && let Some(content) =
                tiny_skia::Rect::from_xywh(margin, margin, content_width, content_height)
        {
            shadow.paint(&mut canvas, content, radius, scale);
        }
        canvas.fill_round_rect(
            margin,
            margin,
            content_width,
            content_height,
            radius,
            theme.colors.background,
        );
        let mut y = margin + metrics.padding();
        y += self.draw_top_line(&mut canvas, frame, &metrics, margin, y);
        match layout {
            Layout::Vertical => {
                self.draw_vertical(&mut canvas, frame, &metrics, margin, y, content_width);
            }
            Layout::Horizontal if frame.columns > 0 => {
                self.draw_matrix(&mut canvas, frame, &metrics, margin, y, content_width);
            }
            Layout::Horizontal => {
                self.draw_horizontal(&mut canvas, frame, &metrics, margin, y, content_width);
            }
        }
        Ok(Rendered {
            pixmap: canvas.into_pixmap(),
            content_x: margin as u32,
            content_y: margin as u32,
            content_width: content_width.ceil() as u32,
            content_height: content_height.ceil() as u32,
            scale,
        })
    }

    /// 量一段文字在 `size` 点字号下的宽度（点），与原生排版的数值对照用。
    pub fn measure_points(&mut self, text: &str, size: f32) -> f32 {
        let style = TextStyle::new(FontSpec::new(size, size), size, Color::rgb(0, 0, 0), 1.0);
        self.text.measure(text, &style).width
    }

    /// 每个字形用到的字族名，验证回退链用。
    pub fn trace_families(&mut self, text: &str, theme: &Theme) -> Vec<String> {
        let metrics = Metrics { theme, scale: 1.0 };
        self.text.trace_families(text, &metrics.text_style())
    }

    /// 内容需要的像素宽高（不含阴影边）。
    fn preferred_size(&mut self, frame: &Frame, layout: Layout, m: &Metrics) -> (f32, f32) {
        let (top_width, top_height) = self.top_line_size(frame, m);
        let (body_width, body_height) = match layout {
            Layout::Vertical => self.vertical_size(frame, m),
            Layout::Horizontal if frame.columns > 0 => self.matrix_size(frame, m),
            Layout::Horizontal => self.horizontal_size(frame, m),
        };
        let width = top_width.max(body_width) + m.padding() * 2.0;
        // 竖排时候选都很短窗口会窄得难看，给个下限
        let width = match layout {
            Layout::Vertical => width.max(m.theme.min_width_pixels / m.scale),
            Layout::Horizontal => width,
        };
        (width, top_height + body_height + m.padding() * 2.0)
    }

    pub(super) fn measure(&mut self, text: &str, style: &TextStyle) -> TextSize {
        self.text.measure(text, style)
    }

    /// 画一段文字（`x` 左边、`y` 行框顶边），返回它的宽度。
    pub(super) fn draw_text(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> f32 {
        self.text.draw(canvas, text, style, x, y)
    }

    /// 候选词本体。
    fn draw_word(&mut self, canvas: &mut Canvas, m: &Metrics, row: &Row, x: f32, top: f32) {
        let style = m.text_style();
        self.draw_text(canvas, &row.text, &style, x, top);
    }

    /// 来源角标的宽度（点）；没有角标为 0。
    pub(super) fn badge_width(&mut self, badge: Option<&str>, m: &Metrics) -> f32 {
        badge.map_or(0.0, |text| self.measure(text, &m.badge_style()).width)
    }

    /// 把来源角标右对齐画在候选格内（`right` 是格子右边缘），与序号一样垂直居中；返回没画时格右边缘。
    pub(super) fn draw_badge(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        badge: Option<&str>,
        right: f32,
        top: f32,
        text_height: f32,
    ) -> f32 {
        let Some(text) = badge else {
            return right;
        };
        let style = m.badge_style();
        let width = self.measure(text, &style).width;
        let x = right - m.px(BADGE_GAP) - width;
        self.draw_text(canvas, text, &style, x, top + m.index_offset(text_height));
        x
    }

    fn fill_highlight(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        canvas.fill_round_rect(
            x,
            y,
            width,
            height,
            m.corner_radius() / 2.0,
            m.theme.colors.highlight,
        );
    }
}
