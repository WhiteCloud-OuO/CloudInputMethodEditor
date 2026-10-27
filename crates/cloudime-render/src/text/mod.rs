//! 文字测绘：cosmic-text 整形 + swash 栅格，单行、像素坐标；普通字形走覆盖率遮罩，彩色 emoji 走 RGBA 位图。

mod size;
mod style;

use std::collections::HashMap;

use cosmic_text::fontdb::{Family, ID, Query, Stretch, Style, Weight};
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

use crate::canvas::Canvas;
use crate::fonts::{FontLibrary, Trak, UI_FAMILY};
pub use size::TextSize;
pub use style::TextStyle;

pub struct TextPainter {
    /// 字体库与回退链。
    font_system: FontSystem,

    /// 字形位图缓存（按字体、字号、亚像素位移）。
    cache: SwashCache,

    /// 量过的宽度：同一段字 / 同一套样式在布局阶段会被量好几遍（一次出帧至少两遍），存下来省整形。
    widths: HashMap<WidthKey, TextSize>,

    /// 复用的单行缓冲。
    buffer: Buffer,

    /// 每张字体的字距表（没有的记 `None`），按字形所用字体各查各的。
    tracking: HashMap<ID, Option<Trak>>,

    /// 覆盖率 gamma 查找表，按 gamma 值缓存。
    gamma_tables: HashMap<u32, Box<[u8; 256]>>,

    /// 某个字族本身能不能显示某个字符（按字族 + 字符缓存）。
    supports: HashMap<(Option<String>, u16, char), bool>,
}

impl TextPainter {
    pub(crate) fn new(library: FontLibrary) -> Self {
        let mut font_system = library.into_font_system();
        let buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 19.0));
        Self {
            font_system,
            cache: SwashCache::new(),
            buffer,
            tracking: HashMap::new(),
            gamma_tables: HashMap::new(),
            widths: HashMap::new(),
            supports: HashMap::new(),
        }
    }

    /// 光学字号（点）：SF 这类带 `opsz` 轴的字体在小字号应用文本视觉尺寸，这里要显式给。
    /// 现在是整个画笔一个值（cosmic-text 的字体实例缓存没按它分键），候选窗几种字号都在 20 pt 以下，落到同一档。
    pub(crate) fn set_optical_size(&mut self, points: Option<f32>) {
        self.font_system.set_optical_size(points);
    }

    /// 一段文字的宽高（像素）。高度就是行高。
    pub(crate) fn measure(&mut self, text: &str, style: &TextStyle) -> TextSize {
        let key = WidthKey::new(text, style);
        if let Some(size) = self.widths.get(&key) {
            return *size;
        }
        let size = self.measure_uncached(text, style);
        if self.widths.len() >= WIDTH_CACHE_ENTRIES {
            self.widths.clear();
        }
        self.widths.insert(key, size);
        size
    }

    /// 真去整形 + 量（[`Self::measure`] 的缓存没命中时走这里）。
    fn measure_uncached(&mut self, text: &str, style: &TextStyle) -> TextSize {
        self.shape(text, style);
        let mut width = 0.0_f32;
        for run in self.buffer.layout_runs() {
            let tracked: f32 = run
                .glyphs
                .iter()
                .map(|glyph| {
                    tracking_px(&self.font_system, &mut self.tracking, glyph.font_id, style)
                })
                .sum();
            width = width.max(run.line_w + tracked);
        }
        TextSize {
            width,
            height: style.line_height,
        }
    }

    /// 把文字画到 `(x, y)`，`y` 是行框顶边。返回宽度。
    pub(crate) fn draw(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> f32 {
        self.shape(text, style);
        let mut width = 0.0_f32;
        let mut line: Option<(f32, f32)> = None;
        for run in self.buffer.layout_runs() {
            let baseline = y + run.line_y;
            // 每个字形画完把它那份字距累加到后面所有字形的 x 上
            let mut tracked = 0.0_f32;
            for glyph in run.glyphs {
                let physical = glyph.physical((x + tracked, y), 1.0);
                tracked += tracking_px(&self.font_system, &mut self.tracking, glyph.font_id, style);
                let Some(image) = self
                    .cache
                    .get_image(&mut self.font_system, physical.cache_key)
                else {
                    continue;
                };
                let gx = physical.x + image.placement.left;
                let gy = run.line_y.round() as i32 + physical.y - image.placement.top;
                let (w, h) = (image.placement.width, image.placement.height);
                match image.content {
                    SwashContent::Mask => {
                        let table = gamma_table(&mut self.gamma_tables, style.gamma);
                        let data: Vec<u8> = image.data.iter().map(|&c| table[c as usize]).collect();
                        canvas.blend_mask(gx, gy, w, h, &data, style.color);
                    }
                    SwashContent::Color => canvas.blend_rgba(gx, gy, w, h, &image.data),
                    // 没有申请亚像素格式，不会出现
                    SwashContent::SubpixelMask => {}
                }
            }
            width = width.max(run.line_w + tracked);
            // 删除线穿小写字母中部；下划线贴在基线下面一点（辅助码里已敲的那几位用它）
            if style.strike {
                line = Some((baseline - style.size * 0.3, run.line_w + tracked));
            } else if style.underline {
                line = Some((baseline + style.size * 0.12, run.line_w + tracked));
            }
        }
        if let Some((line_y, line_w)) = line {
            let thickness = (style.size / 14.0).max(1.0);
            canvas.fill_rect(x, line_y, line_w, thickness, style.color);
        }
        width
    }

    /// 这个字族本身能不能显示 `ch`：查字体的 cmap，`0` 是缺字（不把系统回退算进来 —— 调用方要据此
    /// 决定什么时候换补充字体）。空白字符直接算作能显示。
    pub(crate) fn supports_char(&mut self, ch: char, style: &TextStyle) -> bool {
        if ch.is_whitespace() {
            return true;
        }
        let key = (style.family.clone(), style.weight, ch);
        if let Some(supported) = self.supports.get(&key) {
            return *supported;
        }
        let family = style
            .family
            .as_deref()
            .map(Family::Name)
            .unwrap_or(Family::SansSerif);
        let weight = Weight(style.weight);
        let query = Query {
            families: std::slice::from_ref(&family),
            weight,
            stretch: Stretch::Normal,
            style: if style.italic {
                Style::Italic
            } else {
                Style::Normal
            },
        };
        let supported = self
            .font_system
            .db()
            .query(&query)
            .and_then(|id| self.font_system.get_font(id, weight))
            .is_some_and(|font| font.as_swash().charmap().map(ch) != 0);
        self.supports.insert(key, supported);
        supported
    }

    /// 每个字形用的字族名（相邻相同的合并），拿来核对中日字形与 emoji 回退到了哪家字体。
    pub(crate) fn trace_families(&mut self, text: &str, style: &TextStyle) -> Vec<String> {
        self.shape(text, style);
        let mut names: Vec<String> = Vec::new();
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let name = self
                    .font_system
                    .db()
                    .face(glyph.font_id)
                    .and_then(|face| face.families.first().map(|(n, _)| n.clone()))
                    .unwrap_or_else(|| "?".to_owned());
                if names.last() != Some(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    fn shape(&mut self, text: &str, style: &TextStyle) {
        let attrs = match &style.family {
            Some(family) => Attrs::new().family(Family::Name(family)),
            None => Attrs::new().family(UI_FAMILY),
        };
        let attrs = attrs.weight(Weight(style.weight));
        let attrs = if style.italic {
            attrs.style(Style::Italic)
        } else {
            attrs
        };
        let attrs = attrs.color(style.color.to_cosmic());
        self.buffer
            .set_metrics(Metrics::new(style.size, style.line_height));
        self.buffer.set_size(None, None);
        self.buffer.set_text(text, &attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.font_system, false);
    }
}

/// 量宽缓存的键：只放会影响宽度的东西（颜色不影响排版）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WidthKey {
    text: String,
    family: Option<String>,
    size: u32,
    line_height: u32,
    italic: bool,
    weight: u16,
}

impl WidthKey {
    fn new(text: &str, style: &TextStyle) -> Self {
        Self {
            text: text.to_owned(),
            family: style.family.clone(),
            size: style.size.to_bits(),
            line_height: style.line_height.to_bits(),
            italic: style.italic,
            weight: style.weight,
        }
    }
}

/// 量宽缓存留多少条（超过就整张清掉，别让它无限长）。
const WIDTH_CACHE_ENTRIES: usize = 512;

/// 某张字体在这个字号下每个字形要加的间距（像素）；第一次用到时解析它的 `trak` 表。
fn tracking_px(
    font_system: &FontSystem,
    cache: &mut HashMap<ID, Option<Trak>>,
    font_id: ID,
    style: &TextStyle,
) -> f32 {
    let trak = cache.entry(font_id).or_insert_with(|| {
        font_system
            .db()
            .with_face_data(font_id, Trak::parse)
            .flatten()
    });
    trak.as_ref()
        .map_or(0.0, |trak| trak.tracking_em(style.points) * style.size)
}

/// 覆盖率 → gamma 校正后的覆盖率。
fn gamma_table(cache: &mut HashMap<u32, Box<[u8; 256]>>, gamma: f32) -> &[u8; 256] {
    cache.entry(gamma.to_bits()).or_insert_with(|| {
        let mut table = [0u8; 256];
        for (i, out) in table.iter_mut().enumerate() {
            *out = ((i as f32 / 255.0).powf(gamma) * 255.0).round() as u8;
        }
        Box::new(table)
    })
}
