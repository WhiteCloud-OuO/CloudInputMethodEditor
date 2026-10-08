//! 云朵渲染器在 Windows 壳里的落地：字体库 + 渲染器一份，候选窗口与状态条共用（字形缓存共享）。
//! 字体库加载失败时没有它（`None`），两个窗口不绘制也不显示，并记日志。

use std::cell::RefCell;
use std::rc::Rc;

use cloudime_platform::LayoutMode;
use cloudime_render::{
    Color, FontLibrary, Frame, Layout, Rendered, RenderedStatus, Renderer, Shadow, StatusCell,
    Theme, UiFont, system_fonts,
};

use crate::dispatch::RenderSettings;

/// 按字族名加载字体时的 locale（中日同形字选哪家的字形）。
pub(super) const LOCALE: &str = "zh-CN";

/// UI 线程上共享的渲染器；`None` = 字体库加载失败，渲染不出来。
pub(super) type SharedPainter = Rc<RefCell<Option<Painter>>>;

pub(super) struct Painter {
    /// 渲染器（字体库随它）。
    renderer: Renderer,

    /// 主题：三项字体与竖排最小宽度按配置改过，颜色按当前主题文件改过，其余用缺省。
    theme: Theme,

    /// 候选窗口 / 悬浮工具栏 / 状态切换提示各自的阴影（颜色来自主题）。
    candidate_shadow: Shadow,
    bar_shadow: Shadow,
    tip_shadow: Shadow,

    /// 建它时用的渲染设置，没变就不重建。
    settings: RenderSettings,
}

/// 状态条 / 状态切换提示用哪一套主题色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusKind {
    /// 悬浮工具栏。
    Bar,

    /// 状态切换提示。
    Tip,
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
            (
                family(&settings.translate_font.0),
                settings.translate_font.1,
            ),
        );
        theme.min_width_pixels = settings.min_width_pixels;
        theme.max_cell_width = settings.max_cell_width;
        apply_theme_colors(&mut theme, &settings.theme);
        let panel = Shadow::panel();
        Some(Self {
            renderer: Renderer::new(library),
            candidate_shadow: Shadow {
                color: theme.colors.shadow,
                ..panel
            },
            bar_shadow: Shadow {
                color: theme.colors.bar_shadow,
                ..panel
            },
            tip_shadow: Shadow {
                color: theme.colors.tip_shadow,
                ..panel
            },
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
            .render(
                frame,
                layout,
                &self.theme,
                scale(dpi),
                Some(&self.candidate_shadow),
            )
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

    /// 画状态条或状态切换提示：`kind` 决定用哪一套背景 / 图标 / 阴影色。
    pub(super) fn render_status(
        &mut self,
        cells: &[StatusCell],
        dpi: u32,
        kind: StatusKind,
    ) -> Option<RenderedStatus> {
        let (background, icon, shadow) = match kind {
            StatusKind::Bar => (
                self.theme.colors.bar_background,
                self.theme.colors.bar_icon,
                &self.bar_shadow,
            ),
            StatusKind::Tip => (
                self.theme.colors.tip_background,
                self.theme.colors.tip_icon,
                &self.tip_shadow,
            ),
        };
        self.renderer
            .render_status(
                cells,
                &self.theme,
                scale(dpi),
                Some(shadow),
                background,
                icon,
            )
            .inspect_err(|error| tracing::warn!(%error, "状态条渲染失败"))
            .ok()
    }
}

/// 把主题文件里的 21 个颜色套到渲染主题上（其余注解色保持缺省）。
fn apply_theme_colors(theme: &mut Theme, file: &cloudime_platform::ThemeFile) {
    fn color(value: cloudime_platform::ThemeColor) -> Color {
        Color::rgba(value.r, value.g, value.b, value.a)
    }
    let candidate = file.candidate;
    theme.colors.background = color(candidate.background);
    theme.colors.highlight = color(candidate.highlight);
    theme.colors.shadow = color(candidate.shadow);
    theme.colors.pinyin = color(candidate.pinyin);
    theme.colors.pinyin_caret = color(candidate.pinyin_caret);
    theme.colors.highlight_text = color(candidate.highlight_text);
    theme.colors.text = color(candidate.text);
    theme.colors.index = color(candidate.index);
    theme.colors.highlight_index = color(candidate.highlight_index);
    theme.colors.footer = color(candidate.page_number);
    theme.colors.badge = color(candidate.badge);
    theme.colors.translate_meta = color(candidate.translate_meta);
    theme.colors.translate_learned = color(candidate.translate_learned);
    theme.colors.translate_fresh = color(candidate.translate_fresh);
    theme.colors.extra = color(candidate.extra);
    theme.colors.bar_background = color(file.bar.background);
    theme.colors.bar_icon = color(file.bar.icon);
    theme.colors.bar_shadow = color(file.bar.shadow);
    theme.colors.tip_background = color(file.tip.background);
    theme.colors.tip_icon = color(file.tip.icon);
    theme.colors.tip_shadow = color(file.tip.shadow);
}

/// 配置里的字族名 → 渲染器要的「有就用、空就没有」。
fn family(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// 点 → 像素的倍数。候选窗把滚轮缩放也折进这个 DPI 里，所以这里不能再拿 96 当下限
/// （缩小时倍数会小于 1），只挡住 0。
fn scale(dpi: u32) -> f32 {
    dpi.max(1) as f32 / 96.0
}
