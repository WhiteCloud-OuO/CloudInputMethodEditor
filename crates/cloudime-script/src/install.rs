//! 安装目录：`cloudime.install_dir()` / `install_path()` / `shared_path()` /
//! `layout_path()` / `dll_path()` / `saved_path()` 背后的宿主接口。
//!
//! 只做「把相对路径解析到安装目录里」这一件事；`shared` / `layouts` / `dlls` 锁在
//! **当前脚本文件夹**的对应子目录，`saved` 还会按需建可写子目录。

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

    /// `Scripts\<脚本文件夹>\<kind>\<relative>` 里已有的文件：不递归、不跨出该子目录。
    pub(crate) fn resolve_script(
        &self,
        script: &Path,
        kind: &str,
        relative: &str,
    ) -> Result<Option<String>, String> {
        let Some(root) = self.root() else {
            return Ok(None);
        };
        let root = root.canonicalize().unwrap_or(root);
        let scripts = root.join("Scripts");
        let script = script
            .canonicalize()
            .unwrap_or_else(|_| script.to_path_buf());
        if !script.starts_with(&scripts) {
            return Err("脚本文件夹跑出了安装目录".to_owned());
        }
        // 子目录不存在 = 这个脚本没放这类文件，直接给 nil（不是错误）。
        let Ok(base) = script.join(kind).canonicalize() else {
            return Ok(None);
        };
        if !base.starts_with(&scripts) {
            return Err("脚本目录跑出了安装目录".to_owned());
        }
        let Ok(path) = base.join(relative).canonicalize() else {
            return Ok(None);
        };
        if !path.starts_with(&base) {
            let api = match kind {
                "shared" => "shared",
                "layouts" => "layout",
                "dlls" => "dll",
                _ => kind,
            };
            return Err(format!(
                "cloudime.{api}_path 只认脚本文件夹的 {kind}\\ 子目录里的文件"
            ));
        }
        Ok(Some(path.to_string_lossy().into_owned()))
    }

    /// `Scripts\<脚本文件夹>\saved\<relative>`；目录不存在就建，返回可写的绝对路径。
    ///
    /// `relative` 为空时返回该脚本的 `saved` 目录本身。只允许相对子路径，`..` / 绝对路径报错。
    pub(crate) fn saved(
        &self,
        script: &Path,
        relative: Option<&str>,
    ) -> Result<Option<String>, String> {
        let Some(root) = self.root() else {
            return Ok(None);
        };
        let root = root.canonicalize().unwrap_or(root);
        let scripts = root.join("Scripts");
        let script = script
            .canonicalize()
            .unwrap_or_else(|_| script.to_path_buf());
        if !script.starts_with(&scripts) {
            return Err("脚本文件夹跑出了安装目录".to_owned());
        }
        let base = script.join("saved");
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("建不了脚本的 saved 目录：{error}"))?;
        let base = base
            .canonicalize()
            .map_err(|error| format!("找不到脚本的 saved 目录（创建后仍不可访问）：{error}"))?;
        let Some(relative) = relative.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(Some(base.to_string_lossy().into_owned()));
        };
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err("cloudime.saved_path 只认脚本 saved 目录下的相对路径".to_owned());
        }
        let target = base.join(relative_path);
        let parent = target.parent().unwrap_or(&base);
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("建不了脚本 saved 目录的子目录：{error}"))?;
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("找不到脚本 saved 目录的子目录：{error}"))?;
        if !parent.starts_with(&base) {
            return Err("cloudime.saved_path 跑出了脚本的 saved 目录".to_owned());
        }
        let path = match target.file_name() {
            Some(name) => parent.join(name),
            None => parent,
        };
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}
