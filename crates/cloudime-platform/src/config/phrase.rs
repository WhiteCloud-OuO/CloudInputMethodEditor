//! `[phrase]` 分节：用户短语的存放位置。
//!
//! 短语不再写在 config.toml 里，单独存在数据目录（`%APPDATA%\CloudIME`）下的 SQLite 文件里，
//! config.toml 只记它在哪；「设置 → 短语」页负责增删改。

use serde::{Deserialize, Serialize};

/// 短语库的缺省文件名（相对数据目录）。
pub const DEFAULT_PHRASE_FILE: &str = "Phrase.db";

/// `[phrase]` 分节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhraseConfig {
    /// 短语库的位置（相对数据目录，如 `Phrase.db`；也可以写绝对路径）。
    pub file: String,
}

impl Default for PhraseConfig {
    fn default() -> Self {
        Self {
            file: DEFAULT_PHRASE_FILE.to_owned(),
        }
    }
}
