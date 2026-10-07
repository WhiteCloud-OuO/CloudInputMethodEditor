use serde::{Deserialize, Serialize};

use crate::candidate::CandidateKind;

/// 上屏的文字从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSource {
    /// 词库里的词（含用户词）。
    Word,

    /// 本地整句转换。
    Sentence,

    /// 英文候选。
    English,

    /// 快捷候选（日期 / 算式 / 码点）。
    Shortcut,

    /// 用户配置的自定义短语。
    Custom,

    /// 本地词典的翻译 Tip：上屏的是词典给的译文，不是候选本身（拼音照候选消耗）。
    Translation,

    /// 回车原样上屏敲的字母。
    Raw,
}

impl From<CandidateKind> for InputSource {
    fn from(kind: CandidateKind) -> Self {
        match kind {
            CandidateKind::Chinese => Self::Word,
            CandidateKind::Sentence => Self::Sentence,
            CandidateKind::English => Self::English,
            CandidateKind::Shortcut => Self::Shortcut,
            CandidateKind::Custom => Self::Custom,
        }
    }
}
