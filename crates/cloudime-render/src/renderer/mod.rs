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
use crate::frame::{Frame, HighlightRect, Row, TipSegment, Tone};
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

/// 候选右侧来源角标与候选格右边缘之间的间距（点）。竖排（角标在自己的列里）用它；
/// 横排改用 [`HORIZONTAL_BADGE_GAP_EMS`] 个候选字宽。
const BADGE_GAP: f32 = 4.0;

/// 量「一个候选字有多宽」用的样字：一个汉字 ≈ 一个 em。
const SINGLE_CHAR: &str = "国";

/// 量「一个角标字有多宽」用的样字（角标都是「短 / 句 / 造」这类单个汉字）。
const BADGE_CHAR: &str = "造";

/// 横排里候选词与来源角标之间的最小间隔（字宽数）。
const HORIZONTAL_BADGE_GAP_EMS: f32 = 2.0;

/// 横排展开成网格时每格最小宽度里的候选字宽数（另加角标宽度）。
const HORIZONTAL_CELL_MIN_EMS: f32 = 6.0;

/// 竖排展开成网格时，最长候选之外还要多留的候选字宽数（另加一个角标字宽）。
const VERTICAL_CELL_MIN_EMS: f32 = 2.0;

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

    /// 序号的样式；`highlighted` 是高亮的那一条，序号颜色与普通候选分开取。
    fn index_style(&self, highlighted: bool) -> TextStyle {
        let color = if highlighted {
            self.theme.colors.highlight_index
        } else {
            self.theme.colors.index
        };
        self.style(self.theme.index_font, color)
            .with_family(self.theme.index_family.as_ref())
    }

    /// 页码等页脚小字。
    fn footer_style(&self) -> TextStyle {
        self.style(self.theme.index_font, self.theme.colors.footer)
            .with_family(self.theme.index_family.as_ref())
    }

    /// 翻译 Tip 的一段：字族 / 字号来自 `[candidate] translate_font`，`italic` 给词性用。
    pub(super) fn translate_style(&self, color: Color, italic: bool) -> TextStyle {
        let style = self
            .style(self.theme.translate_font, color)
            .with_family(self.theme.translate_family.as_ref());
        if italic { style.italic() } else { style }
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
            Tone::TranslateMeta => self.theme.colors.translate_meta,
            Tone::TranslateFresh => self.theme.colors.translate_fresh,
            Tone::TranslateLearned => self.theme.colors.translate_learned,
            Tone::Online => self.theme.colors.extra,
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
        let highlight_rects = if frame.columns > 0 {
            // 展开「更多候选项」：竖排 / 横排都走矩阵，一行 `columns` 格、按行优先铺开。
            self.draw_matrix(
                &mut canvas,
                frame,
                &metrics,
                layout,
                margin,
                y,
                content_width,
            )
        } else {
            match layout {
                Layout::Vertical => {
                    self.draw_vertical(&mut canvas, frame, &metrics, margin, y, content_width)
                }
                Layout::Horizontal => {
                    self.draw_horizontal(&mut canvas, frame, &metrics, margin, y, content_width)
                }
            }
        };
        Ok(Rendered {
            pixmap: canvas.into_pixmap(),
            content_x: margin as u32,
            content_y: margin as u32,
            content_width: content_width.ceil() as u32,
            content_height: content_height.ceil() as u32,
            scale,
            highlight_rects,
        })
    }

    /// 量一段文字在 `size` 点字号下的宽度（点），与原生排版的数值对照用。
    pub fn measure_points(&mut self, text: &str, size: f32) -> f32 {
        let style = TextStyle::new(FontSpec::new(size, size), size, Color::rgb(0, 0, 0), 1.0);
        self.text.measure(text, &style).width
    }

    /// 量一段文字在某个字体（字族 + 字号）下的宽高，单位**点**；与真正画出来走同一套整形 / 回退链，
    /// 重量按正常（不加粗）。`family` 是 `None` 就用界面字体（与 [`Theme`] 里那几项一致）。
    ///
    /// 给上层用：脚本的 `cloudime.ui.measure` 拿它算「窗口要多宽才装得下这段译文」。
    pub fn measure_font(
        &mut self,
        text: &str,
        font: &FontSpec,
        family: Option<&String>,
    ) -> (f32, f32) {
        let style = TextStyle::new(*font, font.size, Color::rgb(0, 0, 0), 1.0).with_family(family);
        let size = self.text.measure(text, &style);
        (size.width, size.height)
    }

    /// 每个字形用到的字族名，验证回退链用。
    pub fn trace_families(&mut self, text: &str, theme: &Theme) -> Vec<String> {
        let metrics = Metrics { theme, scale: 1.0 };
        self.text.trace_families(text, &metrics.text_style())
    }

    /// 内容需要的像素宽高（不含阴影边）。
    fn preferred_size(&mut self, frame: &Frame, layout: Layout, m: &Metrics) -> (f32, f32) {
        let (top_width, top_height) = self.top_line_size(frame, m);
        let (body_width, body_height) = if frame.columns > 0 {
            self.matrix_size(frame, m, layout)
        } else {
            match layout {
                Layout::Vertical => self.vertical_size(frame, m),
                Layout::Horizontal => self.horizontal_size(frame, m),
            }
        };
        let width = top_width.max(body_width) + m.padding() * 2.0;
        // 候选都很短时窗口会窄得难看，给个下限（竖排、横排都算）；展开成矩阵后宽度由格子决定，不要再撑。
        // 下限按**点**算（`min_width_pixels` 就是 100% 缩放下的点数），跟着 DPI 与滚轮缩放一起变。
        let width = if frame.columns == 0 {
            width.max(m.px(m.theme.min_width_pixels))
        } else {
            width
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

    /// 候选词本体。`highlighted` 是高亮的那一条，用另一个颜色。
    fn draw_word(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        row: &Row,
        x: f32,
        top: f32,
        highlighted: bool,
    ) {
        let color = if highlighted {
            m.theme.colors.highlight_text
        } else {
            m.theme.colors.text
        };
        let style = m
            .style(m.theme.text_font, color)
            .with_family(m.theme.text_family.as_ref());
        self.draw_text(canvas, &row.text, &style, x, top);
    }

    /// 来源角标的宽度（点）；没有角标为 0。
    pub(super) fn badge_width(&mut self, badge: Option<&str>, m: &Metrics) -> f32 {
        badge.map_or(0.0, |text| self.measure(text, &m.badge_style()).width)
    }

    /// 一个候选字的宽度（点）。用来按「字宽」定间距 / 最小宽度：取一个汉字量，
    /// 楷体 / 雅黑下一个汉字就是一个 em，横排的候选里也以汉字为主。
    pub(super) fn char_width(&mut self, m: &Metrics) -> f32 {
        self.measure(SINGLE_CHAR, &m.text_style()).width
    }

    /// 横排里候选词与来源角标之间的**最小**间隔：两个候选字宽（比原来的 4pt 宽得多，角标不贴词）。
    /// 竖排的角标在自己的列里，仍用 [`BADGE_GAP`]。
    pub(super) fn horizontal_badge_gap(&mut self, m: &Metrics) -> f32 {
        HORIZONTAL_BADGE_GAP_EMS * self.char_width(m)
    }

    /// 横排展开成网格时每格的**最小**宽度：6 个候选字 + 角标宽度（角标不能把候选词挤没）。
    pub(super) fn horizontal_min_cell_width(&mut self, m: &Metrics, badge_width: f32) -> f32 {
        HORIZONTAL_CELL_MIN_EMS * self.char_width(m) + badge_width
    }

    /// 一个角标字的宽度（点）。竖排展开的最小宽度按它算：**不管这一屏有没有角标都算进去**。
    pub(super) fn badge_char_width(&mut self, m: &Metrics) -> f32 {
        self.measure(BADGE_CHAR, &m.badge_style()).width
    }

    /// 把来源角标画在 `left` 处（调用方定它在哪：竖排是「格右边缘往里让 `BADGE_GAP`」，
    /// 横排是「候选文字右边再加 2 个字宽」），与序号一样垂直居中。
    pub(super) fn draw_badge(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        badge: Option<&str>,
        left: f32,
        top: f32,
    ) {
        let Some(text) = badge else {
            return;
        };
        let style = m.badge_style();
        let text_height = m.px(m.theme.text_font.line_height);
        self.draw_text(
            canvas,
            text,
            &style,
            left,
            top + m.index_offset(text_height),
        );
    }

    /// 画高亮条。`rect` 用内容区坐标；`origin` 是内容区左上角在画布里的像素坐标，画前搬过去。
    fn fill_highlight(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        rect: HighlightRect,
        origin: f32,
    ) {
        let rect = rect.translated(origin, origin);
        canvas.fill_round_rect(
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            m.corner_radius() / 2.0,
            m.theme.colors.highlight,
        );
    }

    /// 底部那一行（左侧翻译 Tip、右侧页码）要多高；两样都没有就是 `0`。
    pub(super) fn bottom_line_height(&mut self, frame: &Frame, m: &Metrics) -> f32 {
        if frame.tip.is_none() && frame.footer.is_none() {
            return 0.0;
        }
        let mut height = m.px(m.theme.translate_font.line_height);
        if let Some(footer) = frame.footer.as_deref() {
            height = height.max(self.measure(footer, &m.footer_style()).height);
        }
        height + m.row_padding()
    }

    /// 底部「信息区」要多高：本地 Tip / 页码那一行，加上它下面那几行在线翻译（有才加）。
    /// 脚本体里写了换行（`\n`）就按几行算 —— 一屏最高多少行由脚本自己限。
    pub(super) fn info_height(&mut self, frame: &Frame, m: &Metrics) -> f32 {
        let bottom = self.bottom_line_height(frame, m);
        let Some(segments) = frame.online.as_deref() else {
            return bottom;
        };
        let lines = online_lines(segments).len() as f32;
        bottom + m.px(m.theme.translate_font.line_height) * lines + m.row_padding()
    }

    /// 画底部信息区：`y` 是本地 Tip / 页码那一行的顶边，在线翻译那一块紧贴在它下面（可能有几行）。
    pub(super) fn draw_info(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) {
        let bottom = self.bottom_line_height(frame, m);
        self.draw_bottom_line(canvas, frame, m, left, y, content_width);
        let Some(segments) = frame.online.as_deref() else {
            return;
        };
        // 在线翻译独占底部信息区下面那几行（本地 Tip 与页码在上面那一行里），逐行左对齐、每行放不下各自截断
        let budget = content_width - m.padding() * 2.0;
        let line_height = m.px(m.theme.translate_font.line_height);
        for (i, line) in online_lines(segments).into_iter().enumerate() {
            self.draw_segments(
                canvas,
                &line,
                m,
                left + m.padding(),
                y + bottom + line_height * i as f32,
                budget,
            );
        }
    }

    /// 画底部那一行：`y` 是行框顶边（内容区坐标），`content_width` 是内容区宽度。
    /// 页码靠右、翻译 Tip 靠左，放不下就把 Tip 截断。
    pub(super) fn draw_bottom_line(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) {
        let right = left + content_width - m.padding();
        let mut budget = right - left - m.padding() * 2.0;
        if let Some(footer) = frame.footer.as_deref() {
            let style = m.footer_style();
            let size = self.measure(footer, &style);
            self.draw_text(canvas, footer, &style, right - size.width, y);
            budget -= size.width + m.column_gap();
        }
        let Some(segments) = frame.tip.as_deref() else {
            return;
        };
        self.draw_segments(canvas, segments, m, left + m.padding(), y, budget);
    }

    /// 从左往右一段一段画字（翻译 Tip / 在线翻译共用）：`x` 是左端、`y` 是行框顶边，
    /// `budget` 是还剩多少宽度（点），放不下就把这一段截断。
    fn draw_segments(
        &mut self,
        canvas: &mut Canvas,
        segments: &[TipSegment],
        m: &Metrics,
        x: f32,
        y: f32,
        budget: f32,
    ) {
        let mut x = x;
        let mut budget = budget;
        for segment in segments {
            if budget <= 0.0 {
                break;
            }
            let style = m.translate_style(m.tone_color(segment.tone), segment.italic);
            let (shown, _) = self.truncate(&segment.text, &style, budget);
            let used = self.draw_text(canvas, &shown, &style, x, y);
            x += used;
            budget -= used;
        }
    }
}

/// 在线翻译那一段段文字按 `\n` 拆成几行（脚本体里写了换行就显示成多行，换行后面的字从新一行左端开始）。
/// 没写换行时就是一行 —— 与以前一样。
fn online_lines(segments: &[TipSegment]) -> Vec<Vec<TipSegment>> {
    let mut lines: Vec<Vec<TipSegment>> = vec![Vec::new()];
    for segment in segments {
        for (i, part) in segment.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
            }
            if !part.is_empty() {
                lines.last_mut().expect("至少有一行").push(TipSegment::new(
                    part,
                    segment.tone,
                    segment.italic,
                ));
            }
        }
    }
    lines
}

/// 目标高亮条的矩形（内容区坐标）：有动画信息时从它的起点矩形 lerp 到目标行矩形，否则就是目标行矩形。
/// `rects` 是各候选行 / 格的高亮矩形，下标与 `frame.rows` 对齐。
fn highlight_rect(frame: &Frame, rects: &[HighlightRect]) -> Option<HighlightRect> {
    let to = frame.highlighted?;
    let target = *rects.get(to)?;
    let Some(animation) = frame.highlight_animation else {
        return Some(target);
    };
    Some(HighlightRect::lerp(
        animation.from,
        target,
        animation.progress,
    ))
}

#[cfg(test)]
mod tests {
    use super::highlight_rect;
    use crate::fonts::FontLibrary;
    use crate::frame::{Frame, HighlightAnimation, HighlightRect, Row, TipSegment, Tone};
    use crate::layout::Layout;
    use crate::renderer::Renderer;
    use crate::theme::Theme;

    fn rects() -> [HighlightRect; 3] {
        [
            HighlightRect::new(0.0, 0.0, 10.0, 10.0),
            HighlightRect::new(0.0, 10.0, 10.0, 20.0),
            HighlightRect::new(0.0, 20.0, 10.0, 30.0),
        ]
    }

    /// 没有动画时高亮条就是目标行的矩形；第 0 行也要能取到。
    #[test]
    fn without_animation_the_target_row_is_used() {
        let frame = Frame {
            highlighted: Some(0),
            ..Frame::default()
        };
        assert_eq!(highlight_rect(&frame, &rects()), Some(rects()[0]));
    }

    /// 起点用显式矩形，从第 0 行滑到第 1 行：进度 0.5 时落在两行中间。
    #[test]
    fn animation_lerps_between_explicit_start_and_target() {
        let frame = Frame {
            highlighted: Some(1),
            highlight_animation: Some(HighlightAnimation {
                from: rects()[0],
                progress: 0.5,
            }),
            ..Frame::default()
        };
        assert_eq!(
            highlight_rect(&frame, &rects()),
            Some(HighlightRect::new(0.0, 5.0, 10.0, 15.0))
        );
    }

    /// 没有高亮行（`highlighted == None`）时返回 `None`。
    #[test]
    fn no_highlight_row_draws_nothing() {
        let frame = Frame::default();
        assert_eq!(highlight_rect(&frame, &rects()), None);
    }

    /// 在线翻译那一行里写了换行就按多行算高度：每多一行窗口就多一条行高。
    #[test]
    fn online_newlines_make_the_window_taller() {
        // 没有系统字体的环境（CI 容器）跳过
        let Some(mut renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let theme = Theme::new();
        let mut height = |online: &str| {
            let frame = Frame {
                online: Some(vec![TipSegment::new(online, Tone::Online, false)]),
                ..Frame::default()
            };
            renderer
                .render(&frame, Layout::Vertical, &theme, 1.0, None)
                .unwrap()
                .content_height
        };
        let one = height("a");
        let two = height("a\nb");
        let three = height("a\nb\nc");
        assert!(
            two > one && three > two,
            "多一行该更高：{one} {two} {three}"
        );
        let (first, second) = (two - one, three - two);
        assert!(
            first.abs_diff(second) <= 1,
            "每多一行的增量该一致：{first} vs {second}"
        );
    }

    /// 换行是真画出来的：两行「M」的字形分别落在墨迹范围的上下两半。
    #[test]
    fn online_newlines_actually_paint_two_lines() {
        let Some(mut renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let frame = Frame {
            online: Some(vec![TipSegment::new("M\nM", Tone::Online, false)]),
            ..Frame::default()
        };
        let pixmap = renderer
            .render(&frame, Layout::Vertical, &Theme::new(), 1.0, None)
            .unwrap()
            .pixmap;
        let (w, h) = (pixmap.width(), pixmap.height());
        // 在线译文用的蓝色（`Palette::online`，抗锯齿边缘偏浅，只认深一点的核心像素）
        let inked = |x: u32, y: u32| {
            let p = pixmap.pixel(x, y).unwrap();
            p.red() < 120 && p.blue() > 150
        };
        let rows: Vec<u32> = (0..h).filter(|&y| (0..w).any(|x| inked(x, y))).collect();
        let (first, last) = (*rows.first().expect("一个字都没画上"), rows[rows.len() - 1]);
        let middle = (first + last) / 2;
        let upper = rows.iter().filter(|&&y| y < middle).count();
        let lower = rows.iter().filter(|&&y| y > middle).count();
        assert!(
            upper > 0 && lower > 0,
            "两行该各有字形（墨迹 y 范围 {first}..{last}）"
        );
    }

    /// 「展开后每个候选项的最大宽度」：太长的候选展开时截到上限，格子不再跟着变长。
    #[test]
    fn expanded_cells_respect_the_maximum_width() {
        // 没有系统字体的环境（CI 容器）跳过
        let Some(mut renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        // 48 个汉字：展开（竖排）时基准宽度远大于上限
        let frame = Frame {
            columns: 5,
            rows: vec![Row::plain(0, "国".repeat(48))],
            ..Frame::default()
        };
        let cell = |renderer: &mut Renderer, cap: f32| {
            let mut theme = Theme::new();
            theme.max_cell_width = cap;
            let out = renderer
                .render(&frame, Layout::Vertical, &theme, 1.0, None)
                .unwrap();
            out.highlight_rects
                .first()
                .map(|rect| rect.width())
                .unwrap_or(0.0)
        };
        let uncapped = cell(&mut renderer, 0.0);
        assert!(uncapped > 460.0, "不限宽时该撑得很长：{uncapped}");

        // 格宽被截到 420，高亮条两侧各多出 5 点的留边
        let capped = cell(&mut renderer, 420.0);
        assert!(
            (capped - 430.0).abs() <= 1.0,
            "上限该把格子压到 420：{capped}"
        );
    }

    /// 候选框最小宽度横排也生效（以前只在竖排时撑窗口）。
    #[test]
    fn the_minimum_width_applies_to_horizontal_too() {
        // 没有系统字体的环境（CI 容器）跳过
        let Some(mut renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let mut theme = Theme::new();
        theme.min_width_pixels = 300.0;
        // 单个候选：不设下限时横排窗口只有几十点宽
        let frame = Frame {
            rows: vec![Row::plain(0, "你")],
            ..Frame::default()
        };
        let wide = |renderer: &mut Renderer, minimum: f32| {
            let mut theme = theme.clone();
            theme.min_width_pixels = minimum;
            renderer
                .render(&frame, Layout::Horizontal, &theme, 1.0, None)
                .unwrap()
                .content_width
        };
        let narrow = wide(&mut renderer, 0.0);
        let widened = wide(&mut renderer, 300.0);
        assert!(narrow < 300, "不设下限时该是窄的：{narrow}");
        assert!(widened >= 300, "横排也该撑到最小宽度：{widened}");
    }
}
