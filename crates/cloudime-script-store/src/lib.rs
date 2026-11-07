//! 在线脚本仓库：读取索引、下载脚本文件并安装到安装目录的 `Scripts\<脚本文件夹>\`。
//!
//! 一个脚本就是一个文件夹：入口 `.lua` 放在文件夹根部，`shared\`（共享 Lua）、`layouts\`（布局 JSON）、
//! `dlls\`（动态库）是它的子目录。安装前先把所有文件下载到内存并检查冲突，再一次性写入；
//! **已存在且内容不同的文件不会被覆盖**，由调用方提示用户先删掉。
//!
//! 索引地址可以用 `CLOUDIME_SCRIPT_INDEX` 覆盖（调试 / 镜像用）。

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

/// 默认索引地址。
pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/WhiteCloud-OuO/CloudIME_ScriptWarehouse/main/index.json";

/// 索引文件上限。
const MAX_INDEX_BYTES: usize = 1024 * 1024;

/// 单个脚本文件上限。
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// 单个脚本所有文件总上限。
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;

/// 在线脚本错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("网络请求失败：{0}")]
    Http(#[from] reqwest::Error),

    #[error("脚本索引不是合法 JSON：{0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Invalid(String),

    #[error("读写脚本文件失败：{0}")]
    Io(#[from] std::io::Error),
}

/// 一次读取后的脚本仓库。
#[derive(Debug, Clone)]
pub struct Catalog {
    /// 索引自身的地址；脚本文件的相对路径按它解析。
    pub index_url: Url,

    /// 仓库里的脚本，顺序与索引一致。
    pub scripts: Vec<RemoteScript>,
}

/// 一个可安装的在线脚本。
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteScript {
    /// 稳定标识，同时是安装后的**文件夹名**（`Scripts\<id>\`）；索引内唯一。
    pub id: String,

    /// 显示名。
    pub name: String,

    /// 一句话说明。
    #[serde(default)]
    pub description: String,

    /// 要安装的文件；其中必须**正好有一个** `target = "script"` 的入口 `.lua`。
    pub files: Vec<RemoteFile>,
}

/// 一个要下载的文件。
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteFile {
    /// 相对索引的路径（脚本仓库里的文件）；安装时只取文件名。
    pub path: String,

    /// 装到脚本文件夹的哪里。
    pub target: Target,
}

/// 文件安装位置（都在 `Scripts\<脚本 id>\` 下）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// 脚本文件夹根部（入口 `.lua`）。
    Script,

    /// `shared\`。
    Shared,

    /// `layouts\`。
    Layout,

    /// `dlls\`。
    Dll,
}

/// 安装结果。
#[derive(Debug, Clone, Default)]
pub struct InstallReport {
    /// 本次新写入的文件（相对 `Scripts\`，含脚本文件夹名）。
    pub written: Vec<String>,

    /// 已存在且内容相同的文件（没有重复写）。
    pub skipped: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawCatalog {
    version: u32,
    scripts: Vec<RemoteScript>,
}

/// 当前应使用的索引地址：环境变量优先。
pub fn index_url() -> String {
    std::env::var("CLOUDIME_SCRIPT_INDEX")
        .ok()
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_INDEX_URL.to_owned())
}

/// 下载并解析索引。
pub fn fetch_catalog() -> Result<Catalog, Error> {
    let url = Url::parse(&index_url()).map_err(|error| {
        Error::Invalid(format!(
            "脚本索引地址不合法（{error}）；检查 CLOUDIME_SCRIPT_INDEX"
        ))
    })?;
    let client = client(Duration::from_secs(15))?;
    let response = client.get(url.clone()).send()?.error_for_status()?;
    let bytes = response.bytes()?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(Error::Invalid(format!(
            "脚本索引太大（{} 字节，上限 {MAX_INDEX_BYTES}）",
            bytes.len()
        )));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| Error::Invalid(format!("脚本索引不是 UTF-8：{error}")))?;
    Catalog::parse(url, text)
}

impl Catalog {
    /// 从 JSON 文本解析并校验索引。
    pub fn parse(index_url: Url, text: &str) -> Result<Self, Error> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let raw: RawCatalog = serde_json::from_str(text)?;
        if raw.version != 2 {
            return Err(Error::Invalid(format!(
                "不认识的脚本索引版本 {}（只支持 2）",
                raw.version
            )));
        }
        let mut ids = std::collections::HashSet::new();
        for script in &raw.scripts {
            validate_id(&script.id)?;
            if !ids.insert(script.id.clone()) {
                return Err(Error::Invalid(format!("脚本 id 重复：{}", script.id)));
            }
            if script.name.trim().is_empty() {
                return Err(Error::Invalid(format!("脚本 {} 没有 name", script.id)));
            }
            if script.files.is_empty() {
                return Err(Error::Invalid(format!("脚本 {} 没有文件", script.id)));
            }
            for file in &script.files {
                validate_file(&script.id, file)?;
            }
            let entries = script
                .files
                .iter()
                .filter(|file| file.target == Target::Script)
                .count();
            if entries != 1 {
                return Err(Error::Invalid(format!(
                    "脚本 {} 必须正好有一个 target = script 的入口 .lua（现在 {entries} 个）",
                    script.id
                )));
            }
        }
        Ok(Self {
            index_url,
            scripts: raw.scripts,
        })
    }

    /// 安装一个脚本；先下载全部文件并检查冲突，再写盘。
    ///
    /// `scripts_root` 是安装目录下的 `Scripts\`；文件装进 `Scripts\<脚本 id>\`。已存在且内容
    /// 不同的文件会报错，不会覆盖；内容相同的文件计入 [`InstallReport::skipped`]。
    pub fn install(
        &self,
        script: &RemoteScript,
        scripts_root: &Path,
    ) -> Result<InstallReport, Error> {
        validate_id(&script.id)?;
        let client = client(Duration::from_secs(30))?;
        let mut downloads = Vec::with_capacity(script.files.len());
        let mut seen = std::collections::HashSet::new();
        let mut total = 0usize;
        for file in &script.files {
            let target = self.target_path(scripts_root, &script.id, file)?;
            if !seen.insert(target.clone()) {
                return Err(Error::Invalid(format!(
                    "脚本 {} 有两个文件装到同一位置：{}",
                    script.id,
                    target.display()
                )));
            }
            let url = self.file_url(&file.path)?;
            let response = client.get(url).send()?.error_for_status()?;
            let bytes = response.bytes()?.to_vec();
            if bytes.len() > MAX_FILE_BYTES {
                return Err(Error::Invalid(format!(
                    "{} 太大（{} 字节，上限 {MAX_FILE_BYTES}）",
                    file.path,
                    bytes.len()
                )));
            }
            total += bytes.len();
            if total > MAX_TOTAL_BYTES {
                return Err(Error::Invalid(format!(
                    "脚本 {} 的文件总大小超过上限 {MAX_TOTAL_BYTES} 字节",
                    script.id
                )));
            }
            downloads.push((file.clone(), target, bytes));
        }

        // 先查冲突：有一个不同就整份不装，避免装一半。
        for (file, target, bytes) in &downloads {
            if target.is_file() {
                let existing = std::fs::read(target)?;
                if existing != *bytes {
                    return Err(Error::Invalid(format!(
                        "{} 已存在且内容不同；先删除它再获取",
                        file.path
                    )));
                }
            }
        }

        let mut report = InstallReport::default();
        for (file, target, bytes) in downloads {
            let relative = target
                .strip_prefix(scripts_root)
                .unwrap_or(&target)
                .to_string_lossy()
                .replace('\\', "/");
            if target.is_file() {
                report.skipped.push(relative);
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temp = target.with_file_name(format!(
                "{}.part",
                target
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default()
            ));
            std::fs::write(&temp, &bytes)?;
            std::fs::rename(&temp, &target).inspect_err(|_| {
                let _ = std::fs::remove_file(&temp);
            })?;
            tracing::info!(file = %file.path, target = %target.display(), "在线脚本文件已安装");
            report.written.push(relative);
        }
        Ok(report)
    }

    /// 相对索引地址解析一个文件 URL。
    pub fn file_url(&self, path: &str) -> Result<Url, Error> {
        self.index_url
            .join(path)
            .map_err(|error| Error::Invalid(format!("脚本文件路径不合法（{path}）：{error}")))
    }

    fn target_path(
        &self,
        scripts_root: &Path,
        id: &str,
        file: &RemoteFile,
    ) -> Result<PathBuf, Error> {
        let relative = Path::new(&file.path);
        let name = relative
            .file_name()
            .ok_or_else(|| Error::Invalid(format!("脚本文件路径没有文件名：{}", file.path)))?;
        let folder = scripts_root.join(id);
        let base = match file.target {
            Target::Script => {
                if !relative
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
                {
                    return Err(Error::Invalid(format!(
                        "脚本文件必须是 .lua：{}",
                        file.path
                    )));
                }
                folder
            }
            Target::Shared => folder.join("shared"),
            Target::Layout => folder.join("layouts"),
            Target::Dll => folder.join("dlls"),
        };
        Ok(base.join(name))
    }
}

/// 脚本 id 同时是安装后的文件夹名：挡掉路径分隔符、Windows 保留名与结尾的点 / 空格。
fn validate_id(id: &str) -> Result<(), Error> {
    if id.is_empty() || id.trim() != id {
        return Err(Error::Invalid(format!(
            "脚本 id 不能为空，也不能以空白开头 / 结尾：{id:?}"
        )));
    }
    if id == "." || id == ".." {
        return Err(Error::Invalid(format!("脚本 id 不能是 {id:?}")));
    }
    if let Some(bad) = id.chars().find(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(Error::Invalid(format!(
            "脚本 id 里不能有 {bad:?}（它会当文件夹名）：{id}"
        )));
    }
    if id.ends_with('.') || id.ends_with(' ') {
        return Err(Error::Invalid(format!(
            "脚本 id 不能以点或空格结尾（Windows 文件夹名不允许）：{id}"
        )));
    }
    let upper = id.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err(Error::Invalid(format!(
            "脚本 id 是 Windows 保留名，不能当文件夹名：{id}"
        )));
    }
    Ok(())
}

fn validate_file(script: &str, file: &RemoteFile) -> Result<(), Error> {
    let path = Path::new(&file.path);
    if file.path.trim().is_empty() {
        return Err(Error::Invalid(format!("脚本 {script} 有空文件路径")));
    }
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::Invalid(format!(
            "脚本 {script} 的文件路径必须是相对路径且不能含 ..：{}",
            file.path
        )));
    }
    if file.target == Target::Script
        && !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
    {
        return Err(Error::Invalid(format!(
            "脚本 {script} 的脚本文件必须是 .lua：{}",
            file.path
        )));
    }
    Ok(())
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, Error> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent("cloudime-settings")
        .build()
        .map_err(Error::Http)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(text: &str) -> Result<Catalog, Error> {
        Catalog::parse(
            Url::parse("https://example.com/scripts/index.json").unwrap(),
            text,
        )
    }

    #[test]
    fn parses_and_joins_file_urls() {
        let catalog = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "src/demo/main.lua", "target": "script" }
                    ]
                }]
            }"##,
        )
        .expect("catalog");
        assert_eq!(catalog.scripts.len(), 1);
        let url = catalog.file_url("src/demo/main.lua").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/scripts/src/demo/main.lua"
        );
    }

    #[test]
    fn script_target_must_be_lua() {
        let error = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "src/main.json", "target": "script" }
                    ]
                }]
            }"##,
        )
        .expect_err("script target");
        assert!(error.to_string().contains("必须是 .lua"), "{error}");
    }

    #[test]
    fn target_paths_live_inside_the_script_folder() {
        let catalog = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "src/demo/main.lua", "target": "script" },
                        { "path": "src/demo/helper.lua", "target": "shared" },
                        { "path": "src/demo/ui.json", "target": "layout" },
                        { "path": "src/demo/native.dll", "target": "dll" }
                    ]
                }]
            }"##,
        )
        .expect("catalog");
        let root = Path::new("Scripts");
        let script = &catalog.scripts[0];
        for (file, expected) in [
            (&script.files[0], "Scripts/demo/main.lua"),
            (&script.files[1], "Scripts/demo/shared/helper.lua"),
            (&script.files[2], "Scripts/demo/layouts/ui.json"),
            (&script.files[3], "Scripts/demo/dlls/native.dll"),
        ] {
            assert_eq!(
                catalog.target_path(root, &script.id, file).unwrap(),
                Path::new(expected)
            );
        }
    }

    #[test]
    fn rejects_unsafe_script_ids() {
        for id in ["", " demo", "a/b", "a\\b", "..", "con", "COM1", "demo."] {
            let text = format!(
                r##"{{
                    "version": 2,
                    "scripts": [{{ "id": "{id}", "name": "演示",
                        "files": [{{ "path": "main.lua", "target": "script" }}] }}]
                }}"##
            );
            assert!(catalog(&text).is_err(), "id {id:?} 不该通过");
        }
    }

    #[test]
    fn requires_exactly_one_entry_script() {
        let error = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "a.lua", "target": "shared" }
                    ]
                }]
            }"##,
        )
        .expect_err("missing entry");
        assert!(error.to_string().contains("正好有一个"), "{error}");

        let error = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "a.lua", "target": "script" },
                        { "path": "b.lua", "target": "script" }
                    ]
                }]
            }"##,
        )
        .expect_err("two entries");
        assert!(error.to_string().contains("正好有一个"), "{error}");
    }

    #[test]
    fn rejects_parent_paths() {
        let error = catalog(
            r##"{
                "version": 2,
                "scripts": [{
                    "id": "demo",
                    "name": "演示",
                    "files": [
                        { "path": "../main.lua", "target": "shared" }
                    ]
                }]
            }"##,
        )
        .expect_err("parent path");
        assert!(error.to_string().contains("相对路径"), "{error}");
    }
}
