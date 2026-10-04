use serde::{Deserialize, Serialize};

use super::LogLevel;

/// 每页最多几个候选：数字键只有 1–9（候选个数的上下限见 [`crate::config::MIN_CANDIDATE_COUNT`]）。
pub const MAX_PAGE_SIZE: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// 日志级别，缺省 info（不含用户敲的内容）。
    pub log_level: LogLevel,

    /// 输入日志：每次上屏记一行到数据目录的 `input-log.jsonl`（敲的键、看到的候选、选了什么），只写本机，
    /// 给离线回归评测与个人模型用。缺省开；关掉就不记，「高级」页可清空。
    pub input_log: bool,

    /// 学习输入习惯：按选择调整候选顺序、记新词与敲错纠正。关掉后不再记，已学的仍参与排序。
    pub learning: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            log_level: LogLevel::default(),
            input_log: true,
            learning: true,
        }
    }
}
