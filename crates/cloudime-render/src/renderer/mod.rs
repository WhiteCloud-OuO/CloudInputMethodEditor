//! 渲染器：一帧 + 排布 + 主题 → 位图。排版逻辑：顶部拼音行，竖排一行一个候选、横排排成一行。
//!
//! 内部全用像素：主题里的点数进来先乘缩放倍数。文字的 y 都指行框顶边，字形在行高里垂直居中。

mod columns;
mod horizontal;
mod item;
mod key_map;
mod matrix;
mod rendered;
mod status;
mod top_line;
mod vertical;

use std::cell::RefCell;
use std::rc::Rc;

use crate::color::Color;
use crate::document::{self, Document, Element, HitRect, HitShape};
use crate::elements::{
    ImageElement, LineElement, PathElement, RectElement, StrokeElement, SvgElement, TextElement,
};
use crate::error::RenderError;
use crate::fonts::FontLibrary;
use crate::frame::{Auxiliary, Frame, HighlightRect, Row, TipSegment, Tone};
use crate::layout::Layout;
use crate::render::Render;
use crate::shadow::Shadow;
use crate::text::{TextPainter, TextSize, TextStyle};
use crate::theme::{FontSpec, Theme};

pub use key_map::{
    KeyCap, KeyMapLayout, draw_key_map, key_map_cell, key_map_pixel_size, key_map_size,
};
pub use rendered::{Rendered, RenderedDocument};
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

/// 辅助码两头的方括号：紧跟候选词那一段画成 `[XH]`（见 `Renderer::draw_auxiliary`），
/// 括号自己不带下划线（只有已经敲上的那几位带）。
const AUXILIARY_OPEN: &str = "[";
const AUXILIARY_CLOSE: &str = "]";

/// 横排里候选词与来源角标之间的最小间隔（字宽数）。
const HORIZONTAL_BADGE_GAP_EMS: f32 = 2.0;

/// 横排展开成网格时每格最小宽度里的候选字宽数（另加角标宽度）。
const HORIZONTAL_CELL_MIN_EMS: f32 = 6.0;

/// 竖排展开成网格时，最长候选之外还要多留的候选字宽数（另加一个角标字宽）。
const VERTICAL_CELL_MIN_EMS: f32 = 2.0;

/// 光学字号（点）：20 pt 以下系统给字体用的就是这一档（调研期在 macOS 上量的）。
const OPTICAL_SIZE: f32 = 17.0;

pub struct Renderer {
    /// 文字测绘（渲染与量字共用一个句柄：画的时候还要量字）。
    text: Rc<RefCell<TextPainter>>,

    /// 阴影层缓存（最多 [`SHADOW_CACHE_ENTRIES`] 份）：阴影只跟尺寸 / 圆角 / 参数有关，跟内容无关。
    shadows: RefCell<Vec<(ShadowKey, tiny_skia::Pixmap)>>,
}

/// 阴影层缓存留几份（候选窗 / 键位映射参考 / 状态条 / 状态提示各一份就够）。
const SHADOW_CACHE_ENTRIES: usize = 4;

/// 阴影层的「同不同样」：画布尺寸 + 内容矩形 + 圆角 + 缩放 + 阴影参数。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ShadowKey {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    content_width: u32,
    content_height: u32,
    radius: u32,
    scale: u32,
    blur: u32,
    offset: u32,
    color: Color,
}

impl ShadowKey {
    fn new(
        width: u32,
        height: u32,
        content: tiny_skia::Rect,
        radius: f32,
        scale: f32,
        shadow: &Shadow,
    ) -> Self {
        Self {
            width,
            height,
            x: content.x().to_bits(),
            y: content.y().to_bits(),
            content_width: content.width().to_bits(),
            content_height: content.height().to_bits(),
            radius: radius.to_bits(),
            scale: scale.to_bits(),
            blur: shadow.blur.to_bits(),
            offset: shadow.offset_y.to_bits(),
            color: shadow.color,
        }
    }
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
        Self {
            text: Rc::new(RefCell::new(text)),
            shadows: RefCell::new(Vec::new()),
        }
    }

    /// 画一帧。`scale` 是点 → 像素的倍数（Retina 为 2）；带 `shadow` 时位图四周留出阴影的边。
    pub fn render(
        &self,
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
        let mut render = Render::new(width as u32, height as u32, self.text.clone())?;
        let radius = metrics.corner_radius();
        if let Some(shadow) = shadow
            && let Some(content) =
                tiny_skia::Rect::from_xywh(margin, margin, content_width, content_height)
        {
            self.paint_shadow(&mut render, content, radius, scale, shadow);
        }
        render.round_rect(
            margin,
            margin,
            content_width,
            content_height,
            radius,
            theme.colors.background,
        );
        let mut y = margin + metrics.padding();
        y += self.draw_top_line(&mut render, frame, &metrics, margin, y);
        let highlight_rects = if frame.columns > 0 {
            // 展开「更多候选项」：竖排 / 横排都走矩阵，一行 `columns` 格、按行优先铺开。
            self.draw_matrix(
                &mut render,
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
                    self.draw_vertical(&mut render, frame, &metrics, margin, y, content_width)
                }
                Layout::Horizontal => {
                    self.draw_horizontal(&mut render, frame, &metrics, margin, y, content_width)
                }
            }
        };
        Ok(Rendered {
            pixmap: render.into_pixmap(),
            content_x: margin as u32,
            content_y: margin as u32,
            content_width: content_width.ceil() as u32,
            content_height: content_height.ceil() as u32,
            scale,
            highlight_rects,
        })
    }

    /// 量一帧排出来的**内容区**像素尺寸（宽, 高）—— 不画位图。
    ///
    /// 给「辅助窗口模板」（`server` 的 `auxiliary_window_template`）按候选窗口的形状算默认大小用：
    /// 拿一个「9 个两字候选」的帧去量竖排高度、拿「6 个两字候选」的帧去量横排宽度。
    pub fn measure_frame(
        &self,
        frame: &Frame,
        layout: Layout,
        theme: &Theme,
        scale: f32,
    ) -> (u32, u32) {
        let metrics = Metrics { theme, scale };
        let (width, height) = self.preferred_size(frame, layout, &metrics);
        (width.ceil() as u32, height.ceil() as u32)
    }

    /// 画一块**空面板**：主题背景色 + 圆角（带 `shadow` 时四周留阴影边），内容区尺寸由调用方给。
    ///
    /// 给「辅助窗口模板」（`server` 的 `auxiliary_window_template`）用 —— 往里面画什么由用它的那件资产自己定。
    pub fn render_panel(
        &self,
        content_width: f32,
        content_height: f32,
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<Rendered, RenderError> {
        let (render, margin) =
            self.panel_canvas(content_width, content_height, theme, scale, shadow, true)?;
        Ok(Self::panel_rendered(
            render,
            margin,
            (content_width, content_height),
            scale,
        ))
    }

    /// 画一份**脚本内容**（[`Document`]：一块面板 + 元素表），并给出每个元素的包围盒（点）——
    /// 脚本窗口的鼠标命中测试用它。
    ///
    /// 元素坐标一律是点；这里乘 `scale` 换成像素。画在 `panel_canvas` 的内容区里（背景 / 圆角 / 阴影与
    /// 候选窗同源），超出内容区的部分自然被位图边界裁掉。
    pub fn render_document(
        &self,
        document: &Document,
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
        background: bool,
    ) -> Result<RenderedDocument, RenderError> {
        let width = (document.width * scale).max(1.0);
        let height = (document.height * scale).max(1.0);
        let (mut render, margin) =
            self.panel_canvas(width, height, theme, scale, shadow, background)?;
        let metrics = Metrics { theme, scale };
        let mut hits = Vec::with_capacity(document.elements.len());
        for element in &document.elements {
            hits.push(draw_element(
                &mut render,
                element,
                &metrics,
                margin,
                (0.0, 0.0),
            ));
        }
        Ok(RenderedDocument {
            rendered: Self::panel_rendered(render, margin, (width, height), scale),
            hits,
        })
    }

    /// 在一块**空画布**上铺阴影层：同样的尺寸 / 参数直接复用上次算好的那一层。
    ///
    /// 阴影只跟「内容矩形 + 圆角 + 模糊 / 偏移 / 颜色 + 缩放」有关，跟候选词是什么无关；
    /// 每次出帧重算的话 3 遍盒式模糊 + 整画布混合要 2.5–10ms（占整帧九成），缓存后只剩一次整块拷贝。
    fn paint_shadow(
        &self,
        render: &mut Render,
        content: tiny_skia::Rect,
        radius: f32,
        scale: f32,
        shadow: &Shadow,
    ) {
        let key = ShadowKey::new(
            render.width(),
            render.height(),
            content,
            radius,
            scale,
            shadow,
        );
        if let Some(index) = self
            .shadows
            .borrow()
            .iter()
            .position(|(cached, _)| *cached == key)
        {
            let layers = self.shadows.borrow();
            render.blit(&layers[index].1);
            return;
        }
        let Some(layer) = shadow.layer(render.width(), render.height(), content, radius, scale)
        else {
            return;
        };
        render.blit(&layer);
        let mut layers = self.shadows.borrow_mut();
        if layers.len() >= SHADOW_CACHE_ENTRIES {
            layers.remove(0);
        }
        layers.push((key, layer));
    }

    /// 面板类窗口的画布与内容区留白（阴影边）：[`Self::render_panel`] / [`Self::render_key_map`] 共用。
    fn panel_canvas(
        &self,
        content_width: f32,
        content_height: f32,
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
        background: bool,
    ) -> Result<(Render, f32), RenderError> {
        let metrics = Metrics { theme, scale };
        let margin = shadow.map_or(0.0, |s| metrics.px(s.margin()));
        let width = (content_width + margin * 2.0).ceil();
        let height = (content_height + margin * 2.0).ceil();
        let mut render = Render::new(width as u32, height as u32, self.text.clone())?;
        let radius = metrics.corner_radius();
        if let Some(shadow) = shadow
            && let Some(content) =
                tiny_skia::Rect::from_xywh(margin, margin, content_width, content_height)
        {
            self.paint_shadow(&mut render, content, radius, scale, shadow);
        }
        if background {
            render.round_rect(
                margin,
                margin,
                content_width,
                content_height,
                radius,
                theme.colors.background,
            );
        }
        Ok((render, margin))
    }

    /// 画布 + 内容区尺寸 → [`Rendered`]。
    fn panel_rendered(render: Render, margin: f32, content: (f32, f32), scale: f32) -> Rendered {
        Rendered {
            pixmap: render.into_pixmap(),
            content_x: margin as u32,
            content_y: margin as u32,
            content_width: content.0.ceil() as u32,
            content_height: content.1.ceil() as u32,
            scale,
            highlight_rects: Vec::new(),
        }
    }

    /// 量一段文字在 `size` 点字号下的宽度（点），与原生排版的数值对照用。
    pub fn measure_points(&self, text: &str, size: f32) -> f32 {
        let style = TextStyle::new(FontSpec::new(size, size), size, Color::rgb(0, 0, 0), 1.0);
        self.text.borrow_mut().measure(text, &style).width
    }

    /// 量一段文字在某个字体（字族 + 字号）下的宽高，单位**点**；与真正画出来走同一套整形 / 回退链，
    /// 重量按正常（不加粗）。`family` 是 `None` 就用界面字体（与 [`Theme`] 里那几项一致）。
    ///
    /// 给上层用：脚本的 `cloudime.ui.measure` 拿它算「窗口要多宽才装得下这段译文」。
    pub fn measure_font(&self, text: &str, font: &FontSpec, family: Option<&String>) -> (f32, f32) {
        let style = TextStyle::new(*font, font.size, Color::rgb(0, 0, 0), 1.0).with_family(family);
        let size = self.text.borrow_mut().measure(text, &style);
        (size.width, size.height)
    }

    /// 每个字形用到的字族名，验证回退链用。
    pub fn trace_families(&self, text: &str, theme: &Theme) -> Vec<String> {
        let metrics = Metrics { theme, scale: 1.0 };
        self.text
            .borrow_mut()
            .trace_families(text, &metrics.text_style())
    }

    /// 内容需要的像素宽高（不含阴影边）。
    fn preferred_size(&self, frame: &Frame, layout: Layout, m: &Metrics) -> (f32, f32) {
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

    pub(super) fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        self.text.borrow_mut().measure(text, style)
    }

    /// 候选词本体。`highlighted` 是高亮的那一条，用另一个颜色。
    fn draw_word(
        &self,
        render: &mut Render,
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
        render.text(&row.text, &style, x, top);
    }

    /// 来源角标的宽度（点）；没有角标为 0。
    pub(super) fn badge_width(&self, badge: Option<&str>, m: &Metrics) -> f32 {
        badge.map_or(0.0, |text| self.measure(text, &m.badge_style()).width)
    }

    /// 候选词与辅助码之间那**一个空格**的宽度（点）——`[XH]` 前面隔的就是它。
    pub(super) fn space_width(&self, m: &Metrics) -> f32 {
        self.measure(" ", &m.text_style()).width
    }

    /// 辅助码那一段的宽度（点）：**一个空格 + `[` + 已敲的几位 + 还差的几位 + `]`**；没有为 0。
    pub(super) fn auxiliary_width(&self, auxiliary: Option<&Auxiliary>, m: &Metrics) -> f32 {
        let Some(auxiliary) = auxiliary else {
            return 0.0;
        };
        let style = m.badge_style();
        let bracket = self.measure(AUXILIARY_OPEN, &style).width
            + self.measure(AUXILIARY_CLOSE, &style).width;
        self.space_width(m)
            + bracket
            + self.measure(&auxiliary.typed, &style).width
            + self.measure(&auxiliary.rest, &style).width
    }

    /// 一个候选字的宽度（点）。用来按「字宽」定间距 / 最小宽度：取一个汉字量，
    /// 楷体 / 雅黑下一个汉字就是一个 em，横排的候选里也以汉字为主。
    pub(super) fn char_width(&self, m: &Metrics) -> f32 {
        self.measure(SINGLE_CHAR, &m.text_style()).width
    }

    /// 横排里候选词与来源角标之间的**最小**间隔：两个候选字宽（比原来的 4pt 宽得多，角标不贴词）。
    /// 竖排的角标在自己的列里，仍用 [`BADGE_GAP`]。
    pub(super) fn horizontal_badge_gap(&self, m: &Metrics) -> f32 {
        HORIZONTAL_BADGE_GAP_EMS * self.char_width(m)
    }

    /// 横排展开成网格时每格的**最小**宽度：6 个候选字 + 角标宽度（角标不能把候选词挤没）。
    pub(super) fn horizontal_min_cell_width(&self, m: &Metrics, badge_width: f32) -> f32 {
        HORIZONTAL_CELL_MIN_EMS * self.char_width(m) + badge_width
    }

    /// 一个角标字的宽度（点）。竖排展开的最小宽度按它算：**不管这一屏有没有角标都算进去**。
    pub(super) fn badge_char_width(&self, m: &Metrics) -> f32 {
        self.measure(BADGE_CHAR, &m.badge_style()).width
    }

    /// 把来源角标画在 `left` 处（调用方定它在哪：竖排是「格右边缘往里让 `BADGE_GAP`」，
    /// 横排是「候选文字右边再加 2 个字宽」），与序号一样垂直居中。
    pub(super) fn draw_badge(
        &self,
        render: &mut Render,
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
        render.text(text, &style, left, top + m.index_offset(text_height));
    }

    /// 把辅助码画在 `left` 处 —— 调用方给的是「候选词右边再加一个空格」那个位置
    /// （即 `text_x + 候选词宽度 + `[`Self::space_width`]），字体与颜色跟来源角标一致。
    /// 两头补 `[` `]`（不参与下划线），**已经敲上的那几位画下划线**（表示这段码已经打上去了）。
    pub(super) fn draw_auxiliary(
        &self,
        render: &mut Render,
        m: &Metrics,
        auxiliary: Option<&Auxiliary>,
        left: f32,
        top: f32,
    ) {
        let Some(auxiliary) = auxiliary else {
            return;
        };
        let style = m.badge_style();
        let typed_style = style.clone().underlined();
        let text_height = m.px(m.theme.text_font.line_height);
        let top = top + m.index_offset(text_height);
        let mut x = left;
        for (segment, style) in [
            (AUXILIARY_OPEN, &style),
            (auxiliary.typed.as_str(), &typed_style),
            (auxiliary.rest.as_str(), &style),
            (AUXILIARY_CLOSE, &style),
        ] {
            if segment.is_empty() {
                continue;
            }
            x += render.text(segment, style, x, top);
        }
    }

    /// 画高亮条。`rect` 用内容区坐标；`origin` 是内容区左上角在画布里的像素坐标，画前搬过去。
    fn fill_highlight(&self, render: &mut Render, m: &Metrics, rect: HighlightRect, origin: f32) {
        let rect = rect.translated(origin, origin);
        render.round_rect(
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            m.corner_radius() / 2.0,
            m.theme.colors.highlight,
        );
    }

    /// 底部那一行（左侧翻译 Tip、右侧页码）要多高；两样都没有就是 `0`。
    pub(super) fn bottom_line_height(&self, frame: &Frame, m: &Metrics) -> f32 {
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
    pub(super) fn info_height(&self, frame: &Frame, m: &Metrics) -> f32 {
        let bottom = self.bottom_line_height(frame, m);
        let Some(segments) = frame.online.as_deref() else {
            return bottom;
        };
        let lines = online_lines(segments).len() as f32;
        bottom + m.px(m.theme.translate_font.line_height) * lines + m.row_padding()
    }

    /// 画底部信息区：`y` 是本地 Tip / 页码那一行的顶边，在线翻译那一块紧贴在它下面（可能有几行）。
    pub(super) fn draw_info(
        &self,
        render: &mut Render,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) {
        let bottom = self.bottom_line_height(frame, m);
        self.draw_bottom_line(render, frame, m, left, y, content_width);
        let Some(segments) = frame.online.as_deref() else {
            return;
        };
        // 在线翻译独占底部信息区下面那几行（本地 Tip 与页码在上面那一行里），逐行左对齐、每行放不下各自截断
        let budget = content_width - m.padding() * 2.0;
        let line_height = m.px(m.theme.translate_font.line_height);
        for (i, line) in online_lines(segments).into_iter().enumerate() {
            self.draw_segments(
                render,
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
        &self,
        render: &mut Render,
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
            let size = render.measure(footer, &style);
            render.text(footer, &style, right - size.width, y);
            budget -= size.width + m.column_gap();
        }
        let Some(segments) = frame.tip.as_deref() else {
            return;
        };
        self.draw_segments(render, segments, m, left + m.padding(), y, budget);
    }

    /// 从左往右一段一段画字（翻译 Tip / 在线翻译共用）：`x` 是左端、`y` 是行框顶边，
    /// `budget` 是还剩多少宽度（点），放不下就把这一段截断。
    fn draw_segments(
        &self,
        render: &mut Render,
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
            let (shown, _) = truncate_with(
                |text, style| render.measure(text, style),
                &segment.text,
                &style,
                budget,
            );
            let used = render.text(&shown, &style, x, y);
            x += used;
            budget -= used;
        }
    }
}

const ELLIPSIS: &str = "…";

/// `text` 宽度超过 `max_width` 就从末尾去字、补上「…」直到放得下；返回显示文字与是否截断过。
///
/// `measure` 是「量一段字」那一手 —— 绘制路上是 [`crate::Render`] 的 `measure`，版面阶段是
/// [`Renderer::measure`]，键位映射两条路共用（`draw_key_map` 里传目标的 `measure`）。
pub(super) fn truncate_with(
    measure: impl Fn(&str, &TextStyle) -> TextSize,
    text: &str,
    style: &TextStyle,
    max_width: f32,
) -> (String, bool) {
    if text.is_empty() || measure(text, style).width <= max_width {
        return (text.to_owned(), false);
    }
    let mut kept: Vec<char> = text.chars().collect();
    while kept.pop().is_some() {
        let mut candidate: String = kept.iter().collect();
        candidate.push_str(ELLIPSIS);
        if kept.is_empty() || measure(&candidate, style).width <= max_width {
            return (candidate, true);
        }
    }
    (ELLIPSIS.to_owned(), true)
}

/// 画脚本内容里的一个元素，返回它在**内容区**里的命中区域（点）：能抠形状的按形状算
///（圆角 / 圆 / 线段 / 路径填充），文字与图像按外框。画不出来（SVG 解析失败 / 空路径）返回 `None`。
///
/// `margin` 是位图四周的阴影留白（像素）：内容区左上角在位图里的位置，所有坐标都要加上它。
/// 命中区域仍然只报**内容区**坐标（与 `Document` 同一套，鼠标事件那边减掉留白再换算）。
fn draw_element(
    render: &mut Render,
    element: &Element,
    m: &Metrics,
    margin: f32,
    origin: (f32, f32),
) -> Option<HitShape> {
    let scale = m.scale;
    // 位置要加上内容区在位图里的偏移（阴影留白）与控件容器的平移；尺寸不用。
    let x_at = |value: f32| margin + (origin.0 + value) * scale;
    let y_at = |value: f32| margin + (origin.1 + value) * scale;
    let len = |value: f32| value * scale;
    match element {
        Element::Rect {
            x,
            y,
            width,
            height,
            radius,
            color,
        } => {
            let hit = if *radius > 0.0 {
                render.round_rect(
                    x_at(*x),
                    y_at(*y),
                    len(*width),
                    len(*height),
                    len(*radius),
                    *color,
                );
                HitShape::RoundRect {
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                    radius: *radius,
                }
            } else {
                render.rect(x_at(*x), y_at(*y), len(*width), len(*height), *color);
                HitShape::Rect(HitRect::new(*x, *y, *width, *height))
            };
            Some(hit)
        }
        Element::Line {
            from,
            to,
            thickness,
            color,
        } => {
            render.line(
                (x_at(from.0), y_at(from.1)),
                (x_at(to.0), y_at(to.1)),
                len(*thickness),
                *color,
            );
            Some(HitShape::Segment {
                from: *from,
                to: *to,
                thickness: *thickness,
            })
        }
        Element::Circle {
            cx,
            cy,
            radius,
            color,
        } => {
            render.circle(x_at(*cx), y_at(*cy), len(*radius), *color);
            Some(HitShape::Circle {
                cx: *cx,
                cy: *cy,
                radius: *radius,
            })
        }
        Element::Text {
            x,
            y,
            text,
            size,
            slot,
            color,
            italic,
        } => {
            let font = FontSpec::with_size(*size).scaled(scale);
            let mut style = TextStyle::new(font, *size, *color, m.theme.text_gamma);
            style.family = document::family_for(m.theme, *slot).cloned();
            style.italic = *italic;
            let width = render.text(text, &style, x_at(*x), y_at(*y)) / scale;
            Some(HitShape::Rect(HitRect::new(
                *x,
                *y,
                width,
                font.line_height / scale,
            )))
        }
        Element::Svg {
            x,
            y,
            size,
            source,
            tint,
        } => {
            render
                .svg(source, x_at(*x), y_at(*y), len(*size), *tint)
                .ok()?;
            Some(HitShape::Rect(HitRect::new(*x, *y, *size, *size)))
        }
        Element::Image {
            x,
            y,
            width,
            height,
            image,
        } => {
            render.image(image, x_at(*x), y_at(*y), len(*width), len(*height));
            Some(HitShape::Rect(HitRect::new(*x, *y, *width, *height)))
        }
        Element::Path { commands, color } => {
            let path = document::build_path(
                commands,
                scale,
                margin + origin.0 * scale,
                margin + origin.1 * scale,
            )?;
            render.path(&path, *color);
            Some(HitShape::Path {
                contours: document::path_contours(commands),
            })
        }
        // 静态控件没有命中框：只画，不参与 hover / click / element 编号。
        Element::Label(label) => {
            label.draw(render, m.theme, scale, origin, margin);
            None
        }
        Element::TextBlock(text) => {
            text.draw(render, m.theme, scale, origin, margin);
            None
        }
        Element::StaticShape(shape) => {
            shape.draw(render, scale, origin, margin);
            None
        }
        Element::StaticImage(image) => {
            image.draw(render, scale, origin, margin);
            None
        }
        Element::Panel(panel) => {
            if let Some(fill) = panel.fill {
                if panel.radius > 0.0 {
                    render.round_rect(
                        x_at(panel.x),
                        y_at(panel.y),
                        len(panel.width),
                        len(panel.height),
                        len(panel.radius),
                        fill,
                    );
                } else {
                    render.rect(
                        x_at(panel.x),
                        y_at(panel.y),
                        len(panel.width),
                        len(panel.height),
                        fill,
                    );
                }
            }
            if panel.border_width > 0.0
                && let Some(rect) = tiny_skia::Rect::from_xywh(
                    x_at(panel.x),
                    y_at(panel.y),
                    len(panel.width),
                    len(panel.height),
                )
            {
                render.stroke_round_rect(
                    rect,
                    len(panel.radius),
                    len(panel.border_width),
                    panel.color,
                );
            }
            let child_origin = (origin.0 + panel.x, origin.1 + panel.y);
            for child in &panel.children {
                let _ = draw_element(render, child, m, margin, child_origin);
            }
            None
        }
        Element::FlatSvgButton(button) => {
            button.draw(render, scale, origin, margin);
            Some(button.hit_shape(origin))
        }
        Element::ScrollBar(bar) => {
            bar.draw(render, m.theme, scale, origin, margin);
            Some(bar.hit_shape(origin))
        }
        Element::TextBox(edit) => {
            edit.draw(render, m.theme, scale, origin, margin);
            Some(edit.hit_shape(origin))
        }
        Element::TextEdit(edit) => {
            edit.draw(render, m.theme, scale, origin, margin);
            Some(edit.hit_shape(origin))
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
    use crate::frame::{
        Auxiliary, Frame, HighlightAnimation, HighlightRect, Preedit, Row, TipSegment, Tone,
    };
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

    /// 辅助码里「已经敲上」的那几位画下划线：同一段 `[RS]`，敲了一位（`R` 带下划线）与一位没敲
    /// 画出来的像素必须**有差别**（那条线真的画出来了），而且差别只该是那条线本身。
    #[test]
    fn the_typed_part_of_the_auxiliary_code_is_underlined() {
        let Some(mut renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let theme = Theme::new();
        let frame = |typed: &str| Frame {
            rows: vec![Row {
                index: "1".to_owned(),
                text: "侧".to_owned(),
                annotation: Vec::new(),
                badge: None,
                auxiliary: Some(Auxiliary::split("RS", typed)),
            }],
            ..Frame::default()
        };
        let render = |renderer: &mut Renderer, typed: &str| {
            renderer
                .render(&frame(typed), Layout::Vertical, &theme, 1.0, None)
                .unwrap()
                .pixmap
        };
        let underlined = render(&mut renderer, "R");
        let plain = render(&mut renderer, "");
        assert_eq!(
            (underlined.width(), underlined.height()),
            (plain.width(), plain.height())
        );
        let differing = underlined
            .pixels()
            .iter()
            .zip(plain.pixels())
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing > 0, "敲了一位，`R` 下该有一条下划线");
        assert!(differing < 100, "差别只该是那条线：{differing} 个像素");
    }

    /// 窗口模板要的**空面板**：内容区尺寸就是要的那个（不跟内容走），且尺寸与位图一致。
    #[test]
    fn a_panel_keeps_the_size_it_was_given() {
        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let out = renderer
            .render_panel(320.0, 480.0, &Theme::new(), 1.0, None)
            .unwrap();
        assert_eq!((out.content_width, out.content_height), (320, 480));
        assert_eq!((out.pixmap.width(), out.pixmap.height()), (320, 480));
    }

    /// 量帧尺寸跟着候选个数走：竖排 9 个比 6 个高、横排 6 个比 3 个宽
    /// （窗口模板的默认大小按这两条算）。
    #[test]
    fn measuring_a_frame_follows_the_row_count() {
        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let theme = Theme::new();
        let frame = |rows: usize| Frame {
            preedit: Some(Preedit::plain("shu'ru'fa", 0)),
            rows: (0..rows).map(|index| Row::plain(index, "国国")).collect(),
            footer: Some("1/1".to_owned()),
            ..Frame::default()
        };
        let (_, nine) = renderer.measure_frame(&frame(9), Layout::Vertical, &theme, 1.0);
        let (_, six_v) = renderer.measure_frame(&frame(6), Layout::Vertical, &theme, 1.0);
        assert!(nine > six_v, "竖排 9 个该比 6 个高：{nine} vs {six_v}");
        // 横排的宽度会被「候选框最小宽度」兜住，比较时先关掉它，免得差异被盖住
        let mut wide = Theme::new();
        wide.min_width_pixels = 0.0;
        let (six, _) = renderer.measure_frame(&frame(6), Layout::Horizontal, &wide, 1.0);
        let (three, _) = renderer.measure_frame(&frame(3), Layout::Horizontal, &wide, 1.0);
        assert!(six > three, "横排 6 个该比 3 个宽：{six} vs {three}");
    }

    /// 在线翻译那一行里写了换行就按多行算高度：每多一行窗口就多一条行高。
    #[test]
    fn online_newlines_make_the_window_taller() {
        // 没有系统字体的环境（CI 容器）跳过
        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let theme = Theme::new();
        let height = |online: &str| {
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
        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
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

    /// 脚本内容（`Document`）：元素真画进位图里，命中框按元素给出来（文字按量出来的宽度）。
    #[test]
    fn a_document_paints_its_elements_and_reports_hit_boxes() {
        use crate::color::Color;
        use crate::document::{Document, Element, FontSlot, HitRect, HitShape};

        // 没有系统字体的环境（CI 容器）跳过
        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let theme = Theme::new();
        let document = Document {
            width: 120.0,
            height: 48.0,
            regions: Vec::new(),
            control_names: Vec::new(),
            elements: vec![
                Element::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 120.0,
                    height: 20.0,
                    radius: 0.0,
                    color: Color::rgb(255, 0, 0),
                },
                Element::Text {
                    x: 4.0,
                    y: 24.0,
                    text: "云朵输入法".to_owned(),
                    size: 12.0,
                    slot: FontSlot::Candidate,
                    color: Color::rgb(0, 0, 0),
                    italic: false,
                },
            ],
        };
        let rendered = renderer
            .render_document(&document, &theme, 1.0, Some(&crate::Shadow::panel()), true)
            .expect("画得出来");
        assert_eq!(
            (
                rendered.rendered.content_width,
                rendered.rendered.content_height
            ),
            (120, 48)
        );
        // 阴影留白让内容区从位图里挪开：左上角（留白处）透明，内容区左上角才是那块红矩形
        let margin = rendered.rendered.content_x;
        assert!(margin > 0, "带阴影时内容区不该贴着位图边");
        assert_eq!(rendered.rendered.pixmap.pixels()[0].alpha(), 0, "留白透明");
        let pixel = rendered.rendered.pixmap.pixels()
            [(margin * rendered.rendered.pixmap.width() + margin) as usize];
        assert!(
            pixel.red() > 200 && pixel.green() == 0 && pixel.blue() == 0,
            "内容区左上角落在矩形里"
        );

        assert_eq!(rendered.hits.len(), 2);
        assert_eq!(
            rendered.hits[0],
            Some(HitShape::Rect(HitRect::new(0.0, 0.0, 120.0, 20.0))),
            "直角矩形按外框给命中区域"
        );
        let text = match rendered.hits[1] {
            Some(HitShape::Rect(rect)) => rect,
            ref other => panic!("文字该是外框：{other:?}"),
        };
        assert_eq!(text.x, 4.0);
        assert!(text.width > 0.0, "文字宽度是量出来的：{text:?}");
    }

    /// 静态控件：画出来，但整条 `hits` 都是 `None`（点不到、也不参与 `event.element`）。
    #[test]
    fn static_controls_draw_without_hit_boxes() {
        use crate::color::Color;
        use crate::control::{Label, LabelRun, Panel, StaticImage, StaticShape, TextBlock};
        use crate::document::{Document, Element, FontSlot};

        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let mut image = crate::Pixmap::new(2, 2).unwrap();
        for pixel in image.pixels_mut() {
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(255, 0, 0, 255).unwrap();
        }
        let document = Document {
            width: 120.0,
            height: 80.0,
            regions: Vec::new(),
            control_names: Vec::new(),
            elements: vec![
                Element::Panel(Panel {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 60.0,
                    radius: 6.0,
                    color: Color::rgb(0xE0, 0xE0, 0xE0),
                    border_width: 1.0,
                    fill: Some(Color::rgb(0xE0, 0xE0, 0xE0)),
                    children: vec![
                        Element::Label(Label {
                            x: 4.0,
                            y: 4.0,
                            size: 12.0,
                            slot: FontSlot::Candidate,
                            runs: vec![
                                LabelRun {
                                    text: "云朵".to_owned(),
                                    color: Color::rgb(0, 0, 255),
                                },
                                LabelRun {
                                    text: "输入法".to_owned(),
                                    color: Color::rgb(0, 0, 0),
                                },
                            ],
                        }),
                        Element::TextBlock(TextBlock {
                            x: 4.0,
                            y: 24.0,
                            size: 10.0,
                            slot: FontSlot::Candidate,
                            line_spacing: 2.0,
                            color: Color::rgb(0x33, 0x33, 0x33),
                            text: "第一行\n第二行".to_owned(),
                        }),
                        Element::StaticShape(StaticShape::Rect {
                            x: 70.0,
                            y: 4.0,
                            width: 20.0,
                            height: 10.0,
                            color: Color::rgb(0, 255, 0),
                        }),
                    ],
                }),
                Element::StaticImage(StaticImage {
                    x: 10.0,
                    y: 66.0,
                    width: 20.0,
                    height: 10.0,
                    image: std::sync::Arc::new(image),
                }),
            ],
        };
        let rendered = renderer
            .render_document(&document, &Theme::new(), 1.0, None, false)
            .expect("画得出来");
        assert_eq!(rendered.hits.len(), 2);
        assert!(
            rendered.hits.iter().all(Option::is_none),
            "静态控件没有命中框"
        );
        // 面板右下角只剩底色（子控件都在左上 / 右上）。
        let panel = rendered.rendered.pixmap.pixel(90, 50).unwrap();
        assert_eq!(
            (panel.red(), panel.green(), panel.blue()),
            (0xE0, 0xE0, 0xE0)
        );
        // 静态图像按目标矩形缩放画上去了。
        let image = rendered.rendered.pixmap.pixel(15, 70).unwrap();
        assert_eq!((image.red(), image.green(), image.blue()), (255, 0, 0));
    }

    /// 扁平 SVG 按钮：背景形状 / 区域一致，状态颜色自动切换，命中框是可交互的。
    #[test]
    fn flat_svg_buttons_switch_colors_by_state() {
        use crate::color::Color;
        use crate::control::{ButtonColors, ButtonShape, ButtonState, FlatSvgButton};
        use crate::document::{Document, Element, HitShape};

        let Some(renderer) = FontLibrary::system("zh-CN").ok().map(Renderer::new) else {
            return;
        };
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect x="4" y="4" width="12" height="12"/></svg>"#;
        let button = |state| FlatSvgButton {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 20.0,
            shape: ButtonShape::RoundRect { radius: 3.0 },
            svg: svg.to_owned(),
            svg_size: (20.0, 20.0),
            normal: ButtonColors {
                background: Color::rgb(0, 255, 0),
                svg: Color::rgb(0, 0, 0),
            },
            hovered: ButtonColors {
                background: Color::rgb(255, 0, 0),
                svg: Color::rgb(255, 255, 255),
            },
            pressed: ButtonColors {
                background: Color::rgb(0, 0, 255),
                svg: Color::rgb(255, 255, 0),
            },
            state,
            region_name: "button".to_owned(),
        };
        let render = |state| {
            let document = Document {
                width: 40.0,
                height: 40.0,
                regions: Vec::new(),
                control_names: Vec::new(),
                elements: vec![Element::FlatSvgButton(button(state))],
            };
            renderer
                .render_document(&document, &Theme::new(), 1.0, None, false)
                .expect("画得出来")
        };

        let normal = render(ButtonState::Normal);
        assert_eq!(
            normal.hits[0],
            Some(HitShape::RoundRect {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
                radius: 3.0,
            }),
            "区域就是背景形状"
        );
        let center = normal.rendered.pixmap.pixel(20, 20).unwrap();
        assert_eq!(
            (center.red(), center.green(), center.blue()),
            (0, 0, 0),
            "normal 的 SVG 是黑色"
        );
        let edge = normal.rendered.pixmap.pixel(11, 20).unwrap();
        assert_eq!(
            (edge.red(), edge.green(), edge.blue()),
            (0, 255, 0),
            "normal 的背景是绿色"
        );

        let hovered = render(ButtonState::Hovered);
        let center = hovered.rendered.pixmap.pixel(20, 20).unwrap();
        assert_eq!(
            (center.red(), center.green(), center.blue()),
            (255, 255, 255),
            "hovered 的 SVG 是白色"
        );
        let edge = hovered.rendered.pixmap.pixel(11, 20).unwrap();
        assert_eq!(
            (edge.red(), edge.green(), edge.blue()),
            (255, 0, 0),
            "hovered 的背景是红色"
        );
    }
}
