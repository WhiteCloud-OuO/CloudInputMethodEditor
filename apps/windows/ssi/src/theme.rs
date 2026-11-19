//! 从输入法当前主题生成 `cloud-ui` 的配色。
//!
//! 与组字工具同一套映射：读 `%APPDATA%\CloudIME\config.toml` 的 `[theme] curr_theme`，
//! 找到主题 JSON 就把候选窗口的颜色搬过来；找不到用深色主题。

use cloud_ui::Theme;
use cloudime_platform::{Config, ThemeColor, ThemeFile, dirs, find_theme, resources};
use cloudime_render::Color;

/// 读当前输入法主题并映射成工具主题。
pub fn load() -> Theme {
    let bundled = resources::bundled_root();
    let config = dirs::config_path()
        .and_then(|path| Config::load(&path).ok())
        .unwrap_or_default();
    let name = config.theme.curr_theme;
    let Some(entry) = find_theme(bundled.as_deref(), &name) else {
        return Theme::dark();
    };
    ThemeFile::load(&entry.path).map_or_else(|_| Theme::dark(), |file| build(&file))
}

/// 映射规则：候选窗口底色当整窗背景，正文 / 页码当文字，`extra` 当强调色。
pub fn build(file: &ThemeFile) -> Theme {
    let mut theme = Theme::dark();
    let candidate = file.candidate;
    let background = opaque(candidate.background);
    let text = opaque(candidate.text);

    theme.background = background;
    theme.surface = mix(background, text, 0.06);
    theme.surface_alt = mix(background, text, 0.12);
    theme.border = mix(background, text, 0.25);
    theme.text = color(candidate.text);
    theme.muted = color(candidate.page_number);
    theme.accent = opaque(candidate.extra);
    theme.accent_text = readable_on(theme.accent);
    theme
}

fn color(value: ThemeColor) -> Color {
    Color::rgba(value.r, value.g, value.b, value.a)
}

fn opaque(value: ThemeColor) -> Color {
    Color::rgb(value.r, value.g, value.b)
}

/// `t` 从 0（`base`）到 1（`other`）。
fn mix(base: Color, other: Color, t: f32) -> Color {
    let blend = |left: u8, right: u8| {
        (f32::from(left) + (f32::from(right) - f32::from(left)) * t).round() as u8
    };
    Color::rgb(
        blend(base.r, other.r),
        blend(base.g, other.g),
        blend(base.b, other.b),
    )
}

/// 当前窗口主题是不是浅色。
pub fn is_light(theme: &Theme) -> bool {
    luminance(theme.background) > 150.0
}

fn luminance(color: Color) -> f32 {
    0.299 * f32::from(color.r) + 0.587 * f32::from(color.g) + 0.114 * f32::from(color.b)
}

fn readable_on(background: Color) -> Color {
    if luminance(background) > 150.0 {
        Color::rgb(24, 24, 30)
    } else {
        Color::rgb(255, 255, 255)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_platform::CandidateTheme;

    fn light() -> ThemeFile {
        ThemeFile {
            candidate: CandidateTheme {
                background: ThemeColor::rgb(250, 250, 252),
                text: ThemeColor::rgb(32, 32, 36),
                page_number: ThemeColor::rgb(120, 120, 128),
                extra: ThemeColor::rgb(15, 108, 189),
                ..CandidateTheme::default()
            },
            ..ThemeFile::default()
        }
    }

    #[test]
    fn maps_candidate_colors_to_the_window_theme() {
        let theme = build(&light());
        assert_eq!(theme.background, Color::rgb(250, 250, 252));
        assert_eq!(theme.accent, Color::rgb(15, 108, 189));
        assert!(theme.surface.r < theme.background.r);
        assert!(theme.border.r < theme.surface.r);
        assert!(is_light(&theme));
    }
}
