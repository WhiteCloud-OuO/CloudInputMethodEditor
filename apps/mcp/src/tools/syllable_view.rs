//! 一个音节的切分结果。

use schemars::JsonSchema;
use serde::Serialize;

/// 一个音节的切分结果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SyllableView {
    /// 用户敲的字母。
    pub text: String,

    /// 是不是完整音节。
    pub complete: bool,
}
