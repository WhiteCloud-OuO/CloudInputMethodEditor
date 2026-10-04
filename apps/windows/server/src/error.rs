use cloudime_dictionary::DictionaryError;
use cloudime_lm::LmError;

/// 装配 Engine 时的错误。
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("load dictionary: {0}")]
    Dictionary(#[from] DictionaryError),

    #[error("load language model: {0}")]
    LanguageModel(#[from] LmError),
}
