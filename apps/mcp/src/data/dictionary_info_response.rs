//! `cloudime_dictionary_info` 的结果。

use schemars::JsonSchema;
use serde::Serialize;

use super::DictionaryInfo;

/// `cloudime_dictionary_info` 的结果。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DictionaryInfoResponse {
    /// 主词库与已加载的第三方词库。
    pub dictionaries: Vec<DictionaryInfo>,

    /// 主词库的稀有组是否已加载并启用。
    pub rare_enabled: bool,
}
