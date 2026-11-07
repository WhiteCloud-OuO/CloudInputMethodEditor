//! 用记事本打开脚本入口文件（「编辑此脚本」）。

use std::path::Path;

use crate::log;

/// 用记事本打开 `path`，不碰它的内容。
pub(crate) fn open(path: &Path) {
    if let Err(error) = std::process::Command::new("notepad.exe").arg(path).spawn() {
        log::warn(format!("打开记事本失败：{error}"));
    }
}
