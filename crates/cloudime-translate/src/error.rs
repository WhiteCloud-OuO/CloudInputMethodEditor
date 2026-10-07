use thiserror::Error;

/// 本地词典与学习状态出错。
#[derive(Debug, Error)]
pub enum TranslateError {
    #[error(".qj 容器：{0}")]
    Format(#[from] cloudime_format::FormatError),

    #[error("读文件失败：{0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite：{0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("不认这种词典文件（只支持 .qj 与 .db）：{0}")]
    Unsupported(String),

    #[error("词典文件坏了：{0}")]
    Corrupt(&'static str),
}
