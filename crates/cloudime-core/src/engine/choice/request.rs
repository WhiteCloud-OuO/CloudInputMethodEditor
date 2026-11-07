//! 一次选词请求 / 出字请求。

/// 一次选词请求：前文、敲的键的输入单元、要比的候选文本。三样都相同才算同一页，缓存按它对上。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChoiceRequest {
    pub context: String,
    pub typed: Vec<u32>,
    pub texts: Vec<String>,
}

/// 一次出字请求：缓存键、前文、每个音节与它敲全了没有。
#[derive(Debug, Clone)]
pub(crate) struct ComposeRequest {
    pub key: String,
    pub context: String,
    pub syllables: Vec<(String, bool)>,
}
