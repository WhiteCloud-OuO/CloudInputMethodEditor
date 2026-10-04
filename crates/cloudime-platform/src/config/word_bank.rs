//! `[word_bank]` 分节：词库查询的开关与用户自造词库的位置。
//!
//! 第三方词库目录固定是随包根下的 `WordBank\`、主词库固定 `Dict.db`，这一节不再管目录与启用清单；
//! 这里只剩「生僻项（稀有组）是否参与查询」与「用户自造词库 `UserWordBank.db` 放哪」。

use serde::{Deserialize, Serialize};

/// 用户自造词库的缺省位置（相对安装目录，与随包词库同在 `WordBank\`）。
pub const DEFAULT_USER_WORD_BANK_FILE: &str = "WordBank/UserWordBank.db";

/// `[word_bank]` 分节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WordBankConfig {
    /// 从词库的稀有组（`Dict.db` 里的生僻字 / 生僻词）取词；缺省关。
    /// 关掉后候选与整句都不含这些冷僻条目，查询也快一些。
    pub rare_items: bool,

    /// 用户自造词库 `UserWordBank.db` 的位置（相对安装目录，如 `WordBank/UserWordBank.db`；也可写绝对路径）。
    #[serde(default = "default_user_file")]
    pub user_file: String,
}

/// `user_file` 缺省值：不能是空串，否则自造词落盘时会把路径当成空目录。
fn default_user_file() -> String {
    DEFAULT_USER_WORD_BANK_FILE.to_owned()
}

impl Default for WordBankConfig {
    fn default() -> Self {
        Self {
            rare_items: false,
            user_file: default_user_file(),
        }
    }
}
