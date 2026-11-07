//! `cloudime_lookup_pinyin` 的结果。

use schemars::JsonSchema;
use serde::Serialize;

use super::{CandidateView, SegmentationView};

/// `cloudime_lookup_pinyin` 的结果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LookupResponse {
    /// 原始输入。
    pub input: String,

    /// 首选切分。
    pub preferred: Option<String>,

    /// 所有切分，最多 16 个。
    pub segmentations: Vec<SegmentationView>,

    /// 排序后的候选。
    pub candidates: Vec<CandidateView>,

    /// 末尾无法切分的字母。
    pub tail: String,
}
