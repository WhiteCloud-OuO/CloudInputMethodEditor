//! `[theme]` 分节：当前用哪份主题（`Themes\` 下的 JSON 主题文件名）。

use serde::{Deserialize, Serialize};

/// 缺省用的主题文件（随包 `Themes\default.json`）。
pub const DEFAULT_CURR_THEME: &str = crate::theme::DEFAULT_THEME_FILE;

/// `[theme]` 分节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    /// 当前主题的文件名（`Themes\` 下；用户目录 `%APPDATA%\CloudIME\Themes` 优先）。
    pub curr_theme: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            curr_theme: DEFAULT_CURR_THEME.to_owned(),
        }
    }
}
