//! 一种拼音切分。

use schemars::JsonSchema;
use serde::Serialize;

use super::SyllableView;

/// 一种拼音切分。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SegmentationView {
    /// 用 `'` 连接后的切分。
    pub joined: String,

    /// 各音节。
    pub syllables: Vec<SyllableView>,

    /// 覆盖的字母数。
    pub letters: usize,

    /// 未完成音节数。
    pub incomplete: usize,
}
