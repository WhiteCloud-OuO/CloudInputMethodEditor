//! `cloudime_lookup_pinyin` 的参数。

use schemars::JsonSchema;
use serde::Deserialize;

/// `cloudime_lookup_pinyin` 的参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupRequest {
    /// 要查询的拼音串，可以用 `'` 强制分隔。
    pub pinyin: String,

    /// 最多返回多少条候选，1–100，缺省 20。
    pub limit: Option<usize>,

    /// 是否允许简拼；缺省 true。
    pub use_jian_pin: Option<bool>,
}
