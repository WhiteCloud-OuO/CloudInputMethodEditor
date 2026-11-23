//! 形码输入（`[input] input_schemes = "shape_code"`）的方案目录（设计见 `docs/design/shape-code.md`）。
//!
//! 安装目录下：
//!
//! ```text
//! InputSchemes/ShapeCode/
//!   use_input_scheme.cfg   键值行：scheme = wubi
//!   wubi/
//!     scheme.toml          [Info] Name / Version / Author，[Shape] Table / MaxCodeLength
//!     table.tsv            编码<TAB>文字<TAB>权重
//!   cangjie/               以后的方案，一个方案一个目录
//! ```
//!
//! 一个方案一个子目录，`use_input_scheme.cfg` 的 `scheme` 指目录名；换方案只改这一行。
//! Core 不读文件：这里把 `table.tsv` 解析成 [`cloudime_core::shape::Table`] 再交给 Engine。

use std::path::{Path, PathBuf};

use cloudime_core::shape::Table;

/// 方案文件（每个方案目录一份）。
pub const SCHEME_FILE: &str = "scheme.toml";

/// 方案文件里没写 `Table` 时用的码表文件名。
pub const DEFAULT_TABLE: &str = "table.tsv";

/// 方案文件里没写 `LevelOne` 时用的一级简码文件名。
pub const DEFAULT_LEVEL_ONE: &str = "level1.tsv";

/// 方案文件里没写 `MaxCodeLength` 时的最长编码长度（五笔 4 键）。
pub const DEFAULT_MAX_CODE_LENGTH: usize = 4;

/// `use_input_scheme.cfg` 里形码自己的几个开关（用户级，不进 `config.toml`）。
const KEY_MIXTURE_INPUT: &str = "mixture_input";
const KEY_UNIQUE_COMMIT: &str = "unique_commit";
const KEY_FIFTH_KEY_COMMIT: &str = "fifth_key_commit";

/// 形码方案**支持**哪些用户级开关（`scheme.toml` 的 `[Options]`）。
///
/// 这些是五笔特有的（英文混输 / 四码唯一自动上屏 / 第五码顶字），别的形码方案没写就不显示、也不生效；
/// 以后仓颉 / 声笔有自己的一套选项时，往里加自己的能力位。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShapeCapabilities {
    pub mixture_input: bool,
    pub unique_commit: bool,
    pub fifth_key_commit: bool,
}

impl ShapeCapabilities {
    /// 一个可选设置都没有（设置页那块面板只显示方案信息与按钮）。
    pub fn is_empty(self) -> bool {
        !self.mixture_input && !self.unique_commit && !self.fifth_key_commit
    }
}

/// 形码的用户级开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeOptions {
    /// 英文混输：五笔候选后面追加英文词（缺省**关**）。
    pub mixture_input: bool,

    /// 四码唯一时自动上屏（缺省**开**）。
    pub unique_commit: bool,

    /// 第五码顶字上屏（缺省**开**；英文混输开着时由壳跳过这一项）。
    pub fifth_key_commit: bool,
}

impl Default for ShapeOptions {
    fn default() -> Self {
        Self {
            mixture_input: false,
            unique_commit: true,
            fifth_key_commit: true,
        }
    }
}

/// 形码方案目录：**安装目录**下的 `InputSchemes\ShapeCode`（`bundled_root()` 下，不负责创建）。
///
/// 与双键拼音的方案目录同级；安装程序同样给 `InputSchemes\` 这棵目录授了「普通用户可写」。
pub fn shape_code_dir() -> Option<PathBuf> {
    crate::resources::bundled_root().map(|root| root.join("InputSchemes").join("ShapeCode"))
}

/// 目录里的一份形码方案。
#[derive(Debug)]
pub struct ShapeEntry {
    /// 方案 id：子目录名。
    pub id: String,

    /// 方案目录。
    pub dir: PathBuf,

    /// 读出来的方案；`Err` 里是失败原因（设置页用红字展示）。
    pub scheme: Result<ShapeScheme, ShapeSchemeError>,
}

impl ShapeEntry {
    /// 下拉里显示的名字：读不出来就用目录名兜底。
    pub fn label(&self) -> String {
        match &self.scheme {
            Ok(scheme) => scheme.name.clone(),
            Err(_) => self.id.clone(),
        }
    }

    /// 读不出来时那一串原因（读得出来就是空的）。
    pub fn problems(&self) -> String {
        match &self.scheme {
            Ok(_) => String::new(),
            Err(error) => error.to_string(),
        }
    }
}

/// 一份读出来的形码方案。
#[derive(Debug)]
pub struct ShapeScheme {
    /// `[Info] Name`：设置页列表里显示的名字。
    pub name: String,

    /// `[Info] Version`。
    pub version: String,

    /// `[Info] Author`；没写就是空串。
    pub author: String,

    /// `[Shape] MaxCodeLength`：最长编码长度。
    pub max_code_length: usize,

    /// 这个方案支持哪些用户级开关（`[Options]`，五笔特有的三项）。
    pub capabilities: ShapeCapabilities,

    /// 码表文件（`[Shape] Table` 相对方案目录）。
    pub table_path: PathBuf,

    /// 解析好的码表。
    pub table: Table,
}

/// 读方案失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum ShapeSchemeError {
    /// 文件读不出来（不在 / 没权限）。
    #[error("{path} 读不出来：{source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    /// 不是合法的 TOML / 不是 UTF-8。
    #[error("{path} 不是合法的 TOML：{source}")]
    Toml {
        path: PathBuf,
        source: toml::de::Error,
    },

    /// 必填项缺失。
    #[error("缺少必填项 {0}")]
    Missing(&'static str),

    /// 取值不合规（码表为空、长度不是整数等）。
    #[error("{0}")]
    Invalid(String),
}

/// 扫方案目录：只认**含 `scheme.toml` 的子目录**，按目录名（大小写不敏感）升序。
/// 目录不在 / 读不动 → 空表。
pub fn list(dir: &Path) -> Vec<ShapeEntry> {
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<ShapeEntry> = Vec::new();
    for file in files.flatten() {
        let path = file.path();
        if !path.is_dir() || !path.join(SCHEME_FILE).is_file() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let scheme = load(&path);
        entries.push(ShapeEntry {
            id: id.to_owned(),
            dir: path,
            scheme,
        });
    }
    entries.sort_by_key(|entry| entry.id.to_ascii_lowercase());
    entries
}

/// 读一个方案目录：`scheme.toml` + 它指的码表。
pub fn load(dir: &Path) -> Result<ShapeScheme, ShapeSchemeError> {
    let path = dir.join(SCHEME_FILE);
    let text = std::fs::read_to_string(&path).map_err(|source| ShapeSchemeError::Read {
        path: path.clone(),
        source,
    })?;
    let root: toml::Value = toml::from_str(&text).map_err(|source| ShapeSchemeError::Toml {
        path: path.clone(),
        source,
    })?;
    let info = section(&root, "Info")?;
    let name = required(info, "Name")?;
    let version = required(info, "Version")?;
    let author = optional(info, "Author").unwrap_or_default();
    let shape = section(&root, "Shape").ok();
    let capabilities = capabilities(&root);
    let table_name = shape
        .and_then(|shape| optional(shape, "Table"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TABLE.to_owned());
    let max_code_length = match shape.and_then(|shape| optional(shape, "MaxCodeLength")) {
        Some(value) => match value.trim().parse::<usize>() {
            Ok(length) if (1..=8).contains(&length) => length,
            _ => {
                return Err(ShapeSchemeError::Invalid(format!(
                    "MaxCodeLength = `{value}` 不合规：写 1–8 的整数"
                )));
            }
        },
        None => DEFAULT_MAX_CODE_LENGTH,
    };
    let table_path = dir.join(table_name.trim());
    let source = std::fs::read_to_string(&table_path).map_err(|source| ShapeSchemeError::Read {
        path: table_path.clone(),
        source,
    })?;
    let (mut table, problems) = Table::parse(&source, max_code_length);
    if !problems.is_empty() {
        tracing::warn!(
            path = %table_path.display(),
            skipped = problems.len(),
            first = %problems[0],
            "形码码表有格式不对的行，已跳过"
        );
    }
    if table.is_empty() {
        return Err(ShapeSchemeError::Invalid(format!(
            "码表 {} 里一条能用的条目都没有",
            table_path.display()
        )));
    }
    // 一级简码表可选：没写 `LevelOne` 就用 `level1.tsv`，文件不在就是没有一级简码。
    let level_one_name = shape
        .and_then(|shape| optional(shape, "LevelOne"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LEVEL_ONE.to_owned());
    let level_one_path = dir.join(level_one_name.trim());
    match std::fs::read_to_string(&level_one_path) {
        Ok(source) => {
            let problems = table.load_level_one(&source);
            if !problems.is_empty() {
                tracing::warn!(
                    path = %level_one_path.display(),
                    skipped = problems.len(),
                    first = %problems[0],
                    "一级简码表有格式不对的行，已跳过"
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %level_one_path.display(), "没有一级简码表");
        }
        Err(error) => {
            return Err(ShapeSchemeError::Read {
                path: level_one_path,
                source: error,
            });
        }
    }
    Ok(ShapeScheme {
        name,
        version,
        author,
        max_code_length,
        capabilities,
        table_path,
        table,
    })
}

/// 读 `[Options]` 里这个方案支持的用户级开关；节不在 / 没写就是全不支持。
fn capabilities(root: &toml::Value) -> ShapeCapabilities {
    let Some(options) = section(root, "Options").ok() else {
        return ShapeCapabilities::default();
    };
    let flag = |key: &str| optional(options, key).is_some_and(|value| super::flag_of(&value));
    ShapeCapabilities {
        mixture_input: flag("MixtureInput"),
        unique_commit: flag("UniqueCommit"),
        fifth_key_commit: flag("FifthKeyCommit"),
    }
}

/// 方案目录里有哪些方案 id（含 `scheme.toml` 的子目录，按目录名升序）；只扫目录名，不读码表。
fn scheme_ids(dir: &Path) -> Vec<String> {
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = files
        .flatten()
        .filter(|file| file.path().is_dir() && file.path().join(SCHEME_FILE).is_file())
        .filter_map(|file| file.file_name().to_str().map(str::to_owned))
        .collect();
    ids.sort_by_key(|id| id.to_ascii_lowercase());
    ids
}

/// 只读一个方案目录的 `scheme.toml` 能力位（不加载码表；读不出来就是全不支持）。
fn read_capabilities(dir: &Path) -> ShapeCapabilities {
    let Ok(text) = std::fs::read_to_string(dir.join(SCHEME_FILE)) else {
        return ShapeCapabilities::default();
    };
    match toml::from_str::<toml::Value>(&text) {
        Ok(root) => capabilities(&root),
        Err(_) => ShapeCapabilities::default(),
    }
}

/// **当前方案**：`use_input_scheme.cfg` 的 `scheme` 指哪份用哪份；指不到（没写 / 指错了）
/// 就取按目录名升序的**第一个可用方案**，并把 `.cfg` 重建过来。
pub fn current() -> Option<ShapeScheme> {
    let dir = shape_code_dir()?;
    let entries = list(&dir);
    let current = resolve_current(&dir, &entries)?;
    for entry in entries {
        if entry.id.eq_ignore_ascii_case(&current) {
            return match entry.scheme {
                Ok(scheme) => Some(scheme),
                Err(error) => {
                    tracing::warn!(id = %entry.id, %error, "形码方案读不出来，这一轮退回全键拼音");
                    None
                }
            };
        }
    }
    None
}

/// **当前方案**的码表（Server / CLI 挂到 Engine 上）；没有可用方案为 `None`（退回全键拼音）。
pub fn current_table() -> Option<Table> {
    let scheme = current()?;
    tracing::info!(
        name = %scheme.name,
        version = %scheme.version,
        rows = scheme.table.len(),
        path = %scheme.table_path.display(),
        "形码方案已加载"
    );
    Some(scheme.table)
}

/// 读方案目录 `use_input_scheme.cfg` 里的形码开关；没写就是 [`ShapeOptions::default`]。
pub fn read_options(dir: &Path) -> ShapeOptions {
    let config = super::read_config(dir);
    let flag = |key: &str, default: bool| {
        config
            .get(key)
            .map_or(default, |value| super::flag_of(value))
    };
    ShapeOptions {
        mixture_input: flag(KEY_MIXTURE_INPUT, false),
        unique_commit: flag(KEY_UNIQUE_COMMIT, true),
        fifth_key_commit: flag(KEY_FIFTH_KEY_COMMIT, true),
    }
}

/// 当前形码方案目录的开关（没有安装目录 / 没有方案目录就是缺省值）。
///
/// **只返回当前方案支持的那几项**：方案没有声明 `[Options]` 里的能力时一律按关处理，
/// 切到仓颉 / 声笔时五笔特有的三项不会偷偷生效。
pub fn current_options() -> ShapeOptions {
    let Some(dir) = shape_code_dir() else {
        return ShapeOptions::default();
    };
    let mut options = read_options(&dir);
    let capabilities = current_capabilities();
    options.mixture_input &= capabilities.mixture_input;
    options.unique_commit &= capabilities.unique_commit;
    options.fifth_key_commit &= capabilities.fifth_key_commit;
    options
}

/// 当前方案声明支持哪些用户级开关（只读 `scheme.toml`，不加载码表）。
pub fn current_capabilities() -> ShapeCapabilities {
    let Some(dir) = shape_code_dir() else {
        return ShapeCapabilities::default();
    };
    let id = super::read_current(&dir)
        .filter(|id| dir.join(id).join(SCHEME_FILE).is_file())
        .or_else(|| scheme_ids(&dir).into_iter().next());
    id.map(|id| read_capabilities(&dir.join(id)))
        .unwrap_or_default()
}

/// 改一项开关（`mixture_input` / `unique_commit` / `fifth_key_commit`），别的行原样保留。
pub fn set_option(dir: &Path, key: &str, on: bool) -> std::io::Result<()> {
    set_config_line(dir, key, super::flag_key(on))
}

/// 换当前方案：只改 `scheme` 那一行（另外几行原样留着）。
pub fn set_current(dir: &Path, id: &str) -> std::io::Result<()> {
    write_current(dir, id)
}

/// 定下当前方案：`.cfg` 里指得到的就用它；否则取**第一个读得出来的**方案并**重建** `.cfg`。
/// 一个方案都没有 → `None`。
pub fn resolve_current(dir: &Path, entries: &[ShapeEntry]) -> Option<String> {
    let first = entries
        .iter()
        .find(|entry| entry.scheme.is_ok())
        .or_else(|| entries.first())?;
    let wanted = super::read_current(dir).unwrap_or_default();
    match entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(wanted.trim()))
    {
        Some(entry) => Some(entry.id.clone()),
        None => {
            if let Err(error) = write_current(dir, &first.id) {
                tracing::warn!(%error, "重建形码方案的 use_input_scheme.cfg 失败");
            }
            Some(first.id.clone())
        }
    }
}

/// 把当前方案写进 `use_input_scheme.cfg`（`scheme` 那一行；别的行原样留着）。
fn write_current(dir: &Path, id: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(super::CURRENT_FILE);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line
                .split_once('=')
                .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("scheme"))
        })
        .map(str::to_owned)
        .collect();
    lines.push(format!("scheme = {id}"));
    let mut body = String::from(
        "# 当前使用的形码方案：scheme = 方案目录名（wubi / cangjie / shengbi），改这一行即可换方案。\n",
    );
    body.push_str(&lines.join("\n"));
    body.push('\n');
    cloudime_core::storage::write_atomic_str(&path, &body)
}

/// 改 `use_input_scheme.cfg` 里的一行（同名行只留一行），别的行原样留着。
fn set_config_line(dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(super::CURRENT_FILE);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let is_key = line
            .trim()
            .split_once('=')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key));
        if is_key {
            if !replaced {
                lines.push(format!("{key} = {value}"));
                replaced = true;
            }
        } else {
            lines.push(line.to_owned());
        }
    }
    if !replaced {
        lines.push(format!("{key} = {value}"));
    }
    let mut body = lines.join("\n");
    body.push('\n');
    cloudime_core::storage::write_atomic_str(&path, &body)
}

/// 取 `[名字]` 节（节名大小写不敏感）。
fn section<'a>(root: &'a toml::Value, name: &str) -> Result<&'a toml::Table, ShapeSchemeError> {
    let table = root
        .as_table()
        .and_then(|table| {
            table
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
        .and_then(toml::Value::as_table);
    table.ok_or(ShapeSchemeError::Missing(if name == "Info" {
        "[Info] 节"
    } else {
        "[Shape] 节"
    }))
}

/// 取节里的字符串值（键名大小写不敏感）；不是字符串就当没写。
fn optional(table: &toml::Table, key: &str) -> Option<String> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| match value {
            toml::Value::String(text) => Some(text.clone()),
            toml::Value::Boolean(flag) => Some(flag.to_string()),
            toml::Value::Integer(number) => Some(number.to_string()),
            _ => None,
        })
}

/// 取必填的字符串值；没写 / 不是字符串都算缺失。
fn required(table: &toml::Table, key: &'static str) -> Result<String, ShapeSchemeError> {
    optional(table, key).ok_or(ShapeSchemeError::Missing(match key {
        "Name" => "[Info] Name",
        "Version" => "[Info] Version",
        _ => key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cloudime-shape-code-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn write_scheme(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(SCHEME_FILE),
            format!(
                "[Info]\nName = \"{name}\"\nVersion = \"1.0\"\nAuthor = \"测试\"\n\n[Shape]\nTable = \"table.tsv\"\nMaxCodeLength = 4\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("table.tsv"),
            "a\t工\t20\naaaa\t工\t10\nggll\t一\t20\n",
        )
        .unwrap();
    }

    #[test]
    fn a_scheme_directory_is_scanned_and_loaded() {
        let root = temp_dir("scan");
        write_scheme(&root.join("wubi"), "五笔86");
        std::fs::create_dir_all(root.join("cangjie")).unwrap();
        std::fs::write(root.join("readme.txt"), "不是方案").unwrap();

        let entries = list(&root);
        assert_eq!(entries.len(), 1, "只有带 scheme.toml 的目录才算方案");
        assert_eq!(entries[0].id, "wubi");
        assert_eq!(entries[0].label(), "五笔86");
        assert_eq!(entries[0].problems(), "");
        let scheme = entries[0].scheme.as_ref().unwrap();
        assert_eq!(scheme.version, "1.0");
        assert_eq!(scheme.author, "测试");
        assert_eq!(scheme.max_code_length, 4);
        assert_eq!(scheme.table.len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_current_file_points_at_a_scheme_and_is_rebuilt_when_missing() {
        let root = temp_dir("current");
        write_scheme(&root.join("wubi"), "五笔86");
        let entries = list(&root);

        // 没写 .cfg：取第一个，并把它写进去
        assert_eq!(resolve_current(&root, &entries).as_deref(), Some("wubi"));
        assert_eq!(super::super::read_current(&root).as_deref(), Some("wubi"));

        // 指错了：回到第一个并重建
        std::fs::write(root.join(super::super::CURRENT_FILE), "scheme = 没有这个\n").unwrap();
        assert_eq!(resolve_current(&root, &entries).as_deref(), Some("wubi"));
        assert_eq!(super::super::read_current(&root).as_deref(), Some("wubi"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_current_file_keeps_the_other_lines() {
        let root = temp_dir("keep");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(super::super::CURRENT_FILE),
            "# 注释\nscheme = old\nother = 1\n",
        )
        .unwrap();
        write_current(&root, "wubi").unwrap();
        let text = std::fs::read_to_string(root.join(super::super::CURRENT_FILE)).unwrap();
        assert!(text.contains("other = 1"), "{text}");
        assert!(text.contains("scheme = wubi"));
        assert!(!text.contains("scheme = old"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_or_broken_scheme_files_are_reported() {
        let root = temp_dir("broken");
        std::fs::create_dir_all(root.join("bad")).unwrap();
        std::fs::write(
            root.join("bad").join(SCHEME_FILE),
            "[Info]\nName = \"缺版本\"\n",
        )
        .unwrap();
        let entries = list(&root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label(), "bad", "读不出来就用目录名兜底");
        assert!(
            entries[0].problems().contains("Version"),
            "{}",
            entries[0].problems()
        );

        // 码表不在：也算读不出来
        let root = temp_dir("no-table");
        let dir = root.join("wubi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(SCHEME_FILE),
            "[Info]\nName = \"五笔86\"\nVersion = \"1.0\"\n",
        )
        .unwrap();
        let entries = list(&root);
        assert!(
            entries[0].problems().contains("table.tsv"),
            "{}",
            entries[0].problems()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_table_name_and_max_code_length_can_be_written() {
        let root = temp_dir("options");
        let dir = root.join("wubi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(SCHEME_FILE),
            "[info]\nname = \"五笔86\"\nversion = \"1.0\"\n\n[shape]\ntable = \"custom.tsv\"\nmaxcodelength = 3\n",
        )
        .unwrap();
        std::fs::write(dir.join("custom.tsv"), "abc\t字\t1\nabcd\t太长\t1\n").unwrap();
        let scheme = load(&dir).unwrap();
        assert_eq!(scheme.name, "五笔86", "节名 / 键名大小写不敏感");
        assert_eq!(scheme.max_code_length, 3);
        assert_eq!(scheme.table.len(), 1, "超过 3 位的编码被跳过");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn level_one_is_loaded_with_the_scheme() {
        let root = temp_dir("level-one");
        let dir = root.join("wubi");
        write_scheme(&dir, "五笔86");
        std::fs::write(
            dir.join(SCHEME_FILE),
            "[Info]\nName = \"五笔86\"\nVersion = \"1.0\"\n\n[Shape]\nTable = \"table.tsv\"\nLevelOne = \"level1.tsv\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("level1.tsv"), "g\t一\nf\t地\n").unwrap();
        let scheme = load(&dir).unwrap();
        assert_eq!(scheme.table.level_one('g'), Some("一"));
        assert_eq!(scheme.table.level_one('f'), Some("地"));
        assert_eq!(scheme.table.level_one('a'), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn capabilities_come_from_the_scheme_options_section() {
        let root = temp_dir("capabilities");
        let dir = root.join("wubi");
        write_scheme(&dir, "五笔86");
        std::fs::write(
            dir.join(SCHEME_FILE),
            "[Info]\nName = \"五笔86\"\nVersion = \"1.0\"\n\n[Shape]\nTable = \"table.tsv\"\n\n[options]\nMixtureInput = true\nUniqueCommit = false\n",
        )
        .unwrap();
        let scheme = load(&dir).unwrap();
        assert!(scheme.capabilities.mixture_input);
        assert!(!scheme.capabilities.unique_commit);
        assert!(!scheme.capabilities.fifth_key_commit, "没写就是不支持");

        // 没有 [Options] 的方案：三项都不支持（设置页不显示这些五笔专属开关）
        std::fs::write(
            dir.join(SCHEME_FILE),
            "[Info]\nName = \"五笔86\"\nVersion = \"1.0\"\n\n[Shape]\nTable = \"table.tsv\"\n",
        )
        .unwrap();
        let scheme = load(&dir).unwrap();
        assert!(scheme.capabilities.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_shape_options_round_trip_and_keep_other_lines() {
        let dir = temp_dir("shape-options");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(super::super::CURRENT_FILE),
            "# 注释\nscheme = wubi\nother = 1\n",
        )
        .unwrap();

        let options = read_options(&dir);
        assert!(!options.mixture_input, "英文混输缺省关");
        assert!(options.unique_commit, "四码唯一缺省开");
        assert!(options.fifth_key_commit, "第五码顶字缺省开");

        set_option(&dir, "mixture_input", true).unwrap();
        set_option(&dir, "fifth_key_commit", false).unwrap();
        let options = read_options(&dir);
        assert!(options.mixture_input);
        assert!(options.unique_commit);
        assert!(!options.fifth_key_commit);

        let text = std::fs::read_to_string(dir.join(super::super::CURRENT_FILE)).unwrap();
        assert!(text.contains("other = 1"), "{text}");
        assert!(text.contains("scheme = wubi"), "{text}");
        assert!(text.contains("mixture_input = on"), "{text}");
        assert!(text.contains("fifth_key_commit = off"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
