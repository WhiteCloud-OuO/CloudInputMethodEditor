//! 一个已加载词库的公开信息。

use schemars::JsonSchema;
use serde::Serialize;

/// 一个已加载词库的公开信息（不暴露本机绝对路径）。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DictionaryInfo {
    /// 词库名称。
    pub name: String,

    /// 词条总数。
    pub entries: usize,

    /// 许可证。
    pub license: String,

    /// `main` 或 `imported`。
    pub kind: String,
}
