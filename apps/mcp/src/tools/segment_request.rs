//! `cloudime_segment_pinyin` 的参数。

use schemars::JsonSchema;
use serde::Deserialize;

/// `cloudime_segment_pinyin` 的参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SegmentRequest {
    /// 要切分的拼音串，可以用 `'` 强制分隔。
    pub pinyin: String,

    /// 是否允许简拼；缺省 true。
    pub use_jian_pin: Option<bool>,
}
