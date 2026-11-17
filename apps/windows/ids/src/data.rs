//! 找并打开 `ids.qj`：命令行 > exe 同目录 > 当前目录。

use std::path::{Path, PathBuf};

use cloudime_ids::{IdsDatabase, IdsError};

/// 打开的数据。
pub struct Database {
    pub inner: IdsDatabase,
}

/// 按顺序找一个存在的 `data/generated/ids.qj`。
pub fn resolve(explicit: Option<&Path>) -> Result<Database, DataError> {
    let mut tried: Vec<PathBuf> = Vec::new();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.push(directory.join("data/generated/ids.qj"));
    }
    candidates.push(PathBuf::from("data/generated/ids.qj"));

    for path in candidates {
        tried.push(path.clone());
        if !path.is_file() {
            continue;
        }
        let inner = IdsDatabase::open(&path).map_err(|source| DataError::Open {
            path: path.clone(),
            source,
        })?;
        return Ok(Database { inner });
    }
    Err(DataError::Missing(
        tried
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("；"),
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error(
        "找不到 ids.qj（试过：{0}）。先在仓库根目录生成：\
         cargo run --release -p cloudime-ids-build -- --ids assets/ids/IDS.TXT \
         --components assets/ids/components.tsv --out data/generated/ids.qj"
    )]
    Missing(String),

    #[error("打开 {path} 失败：{source}")]
    Open { path: PathBuf, source: IdsError },
}
