//! 脚本 `cloudime.ui.measure` 用的量尺。
//!
//! **为什么不借候选窗那个渲染器**：`SharedPainter` 是 `Rc<RefCell<…>>`，只有 UI 线程能用；
//! 从工人线程去用它得跨线程等一轮（卡输入）或加锁抢渲染。所以这里自己建一个渲染器：
//!
//! - **懒加载**：第一次真量才建（字体库要读系统字体，几百毫秒一次）；
//! - **字体与候选窗同源**：字族 / 字号都从同一份 [`RenderSettings`] 算，量出来的点宽与画出来的一致
//!   （同一个整形 / 回退链、同一套行高）；
//! - **不参与缩放**：返回的是**点**（100% 缩放下 1 点 = 1 像素）。窗口渲染时会整体乘 DPI × 缩放，
//!   所以量出来的这个数直接喂 `cloudime.candidate.set_min_width` 就装得下。

use cloudime_render::{FontLibrary, Renderer, Theme, UiFont, system_fonts};
use cloudime_script::MeasureFont;

use super::painter::LOCALE;
use crate::dispatch::RenderSettings;

/// 量尺：一个自己的渲染器 + 建它时用的字体设置。
pub(crate) struct Measurer {
    /// 渲染器（只用来量，不画）。
    renderer: Renderer,

    /// 与候选窗同一套字族 / 行高的主题。
    theme: Theme,

    /// 建它时用的渲染设置：配置改了字体就重建。
    settings: RenderSettings,
}

impl Measurer {
    /// 按配置里的四项字族建一个量尺；字体库加载失败返回 `None`（脚本那边会报「量不了」）。
    fn new(settings: &RenderSettings) -> Option<Self> {
        let mut fonts: Vec<UiFont> = Vec::new();
        for (family, _) in [
            &settings.candidate_font,
            &settings.pinyin_font,
            &settings.item_number_font,
            &settings.translate_font,
        ] {
            let family = family.trim();
            if family.is_empty()
                || fonts
                    .iter()
                    .any(|font| font.family.eq_ignore_ascii_case(family))
            {
                continue;
            }
            fonts.push(UiFont {
                family: family.to_owned(),
                files: system_fonts::family_files(family),
            });
        }
        let library = if fonts.is_empty() {
            FontLibrary::system(LOCALE)
        } else {
            FontLibrary::with_fonts(LOCALE, &fonts)
        };
        let library = library.ok()?;
        let mut theme = Theme::new();
        theme.set_fonts(
            (family(&settings.pinyin_font.0), settings.pinyin_font.1),
            (
                family(&settings.candidate_font.0),
                settings.candidate_font.1,
            ),
            (
                family(&settings.item_number_font.0),
                settings.item_number_font.1,
            ),
            (
                family(&settings.translate_font.0),
                settings.translate_font.1,
            ),
        );
        Some(Self {
            renderer: Renderer::new(library),
            theme,
            settings: settings.clone(),
        })
    }

    /// 这次要量的字体设置还对不对得上（配置改了字体就重建）。
    fn is_stale(&self, settings: &RenderSettings) -> bool {
        self.settings != *settings
    }

    /// 量一段文字，返回（宽, 高）单位**点**。
    fn measure(&mut self, text: &str, font: MeasureFont) -> (f32, f32) {
        let (spec, family) = match font {
            MeasureFont::Pinyin => (self.theme.pinyin_font, self.theme.pinyin_family.clone()),
            MeasureFont::Candidate => (self.theme.text_font, self.theme.text_family.clone()),
            MeasureFont::ItemNumber => (self.theme.index_font, self.theme.index_family.clone()),
            MeasureFont::Translate => (
                self.theme.translate_font,
                self.theme.translate_family.clone(),
            ),
        };
        self.renderer.measure_font(text, &spec, family.as_ref())
    }
}

/// 字族名：空白当没写（用系统界面字体）。
fn family(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// 量尺的句柄：懒加载 + 字体设置变了自动重建。Server 把它装进脚本运行时（`cloudime.ui.measure`）。
#[derive(Default)]
pub(crate) struct SharedMeasurer {
    /// `None` = 还没建（或上次字体库加载失败）。
    measurer: Option<Measurer>,
}

impl SharedMeasurer {
    /// 量一段文字；没建就建（按 `settings`），字体设置变了就重建。量不了返回 `None`。
    pub(crate) fn measure(
        &mut self,
        settings: &RenderSettings,
        text: &str,
        font: MeasureFont,
    ) -> Option<(f32, f32)> {
        if self
            .measurer
            .as_ref()
            .is_none_or(|measurer| measurer.is_stale(settings))
        {
            self.measurer = Measurer::new(settings);
        }
        Some(self.measurer.as_mut()?.measure(text, font))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::RouterConfig;

    /// 量尺真的在量：宽度 > 0，而且字多的更宽（保证不是恒等于 0 的假实现）。
    #[test]
    fn the_measurer_measures_text() {
        let settings = RouterConfig::default().render_settings();
        let mut measurer = SharedMeasurer::default();
        let (short, height) = measurer
            .measure(&settings, "abc", MeasureFont::Translate)
            .expect("量尺该建得起来（读系统字体）");
        assert!(short > 0.0 && height > 0.0, "{short}x{height}");

        let (long, _) = measurer
            .measure(&settings, "abcdefgh", MeasureFont::Translate)
            .unwrap();
        assert!(long > short, "{long} 该比 {short} 宽");

        // 换字体设置（字号放大）会重建量尺：同一段文字量出来更宽
        let mut bigger = settings;
        bigger.translate_font.1 *= 2.0;
        let (scaled, _) = measurer
            .measure(&bigger, "abc", MeasureFont::Translate)
            .unwrap();
        assert!(scaled > short, "字号翻倍后 {scaled} 该比 {short} 宽");
    }
}
