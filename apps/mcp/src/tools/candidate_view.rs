//! 一条候选。

use schemars::JsonSchema;
use serde::Serialize;

/// 一条候选。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CandidateView {
    /// 上屏文本。
    pub text: String,

    /// 展示文本。
    pub display: String,

    /// 候选类型：Chinese / English / Sentence / Shortcut / Custom。
    pub kind: String,

    /// 候选对应的音节。
    pub syllables: Vec<String>,

    /// Core 排名权重；没有记录时为 null。
    pub weight: Option<f64>,
}
