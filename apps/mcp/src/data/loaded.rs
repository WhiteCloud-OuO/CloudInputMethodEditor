//! 装载完成的查询引擎与词库信息。

use cloudime_core::Engine;

use super::DictionaryInfoResponse;

/// 装载好的查询引擎与词库信息。
pub struct Loaded {
    /// 只读查询引擎，不注入 Learner / LM / 神经模型。
    pub engine: Engine,

    /// 给 `cloudime_dictionary_info` 用。
    pub info: DictionaryInfoResponse,
}
