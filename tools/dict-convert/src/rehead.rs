//! 把改名前的 `.qj` 就地改写成当前魔数：容器布局一字未动，只改文件头 8 字节。
//!
//! 改名前的魔数是 `QINGJIAN`，改名后是 `CLOUDIME`（[`cloudime_format::MAGIC`]）。读端两者都认
//! （[`cloudime_format::MAGIC_LEGACY`]，见 `cloudime-format`），但发新一版产品数据之前得先把头改过来，
//! 免得 `tools/release/data.lock` 一直钉着要靠兼容垫着的旧数据。改之前按容器完整校验一遍、改完再开一遍，
//! 坏文件原样报错不碰；已是新魔数的跳过。

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use cloudime_format::{Container, MAGIC, MAGIC_LEGACY};

use crate::args::DataKind;
use crate::error::ConvertError;

/// 把 `inputs` 里的文件头魔数改成 `MAGIC`。`kind` 决定按哪个数据种类校验容器。
pub fn rehead(kind: DataKind, inputs: &[PathBuf]) -> Result<(), ConvertError> {
    let expected = kind.container_kind();
    let mut changed = 0usize;
    for path in inputs {
        let magic = magic_of(path)?;
        if magic == MAGIC {
            tracing::debug!(path = %path.display(), "已经是新魔数，跳过");
            continue;
        }
        if magic != MAGIC_LEGACY {
            return Err(ConvertError::NotQj {
                path: path.clone(),
                magic: String::from_utf8_lossy(&magic).into_owned(),
            });
        }
        // 改之前按容器过一遍：魔数之外的版本、分节表、META 也得对得上，坏文件不碰
        Container::open(path, expected)?;
        let mut file = OpenOptions::new().write(true).open(path)?;
        file.write_all(&MAGIC)?;
        file.sync_all()?;
        // 改完再开一遍，确认读端真能用
        let container = Container::open(path, expected)?;
        tracing::info!(
            path = %path.display(),
            kind = ?container.kind(),
            "魔数已改成 CLOUDIME"
        );
        changed += 1;
    }
    tracing::info!(changed, files = inputs.len(), "改写完成");
    Ok(())
}

/// 文件头前 8 字节。
fn magic_of(path: &PathBuf) -> Result<[u8; MAGIC.len()], ConvertError> {
    let mut magic = [0u8; MAGIC.len()];
    File::open(path)?.read_exact(&mut magic)?;
    Ok(magic)
}
