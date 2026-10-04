//! 云朵渲染器在 Windows 壳里的落地：字体库 + 渲染器一份，候选窗口与状态条共用（字形缓存共享）。
//! 字体库加载失败时没有它（`None`），两个窗口不绘制也不显示，并记日志。

use std::cell::RefCell;
use std::rc::Rc;

use cloudime_platform::LayoutMode;
use cloudime_render::{
    FontLibrary, Frame, Layout, Rendered, RenderedStatus, Renderer, Shadow, StatusCell, Theme,
    UiFont, system_fonts,
};

use crate::dispatch::RenderSettings;

/// 按字族名加载字体时的 locale（中日同形字选哪家的字形）。
const LOCALE: &str = "zh-CN";

/// UI 线程上共享的渲染器；`None` = 字体库加载失败，渲染不出来。
pub(super) type SharedPainter = Rc<RefCell<Option<Painter>>>;

pub(super) struct Painter {
    /// 渲染器（字体库随它）。
    renderer: Renderer,

    /// 主题：三项字体与竖排最小宽度按配置改过，其余用缺省。
    theme: Theme,

    /// 建它时用的渲染设置，没变就不重建。
    settings: RenderSettings,
}

impl Painter {
    /// 按配置里的三个字族建字体库（同名字族只加载一次，第一个非空的当界面字体）；
    /// 字体库加载失败返回 `None`，两个窗口不绘制。
    fn new(settings: &RenderSettings) -> Option<Self> {
        let started = std::time::Instant::now();
        let mut fonts: Vec<UiFont> = Vec::new();
        for (family, _) in [
            &settings.candidate_font,
            &settings.pinyin_font,
            &settings.item_number_font,
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
        let library = match library {
            Ok(library) => library,
            Err(error) => {
                tracing::warn!(%error, "渲染器字体库加载失败，候选窗口与状态条将不显示");
                return None;
            }
        };
        tracing::info!(
            elapsed = ?started.elapsed(),
            font = library.ui_family(),
            "候选窗口与状态条使用云朵渲染器"
        );
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
        );
        theme.min_width_pixels = settings.min_width_pixels;
        Some(Self {
            renderer: Renderer::new(library),
            theme,
            settings: settings.clone(),
        })
    }

    /// 渲染设置变了就重建渲染器。
    pub(super) fn configure(shared: &SharedPainter, settings: &RenderSettings) {
        let mut painter = shared.borrow_mut();
        if painter
            .as_ref()
            .is_some_and(|painter| painter.settings == *settings)
        {
            return;
        }
        *painter = Self::new(settings);
    }

    /// 画一帧候选窗口；`dpi` 96 为 100%。失败记日志返回 `None`，调用方不显示候选窗口。
    pub(super) fn render_frame(
        &mut self,
        frame: &Frame,
        layout: LayoutMode,
        dpi: u32,
    ) -> Option<Rendered> {
        let layout = match layout {
            LayoutMode::Vertical => Layout::Vertical,
            LayoutMode::Horizontal => Layout::Horizontal,
        };
        let started = std::time::Instant::now();
        let rendered = self
            .renderer
            .render(frame, layout, &self.theme, scale(dpi), Some(&SHADOW))
            .inspect_err(|error| tracing::warn!(%error, "候选窗渲染失败"))
            .ok()?;
        tracing::debug!(
            elapsed = ?started.elapsed(),
            width = rendered.content_width,
            height = rendered.content_height,
            "候选窗位图已画"
        );
        Some(rendered)
    }

    /// 画状态条。
    pub(super) fn render_status(
        &mut self,
        cells: &[StatusCell],
        dpi: u32,
    ) -> Option<RenderedStatus> {
        self.renderer
            .render_status(cells, &self.theme, scale(dpi), Some(&SHADOW))
            .inspect_err(|error| tracing::warn!(%error, "状态条渲染失败"))
            .ok()
    }
}

/// 配置里的字族名 → 渲染器要的「有就用、空就没有」。
fn family(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// 两个窗口都用渲染器画阴影（分层窗口没有系统阴影）。
const SHADOW: Shadow = Shadow::panel();

/// 点 → 像素的倍数。
fn scale(dpi: u32) -> f32 {
    dpi.max(96) as f32 / 96.0
}
