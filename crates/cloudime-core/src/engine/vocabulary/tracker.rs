use super::VocabularySummary;

/// 词汇记录的落盘方。实现放学习 crate，Core 只送事件。
pub trait VocabularyTracker: Send {
    /// 用户上屏时这个词在屏幕上：记一次看到。
    fn record_exposure(&mut self, word: &str);

    /// 用户上屏了这个词：记一次。
    fn record_commit(&mut self, word: &str);

    /// 落盘。壳在停用输入法时调用，激活期间也定时调；失败只记日志。
    fn flush(&mut self) {}

    /// 「设置 → 统计」页的词汇汇总。
    fn summary(&self) -> VocabularySummary {
        VocabularySummary::default()
    }
}

/// 不记词汇。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVocabularyTracker;

impl VocabularyTracker for NoVocabularyTracker {
    fn record_exposure(&mut self, _word: &str) {}

    fn record_commit(&mut self, _word: &str) {}
}
