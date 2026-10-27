//! 安装目录：`cloudime.install_dir()` / `install_path()` / `shared_path()` /
//! `layout_path()` / `dll_path()` / `saved_path()` 背后的宿主接口。
//!
//! 只做「把相对路径解析到安装目录里」这一件事；`shared` / `layouts` / `dlls` 锁在
//! `Scripts` 的对应子目录，`saved` 还会按需建可写子目录。

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

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

    /// `Scripts\<kind>\<relative>` 里已有的文件：不递归、不跨出该子目录。
    pub(crate) fn resolve_script(
        &self,
        kind: &str,
        relative: &str,
    ) -> Result<Option<String>, String> {
        let Some(root) = self.root() else {
            return Ok(None);
        };
        let root = root.canonicalize().unwrap_or(root);
        let base = root.join("Scripts").join(kind);
        let Ok(base) = base.canonicalize() else {
            return Ok(None);
        };
        if !base.starts_with(&root) {
            return Err("脚本目录跑出了安装目录".to_owned());
        }
        let Ok(path) = base.join(relative).canonicalize() else {
            return Ok(None);
        };
        if !path.starts_with(&base) {
            let api = kind.strip_suffix('s').unwrap_or(kind);
            return Err(format!("cloudime.{api}_path 只认 Scripts\\{kind} 里的文件"));
        }
        Ok(Some(path.to_string_lossy().into_owned()))
    }

    /// `Scripts\saved\<relative>`；目录不存在就建，返回可写的绝对路径。
    ///
    /// `relative` 为空时返回 `Scripts\saved` 本身。只允许相对子路径，`..` / 绝对路径报错。
    pub(crate) fn saved(&self, relative: Option<&str>) -> Result<Option<String>, String> {
        let Some(root) = self.root() else {
            return Ok(None);
        };
        let root = root.canonicalize().unwrap_or(root);
        let base = root.join("Scripts").join("saved");
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("建不了 Scripts\\saved：{error}"))?;
        let base = base
            .canonicalize()
            .map_err(|error| format!("找不到 Scripts\\saved（创建后仍不可访问）：{error}"))?;
        let Some(relative) = relative.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(Some(base.to_string_lossy().into_owned()));
        };
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("cloudime.saved_path 只认 Scripts\\saved 下的相对路径".to_owned());
        }
        let target = base.join(relative_path);
        let parent = target.parent().unwrap_or(&base);
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("建不了 Scripts\\saved 子目录：{error}"))?;
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("找不到 Scripts\\saved 子目录：{error}"))?;
        if !parent.starts_with(&base) {
            return Err("cloudime.saved_path 跑出了 Scripts\\saved".to_owned());
        }
        let path = match target.file_name() {
            Some(name) => parent.join(name),
            None => parent,
        };
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}
