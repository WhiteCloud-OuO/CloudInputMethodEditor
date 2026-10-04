use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Dictionary(#[from] cloudime_dictionary::DictionaryError),

    #[error(transparent)]
    LanguageModel(#[from] cloudime_lm::LmError),

    #[error(transparent)]
    Neural(#[from] cloudime_neural::NeuralError),

    /// `pack` 少了必填的元数据。
    #[error("pack {kind} needs --name")]
    MissingName {
        /// `pack` 的种类名。
        kind: &'static str,
    },

    /// `.qj` 容器校验失败（`rehead` 改之前先按容器读一遍）。
    #[error(transparent)]
    Container(#[from] cloudime_format::FormatError),

    /// 不是 `.qj` 文件。
    #[error("{path} 不是 .qj 文件：魔数 {magic}")]
    NotQj {
        /// 出错的文件。
        path: PathBuf,

        /// 文件头前 8 字节的可读形式。
        magic: String,
    },
}
