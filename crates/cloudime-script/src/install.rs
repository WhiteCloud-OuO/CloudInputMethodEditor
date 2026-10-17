//! 安装目录：`cloudime.install_dir()` / `cloudime.install_path()` 背后的宿主接口。
//!
//! 只做「把相对路径解析到安装目录里」这一件事；脚本要怎么用（读 SVG、拼图片路径）由脚本自己决定。

use std::cell::RefCell;
use std::path::PathBuf;

/// 安装目录（`Scripts\` / `Images\` 的上一级）；没有正式安装时是 `None`。
#[derive(Default)]
pub(crate) struct InstallDir {
    root: RefCell<Option<PathBuf>>,
}

impl InstallDir {
    pub(crate) fn set(&self, dir: PathBuf) {
        *self.root.borrow_mut() = Some(dir);
    }

    pub(crate) fn root(&self) -> Option<PathBuf> {
        self.root.borrow().clone()
    }

    /// 相对路径 → 安装目录里的文件绝对路径。
    ///
    /// `None` = 没有安装目录 / 文件不存在；`Err` = `canonicalize` 之后跑出了安装目录。
    pub(crate) fn resolve(&self, relative: &str) -> Result<Option<String>, String> {
        let Some(root) = self.root() else {
            return Ok(None);
        };
        let root = root.canonicalize().unwrap_or(root);
        let Ok(path) = root.join(relative).canonicalize() else {
            return Ok(None);
        };
        if !path.starts_with(&root) {
            return Err("install_path 只认安装目录里的文件".to_owned());
        }
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}
