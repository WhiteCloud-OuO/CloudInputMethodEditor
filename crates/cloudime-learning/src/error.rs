#[derive(Debug, thiserror::Error)]
pub enum LearningError {
    /// 学习数据文件读写失败（权限、坏盘一类；格式不对的行只跳过不报错）。
    #[error("failed to read or write user learning file: {0}")]
    Io(#[from] std::io::Error),

    /// 自造词库（SQLite）读写失败（坏文件、表结构不对等）。
    #[error("failed to read or write user word database: {0}")]
    Database(#[from] rusqlite::Error),
}
