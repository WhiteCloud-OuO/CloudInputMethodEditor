//! `[phrase]` 分节：自定义短语的开关。
//!
//! 短语库固定在安装目录的 `Phrases\Phrase.db`（位置不可配）；这里只有「软件自带短语是否参与」一项。
//! 「设置 → 短语」页负责增删改，Server 启动与热加载时读进引擎。

use serde::{Deserialize, Serialize};

/// `[phrase]` 分节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhraseConfig {
    /// 软件自带短语（`cloudime_default` 表）是否参与：关掉只用自己的短语。
    pub use_default_phrases: bool,
}

impl Default for PhraseConfig {
    fn default() -> Self {
        Self {
            use_default_phrases: true,
        }
    }
}
