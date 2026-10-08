//! 「主题」页顶部的预览：把当前草稿直接喂给云朵渲染器，出**真位图**再贴出来 —— 真阴影（含主题的
//! 阴影色）、真正尺寸、真 SVG 图标并按主题的图标色染色，与真实窗口像素一致。
//!
//! 与真实窗口同源：同一个 `cloudime-render`、同一套 `Theme`（字体来自「候选」页的配置、颜色来自草稿），
//! 所以预览里看到的和候选窗 / 悬浮工具栏 / 状态切换提示上看到的是同一份渲染结果。

use cloudime_platform::{CandidateConfig, ThemeColor, ThemeFile};
use cloudime_render::{
    Color, FontLibrary, Frame, Layout, Preedit, Renderer, Row, Shadow, StatusCell, Theme,
    TipSegment, Tone, UiFont, system_fonts,
};
use windows_reactor::*;

use crate::panel::controls::note;

/// 按字族名加载字体时的 locale（中日同形字选哪家的字形）；与 Server 那边一致。
const LOCALE: &str = "zh-CN";

/// 预览里候选窗口的最小宽度（点）。用户要求固定成 200，不跟配置走。
const MIN_WIDTH: f32 = 200.0;

/// 预览里拼音行的内容。
const PINYIN: &str = "yun'duo'shu'ru'fa";

/// 悬浮工具栏那一排图标（一种状态一份：中 / 半角 / 中文标点 / 简，与真机缺省状态一致）。
const TOOLBAR: [&str; 7] = [
    include_str!("../../../../../server/src/ui/status/icons/ch.svg"),
    include_str!("../../../../../server/src/ui/status/icons/half.svg"),
    include_str!("../../../../../server/src/ui/status/icons/ch_marks.svg"),
    include_str!("../../../../../server/src/ui/status/icons/simp_ch.svg"),
    include_str!("../../../../../server/src/ui/status/icons/spec_chars.svg"),
    include_str!("../../../../../server/src/ui/status/icons/widgets.svg"),
    include_str!("../../../../../server/src/ui/status/icons/options.svg"),
];

/// 状态切换提示只显示前四个状态按钮。
const TIP: [&str; 4] = [TOOLBAR[0], TOOLBAR[1], TOOLBAR[2], TOOLBAR[3]];

/// 三块预览并排。
pub(super) fn view(file: &ThemeFile, config: &CandidateConfig) -> View {
    let Some(mut renderer) = renderer(config) else {
        return note(
            "预览需要云朵渲染器，但字体库没加载起来（已记日志）；改颜色仍会保存到主题文件。",
        );
    };
    let theme = theme(file, config);
    let candidate_shadow = Shadow {
        color: theme.colors.shadow,
        ..Shadow::panel()
    };
    let bar_shadow = Shadow {
        color: theme.colors.bar_shadow,
        ..Shadow::panel()
    };
    let tip_shadow = Shadow {
        color: theme.colors.tip_shadow,
        ..Shadow::panel()
    };
    let Ok(candidate) = renderer.render(
        &frame(),
        Layout::Vertical,
        &theme,
        1.0,
        Some(&candidate_shadow),
    ) else {
        return note("预览渲染失败（候选窗口），已记日志。");
    };
    let Ok(bar) = renderer.render_status(
        &cells(&TOOLBAR),
        &theme,
        1.0,
        Some(&bar_shadow),
        theme.colors.bar_background,
        theme.colors.bar_icon,
    ) else {
        return note("预览渲染失败（悬浮工具栏），已记日志。");
    };
    let Ok(tip) = renderer.render_status(
        &cells(&TIP),
        &theme,
        1.0,
        Some(&tip_shadow),
        theme.colors.tip_background,
        theme.colors.tip_icon,
    ) else {
        return note("预览渲染失败（状态切换提示），已记日志。");
    };
    StackPanel::new().spacing(8.0).children((
        TextBlock::new()
            .text("预览")
            .font_weight(FontWeight::SEMI_BOLD),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(16.0)
            .children((
                image(&candidate.pixmap),
                image(&bar.rendered.pixmap),
                image(&tip.rendered.pixmap),
            )),
    ))
}

/// 位图 → `Image`：编码成 PNG 交给 `EncodedImage`，`Stretch::None` 按原始像素 1:1 贴出来。
fn image(pixmap: &cloudime_render::Pixmap) -> View {
    match pixmap.encode_png() {
        Ok(png) => Image::new()
            .source_data(EncodedImage::new(png))
            .stretch(Stretch::None)
            .vertical_alignment(VerticalAlignment::Top)
            .into(),
        Err(error) => note(&format!("预览位图编码失败：{error}")),
    }
}

/// 预览用的一帧：拼音行 + 两条候选（角标「句」/「造」）+ 高亮 + 页码 + 译文 Tip + 在线那一行。
fn frame() -> Frame {
    Frame {
        preedit: Some(Preedit::plain(PINYIN, PINYIN.chars().count())),
        rows: vec![
            Row {
                index: "1".to_owned(),
                text: "云朵输入法".to_owned(),
                annotation: Vec::new(),
                badge: Some("句".to_owned()),
            },
            Row {
                index: "2".to_owned(),
                text: "云朵".to_owned(),
                annotation: Vec::new(),
                badge: Some("造".to_owned()),
            },
        ],
        highlighted: Some(0),
        footer: Some("1/2".to_owned()),
        tip: Some(vec![
            TipSegment::new("n.", Tone::TranslateMeta, true),
            TipSegment::new("Cloud IME", Tone::TranslateFresh, false),
        ]),
        online: Some(vec![TipSegment::new("云翻译出错。", Tone::Online, false)]),
        ..Frame::default()
    }
}

fn cells(sources: &[&str]) -> Vec<StatusCell> {
    sources.iter().map(|svg| StatusCell::icon(*svg)).collect()
}

/// 按主题文件 + 配置里的四项字体拼一份渲染主题（与 Server 的 `Painter::new` 同一套）。
fn theme(file: &ThemeFile, config: &CandidateConfig) -> Theme {
    let mut theme = Theme::new();
    theme.set_fonts(
        (family(&config.pinyin_font.family), config.pinyin_font.size),
        (
            family(&config.candidate_font.family),
            config.candidate_font.size,
        ),
        (
            family(&config.item_number_font.family),
            config.item_number_font.size,
        ),
        (
            family(&config.translate_font.family),
            config.translate_font.size,
        ),
    );
    theme.min_width_pixels = MIN_WIDTH;
    apply_colors(&mut theme, file);
    theme
}

/// 按配置里的四项字族建渲染器；字体库加载失败返回 `None`（记一条日志）。
fn renderer(config: &CandidateConfig) -> Option<Renderer> {
    let mut fonts: Vec<UiFont> = Vec::new();
    for font in [
        &config.pinyin_font,
        &config.candidate_font,
        &config.item_number_font,
        &config.translate_font,
    ] {
        let family = font.family.trim();
        if family.is_empty()
            || fonts
                .iter()
                .any(|item| item.family.eq_ignore_ascii_case(family))
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
    match library {
        Ok(library) => Some(Renderer::new(library)),
        Err(error) => {
            crate::log::warn(format!("预览的字体库加载失败: {error}"));
            None
        }
    }
}

/// 配置里的字族名 → 渲染器要的「有就用、空就没有」。
fn family(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// 主题文件的 21 个颜色 → 渲染配色。
///
/// 与 `apps/windows/server/src/ui/painter/mod.rs::apply_theme_colors` 是同一份映射：两个壳依赖的东西
/// 不同（Server 拿 `RenderSettings`、这里只有 `ThemeFile`），谁也不能反过来依赖谁，所以各留一份；
/// **改键名或加颜色时两处一起改**。
fn apply_colors(theme: &mut Theme, file: &ThemeFile) {
    fn color(value: ThemeColor) -> Color {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 预览这一路跑得通：拼主题 → 渲染三块 → 编码 PNG（真 PNG 的魔数）。
    /// 字体库建不起来（机器上没配那几个字族）就跳过 —— 与 `cloudime-render` 那边测试同一套写法。
    #[test]
    fn the_preview_renders_png_bitmaps() {
        let config = CandidateConfig::default();
        let Some(mut renderer) = renderer(&config) else {
            return;
        };
        let theme = theme(&ThemeFile::default(), &config);
        let shadow = Shadow {
            color: theme.colors.shadow,
            ..Shadow::panel()
        };
        let candidate = renderer
            .render(&frame(), Layout::Vertical, &theme, 1.0, Some(&shadow))
            .expect("候选窗那一块应该画得出来");
        assert!(candidate.pixmap.width() > 40 && candidate.pixmap.height() > 20);
        let png = candidate.pixmap.encode_png().expect("编码 PNG 应该成功");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let bar = renderer
            .render_status(
                &cells(&TOOLBAR),
                &theme,
                1.0,
                Some(&shadow),
                theme.colors.bar_background,
                theme.colors.bar_icon,
            )
            .expect("工具栏那一块应该画得出来");
        assert!(bar.rendered.pixmap.width() > 40 && bar.rendered.pixmap.height() > 20);

        let tip = renderer
            .render_status(
                &cells(&TIP),
                &theme,
                1.0,
                Some(&shadow),
                theme.colors.tip_background,
                theme.colors.tip_icon,
            )
            .expect("提示那一块应该画得出来");
        assert!(tip.rendered.pixmap.width() > 20);
    }
}
