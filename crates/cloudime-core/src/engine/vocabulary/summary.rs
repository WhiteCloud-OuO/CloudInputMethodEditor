/// 「设置 → 统计」页的词汇汇总。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VocabularySummary {
    /// 见过的不同词数。
    pub seen: u64,

    /// 上屏过的不同词数。
    pub committed: u64,

    /// 最近 7 天（含今天）第一次见到的词数。
    pub new_this_week: u64,
}
