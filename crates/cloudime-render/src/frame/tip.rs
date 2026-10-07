//! 候选窗口底部那一行左侧的翻译 Tip：一段一段画，词性斜体、释义常规。

use super::Tone;

/// 翻译 Tip 的一段文字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TipSegment {
    /// 文本。
    pub text: String,

    /// 深浅：释义按词条学没学会（[`Tone::TranslateFresh`] / [`Tone::TranslateLearned`]），
    /// 词性与分隔符用 [`Tone::Faint`]。
    pub tone: Tone,

    /// 斜体（词性用）。
    pub italic: bool,
}

impl TipSegment {
    pub fn new(text: impl Into<String>, tone: Tone, italic: bool) -> Self {
        Self {
            text: text.into(),
            tone,
            italic,
        }
    }
}
