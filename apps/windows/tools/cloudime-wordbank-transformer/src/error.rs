//! cwt 的错误类型。

use std::path::PathBuf;

use thiserror::Error;

/// 转换过程中的错误。
#[derive(Debug, Error)]
pub(crate) enum Error {
    /// 读源文件失败（不存在、不是 UTF-8 等）。
    #[error("读源文件 {path} 失败：{source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    /// 源文件里一行都没认出来。
    #[error("源文件 {0} 里没有能转换的条目（检查分隔符是不是制表符，或加 --ignore_head）")]
    Empty(PathBuf),

    /// 写 `.db` 失败。
    #[error("写词库 {path} 失败：{source}")]
    Sqlite {
        path: PathBuf,
        source: rusqlite::Error,
    },

    /// 删不掉要覆盖的旧输出文件 / 转换后的源文件。
    #[error("删除文件 {path} 失败：{source}")]
    Delete {
        path: PathBuf,
        source: std::io::Error,
    },
}
