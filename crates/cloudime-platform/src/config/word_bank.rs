//! `[word_bank]` 分节：词库查询的开关。
//!
//! 词库目录固定是随包根下的 `WordBank\`、主词库固定 `Dict.db`，这一节不再管目录与启用清单；
//! 只剩「生僻项（稀有组）是否参与查询」。

use serde::{Deserialize, Serialize};

/// `[word_bank]` 分节。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WordBankConfig {
    /// 从词库的稀有组（`Dict.db` 里的生僻字 / 生僻词）取词；缺省关。
    /// 关掉后候选与整句都不含这些冷僻条目，查询也快一些。
    pub rare_items: bool,
}
