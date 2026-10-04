//! 用户词频学习与输入日志的落盘。
//!
//! [`FrequencyLearner`] 存聚合数据（选择次数、输入串选择、用户词、自造词库、个人英文词、个人 n-gram，都是 TSV 外加一个 SQLite）；
//! [`InputLog`] 逐条记上屏（jsonl），给离线回归评测与个人模型用；[`UsageStats`] 按天数打了多少字（`usage.tsv`），
//! [`VocabularyBook`] 记候选窗口里见过 / 上屏过哪些词（`user-vocab.tsv`）。

mod error;
mod frequency_learner;
mod input_log;
mod usage_stats;
mod user_word_bank;
mod vocabulary_book;

pub use error::LearningError;
pub use frequency_learner::FrequencyLearner;
pub use input_log::InputLog;
pub use usage_stats::UsageStats;
pub use vocabulary_book::VocabularyBook;
