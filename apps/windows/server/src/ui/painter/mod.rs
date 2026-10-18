//! 云朵渲染器在 Windows 壳里的落地：字体库 + 渲染器一份，候选窗口与状态条共用（字形缓存共享）。
//! 字体库加载失败时没有它（`None`），两个窗口不绘制也不显示，并记日志。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use cloudime_platform::LayoutMode;
use cloudime_render::{
    Color, FontLibrary, FontSlot, Frame, Layout, Preedit, Rendered, RenderedStatus, Renderer, Row,
    Shadow, StatusCell, Theme, UiFont, system_fonts,
};

use crate::dispatch::RenderSettings;

/// 按字族名加载字体时的 locale（中日同形字选哪家的字形）。
pub(super) const LOCALE: &str = "zh-CN";

/// 每建一次 [`Painter`] 就 +1：窗口模板靠它判断「渲染器（主题 / 字体）换了，得重画」。
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// UI 线程上共享的渲染器；`None` = 字体库加载失败，渲染不出来。
pub(super) type SharedPainter = Rc<RefCell<Option<Painter>>>;

pub(super) struct Painter {
    /// 渲染器（字体库随它）。
    renderer: Renderer,

    /// 这一份是第几次建的（见 [`NEXT_GENERATION`]）。
    generation: u64,

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
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
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

    /// 这一份渲染器是第几次建的（见 [`NEXT_GENERATION`]）。
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// 某个字体槽位的字族与字号（点）：系统 EDIT 的字体用。
    pub(super) fn text_font(&self, slot: FontSlot) -> (Option<String>, f32) {
        match slot {
            FontSlot::Candidate => (self.theme.text_family.clone(), self.theme.text_font.size),
            FontSlot::Pinyin => (
                self.theme.pinyin_family.clone(),
                self.theme.pinyin_font.size,
            ),
            FontSlot::ItemNumber => (self.theme.index_family.clone(), self.theme.index_font.size),
            FontSlot::Translate => (
                self.theme.translate_family.clone(),
                self.theme.translate_font.size,
            ),
        }
    }

    /// 画一块空面板（内容区尺寸给死）：给「窗口模板」用（版面还没生成时）。
    pub(super) fn render_panel(&mut self, content: (i32, i32), dpi: u32) -> Option<Rendered> {
        self.renderer
            .render_panel(
                content.0 as f32,
                content.1 as f32,
                &self.theme,
                scale(dpi),
                Some(&self.candidate_shadow),
            )
            .inspect_err(|error| tracing::warn!(%error, "窗口模板渲染失败"))
            .ok()
    }

    /// 画一块「键位映射参考」面板（版面给进来，内容区尺寸由版面自己定）：给「窗口模板」用。
    pub(super) fn render_key_map(
        &mut self,
        layout: &cloudime_render::KeyMapLayout,
        cell: f32,
        dpi: u32,
    ) -> Option<Rendered> {
        self.renderer
            .render_key_map(
                layout,
                cell,
                &self.theme,
                scale(dpi),
                Some(&self.candidate_shadow),
            )
            .inspect_err(|error| tracing::warn!(%error, "辅助窗口渲染失败"))
            .ok()
    }

    /// 画一份**脚本内容**（面板 + 元素表）；`shadow` = 要不要与候选窗同源的那层阴影。
    /// 给「辅助窗口模板」与脚本独立窗口用。
    pub(super) fn render_document(
        &mut self,
        document: &cloudime_render::Document,
        dpi: u32,
        shadow: bool,
        background: bool,
    ) -> Option<cloudime_render::RenderedDocument> {
        self.renderer
            .render_document(
                document,
                &self.theme,
                scale(dpi),
                shadow.then_some(&self.candidate_shadow),
                background,
            )
            .inspect_err(|error| tracing::warn!(%error, "脚本窗口渲染失败"))
            .ok()
    }

    /// 「候选窗口装 9 个候选项」那个内容高度（**点**）：键位映射参考的缺省目标高度
    /// （竖排 / 横排都用它当底，见窗口模板的 `key_map_height`）。
    pub(super) fn key_map_height_nine(&mut self, dpi: u32) -> f32 {
        const ROWS: usize = 9;
        let (_, height) = self.candidate_size(Layout::Vertical, ROWS, dpi);
        height as f32 / scale(dpi)
    }

    /// 按「候选窗口装 `rows` 个候选、每个两个字」的形状量内容区尺寸（像素，不含阴影边）。
    ///
    /// 给「窗口模板」算默认大小用：竖排量 9 个候选的高度、横排量 6 个两字候选的宽度。
    pub(super) fn candidate_size(&mut self, layout: Layout, rows: usize, dpi: u32) -> (i32, i32) {
        let frame = Frame {
            preedit: Some(Preedit::plain("shu'ru'fa", 0)),
            rows: (0..rows).map(|index| Row::plain(index, "国国")).collect(),
            footer: Some("1/1".to_owned()),
            ..Frame::default()
        };
        let (width, height) = self
            .renderer
            .measure_frame(&frame, layout, &self.theme, scale(dpi));
        (width as i32, height as i32)
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
