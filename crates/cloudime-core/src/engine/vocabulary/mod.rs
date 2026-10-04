//! 词汇记录：用户在候选窗口里见过哪些词、上屏过哪些。
//!
//! 「看到」按上屏那一刻屏幕上的那一页算（`Engine::note_displayed` 记下当前页，上屏时才记进词汇表），
//! 逐键刷新时一闪而过的候选不算：用户真正读候选是在要选的时候。
//! Core 只记录与汇总，按词落盘由实现做（`cloudime-learning` 的 `VocabularyBook`）；缺省 [`NoVocabularyTracker`] 不记。

mod summary;
mod tracker;

pub use summary::VocabularySummary;
pub use tracker::{NoVocabularyTracker, VocabularyTracker};
