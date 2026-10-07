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

    #[error(transparent)]
    Phrase(#[from] cloudime_platform::PhraseError),

    /// `pack` 少了必填的元数据。
    #[error("pack {kind} needs --name")]
    MissingName {
        /// `pack` 的种类名。
        kind: &'static str,
    },

    /// `.qj` 容器校验失败（`rehead` 改之前先按容器读一遍）。
    #[error(transparent)]
    Container(#[from] cloudime_format::FormatError),

    #[error(transparent)]
    Translate(#[from] cloudime_translate::TranslateError),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error("写完自查没通过：{0}")]
    Verify(String),

    /// `phrase-db` 要覆盖已有文件，但没给 `--force`。
    #[error(
        "{path} 已存在：随包的 Phrases\\Phrase.db 里有手写的内置短语，整份换成空库会把它们抹掉；真要重来加 --force"
    )]
    PhraseDbExists {
        /// 目标文件。
        path: PathBuf,
    },

    /// 不是 `.qj` 文件。
    #[error("{path} 不是 .qj 文件：魔数 {magic}")]
    NotQj {
        /// 出错的文件。
        path: PathBuf,

        /// 文件头前 8 字节的可读形式。
        magic: String,
    },
}
