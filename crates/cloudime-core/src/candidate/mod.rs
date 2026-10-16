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

    /// 候选右侧要显示的**辅助码**（`XH` 这样；没有为 `None`，画不画由平台层看设置决定）。
    /// 作用域里**光标前那个音节**对应候选的那个字的码：`ai|qg` 显示「爱」的码、`aiqg|` 显示「情」的码。
    /// 只有双键拼音开着辅助码、而且挂着辅助码表时才填（见 `Engine::query_shuangpin`）。
    #[serde(default)]
    pub auxiliary: Option<String>,

    /// 上面那条码里**已经敲上**的前几位（`XH` 敲了 `X` 时是 `X`；一位都没敲就是 `None`）：
    /// 候选窗口把那几位画下划线，表示辅助码已经打上去了（见 `docs/design/input-schemes.md` §4）。
    #[serde(default)]
    pub auxiliary_typed: Option<String>,
}

impl Candidate {
    /// 候选里真正显示的内容：有 `display` 用它，否则用上屏文本。
    pub fn display_text(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.text)
    }
}
