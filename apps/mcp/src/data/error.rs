//! MCP Server 启动阶段错误。

use std::path::PathBuf;

use cloudime_dictionary::DictionaryError;
use thiserror::Error;

/// 启动阶段错误。
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("找不到安装目录；请用 --dict 指定词库")]
    MissingRoot,

    #[error("词库文件不存在：{0}")]
    MissingDictionary(PathBuf),

    #[error("读取词库失败：{path}: {source}")]
    Dictionary {
        path: PathBuf,
        #[source]
        source: DictionaryError,
    },
}
