//! 命令行给出的词库来源。

use std::path::PathBuf;

/// 命令行给出的词库来源。
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// 覆盖主词库路径；给了它又没给 `--word-bank` 时不加载第三方词库。
    pub dict: Option<PathBuf>,

    /// 覆盖 `WordBank\` 目录。
    pub word_bank: Option<PathBuf>,

    /// 是否加载主词库的稀有组。
    pub include_rare: bool,

    /// 只查主词库，不加载第三方词库。
    pub no_extras: bool,
}
