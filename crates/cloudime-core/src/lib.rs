//! 云朵输入法内核。
//!
//! 平台无关：词库、拼音解析、候选生成、排序与学习的接口全部在这里。
//! 平台层（TSF）只负责把按键喂给 [`Engine`]、把候选画出来。
//! 判断标准：换掉平台壳，不应该需要改这里的任何一行。

pub mod candidate;
pub mod char_width;
pub mod composition;
pub mod correction;
pub mod custom_phrase;
pub mod engine;
pub mod english;
pub mod fuzzy;
pub mod history;
pub mod parser;
pub mod punctuation;
pub mod ranking;
pub mod sentence;
pub mod shortcut;
pub mod storage;

pub use custom_phrase::CustomPhrase;

pub use candidate::{
    Candidate, CandidateKind, CandidateLayout, CandidateList, GRID_ROWS, Grid, MAX_CELL_EMS,
};
pub use cloudime_dictionary as dictionary;
pub use composition::Composition;
pub use correction::Correction;
pub use engine::{
    CommitEntry, Engine, EngineSession, Forgotten, INPUT_LOG_VERSION, InputLogEntry, InputLogger,
    InputSource, Learner, MarkedKind, MarkedSegment, NEURAL_MARGIN, NEURAL_WEIGHT, NoInputLogger,
    NoLearner, NoUsageMeter, NoVocabularyTracker, Query, RESCORE_CONTEXT_CHARS, RawPreedit,
    Timings, Usage, UsageMeter, UsageSummary, VocabularySummary, VocabularyTracker,
};
pub use fuzzy::FuzzyRules;
pub use history::InputHistory;
pub use parser::{ParseError, Segmentation};
pub use punctuation::{MappedSymbol, Mapping, Punctuation};
