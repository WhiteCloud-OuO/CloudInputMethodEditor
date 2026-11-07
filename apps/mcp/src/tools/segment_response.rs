//! `cloudime_segment_pinyin` 的结果。

use schemars::JsonSchema;
use serde::Serialize;

use super::SegmentationView;

/// `cloudime_segment_pinyin` 的结果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SegmentResponse {
    /// 原始输入。
    pub input: String,

    /// 首选切分；无法切分时为空。
    pub preferred: Option<String>,

    /// 所有切分，最多 16 个。
    pub segmentations: Vec<SegmentationView>,

    /// 末尾无法切分的字母。
    pub tail: String,
}
