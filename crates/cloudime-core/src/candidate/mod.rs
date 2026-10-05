//! 候选词数据模型。

mod kind;
mod layout;
mod list;

use serde::{Deserialize, Serialize};

pub use kind::CandidateKind;
pub use layout::{CandidateLayout, GRID_ROWS, Grid, MAX_CELL_EMS};
pub use list::CandidateList;

/// 一个可上屏的候选。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// 上屏文本。
    pub text: String,

    /// 候选里显示的内容；为空时显示 [`Self::text`]，上屏始终用 `text`。
    #[serde(default)]
    pub display: Option<String>,

    /// 来源类型。
    pub kind: CandidateKind,

    /// 该候选对应的拼音音节，供平台层高亮已匹配部分。
    pub syllables: Vec<String>,

    /// 候选的辅助读音 / 标注；中文词库候选不使用。
    pub reading: Option<String>,
}

impl Candidate {
    /// 候选里真正显示的内容：有 `display` 用它，否则用上屏文本。
    pub fn display_text(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.text)
    }
}
