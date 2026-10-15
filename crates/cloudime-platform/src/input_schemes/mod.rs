//! 输入方案（规范见 `docs/design/input-schemes.md`）。
//!
//! - [`double_keys`]：双键拼音**方案文件**的格式、解析与校验（设计文档 §3–§8）。
//! - 这一层：**方案目录**（§1）与 **`use_input_scheme.cfg`**（§2）—— 扫目录、定当前方案、首次写样例。

mod auxiliary;
mod double_keys;
mod key_map;

pub use auxiliary::AuxiliaryCodes;
pub use double_keys::{
    DoubleKeysScheme, KeyMapping, SAMPLE_SHOUDAO, SAMPLE_XIAOHE, SAMPLE_ZIRANMA, SAMPLES,
    SchemeError, load, scheme_id, set_using_auxiliary_codes,
};
pub use key_map::{
    KeyCap, KeyMapHotkey, KeyMapKey, KeyMapLayout, KeyMapModifiers, KeyMapReference,
};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 当前使用的方案与辅助码方案记在这个文件里（`键 = 值` 几行）。
pub const CURRENT_FILE: &str = "use_input_scheme.cfg";

/// 辅助码文件放方案目录下的这个子目录。
pub const AUXILIARY_DIR: &str = "AuxiliaryCodes";

/// 认的辅助码文件后缀（大小写不敏感）。
const AUXILIARY_EXTENSIONS: [&str; 2] = ["tsv", "txt"];

/// `use_input_scheme.cfg` 里的六个键（小写）：方案 id（不含后缀）、辅助码文件名（含后缀）、
/// 双键拼音的中英混合输入（`on` / `off`）、候选里显不显示辅助码（`on` / `off`）、
/// 什么时候显示键位映射参考（`off` / `hotkey` / `always`）、那套组合键（`ctrl+f1`）。
const KEY_SCHEME: &str = "scheme";
const KEY_AUXILIARY: &str = "auxiliary";
const KEY_MIXTURE_INPUT: &str = "mixture_input";
const KEY_CANDIDATE_AUXILIARY: &str = "candidate_auxiliary";
const KEY_MAP_REFERENCE: &str = "key_map_reference";
const KEY_MAP_HOTKEY: &str = "key_map_hotkey";

/// 内置样例里排第一的那份（文件名升序 = 默认方案）；`use_input_scheme.cfg` 不在时用它。
pub const DEFAULT_SAMPLE_ID: &str = "shoudao";

/// 辅助码方案的默认选择：雨棠辅助码 `yutang.tsv`（放在 `AuxiliaryCodes\` 下）。
pub const DEFAULT_AUXILIARY: &str = "yutang.tsv";

/// 方案目录：**安装目录**下的 `InputSchemes\DoubleKeysPinyin`（`bundled_root()` 下，不负责创建）。
///
/// 放安装目录是定下的规范 —— 安装程序会给这一棵目录授「普通用户可写」，
/// 设置页才能改方案文件、写 `use_input_scheme.cfg`、放辅助码表
/// （见 `apps/windows/installer/cloudime.iss` 的 `[Run]` ①.8）。
pub fn double_keys_dir() -> Option<PathBuf> {
    crate::resources::bundled_root().map(|root| root.join("InputSchemes").join("DoubleKeysPinyin"))
}

/// 目录里的一份方案。
#[derive(Debug)]
pub struct SchemeEntry {
    /// 方案 id：不含后缀的文件名（大小写按磁盘上的原样）。
    pub id: String,

    /// 文件路径。
    pub path: PathBuf,

    /// 读出来的方案；`Err` 里是失败原因（设置页用红字展示）。
    pub scheme: Result<DoubleKeysScheme, Vec<SchemeError>>,
}

impl SchemeEntry {
    /// 下拉里显示的名字：读不出来就用文件名兜底。
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
            Err(errors) => errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("；"),
        }
    }
}

/// 扫方案目录：只认 `.toml` / `.ini`，**同名只认 toml**，按文件名（大小写不敏感）升序。
/// 目录不在 / 读不动 → 空表。
pub fn list(dir: &Path) -> Vec<SchemeEntry> {
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // 小写文件名 → 选定的路径；后遇到的 toml 会顶掉先遇到的 ini。
    let mut chosen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for file in files.flatten() {
        let path = file.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if extension != "toml" && extension != "ini" {
            continue;
        }
        let stem = scheme_id(&path).to_ascii_lowercase();
        let already_toml = chosen.get(&stem).is_some_and(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("toml"))
        });
        if !already_toml {
            chosen.insert(stem, path);
        }
    }
    let mut entries: Vec<SchemeEntry> = chosen
        .into_values()
        .map(|path| {
            let scheme = load(&path);
            SchemeEntry {
                id: scheme_id(&path),
                path,
                scheme,
            }
        })
        .collect();
    entries.sort_by_key(|entry| entry.id.to_ascii_lowercase());
    entries
}

/// 读 `use_input_scheme.cfg` 的键值行（键名转小写；整行 `#` / `;` 注释与空行忽略；容忍 UTF-8 BOM）。
///
/// **只认 `键 = 值`**：老那种「只写一行方案 id」的文件读出来就是空表 —— 按规范不再支持，
/// 交给 [`resolve_current`] 重建。不是键值行的行直接跳过。
fn read_config(dir: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(dir.join(CURRENT_FILE)) else {
        return BTreeMap::new();
    };
    let mut config = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        config.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    config
}

/// 读 `use_input_scheme.cfg` 的 `scheme`（当前方案 id，不含后缀）。
pub fn read_current(dir: &Path) -> Option<String> {
    read_config(dir)
        .get(KEY_SCHEME)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// 读 `use_input_scheme.cfg` 的 `auxiliary`（辅助码文件名，含后缀）；留空 / 没写就是 `None`。
pub fn read_auxiliary(dir: &Path) -> Option<String> {
    read_config(dir)
        .get(KEY_AUXILIARY)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// 读 `use_input_scheme.cfg` 的 `mixture_input`（双键拼音的中英混合输入，缺省开）。
pub fn read_mixture_input(dir: &Path) -> bool {
    mixture_input_of(&read_config(dir))
}

/// 从已读出来的键值表里取 `mixture_input`；没写过就是**开**。
fn mixture_input_of(config: &BTreeMap<String, String>) -> bool {
    config
        .get(KEY_MIXTURE_INPUT)
        .is_none_or(|value| flag_of(value))
}

/// 读 `use_input_scheme.cfg` 的 `candidate_auxiliary`（候选里显不显示辅助码，缺省关）。
pub fn read_candidate_auxiliary(dir: &Path) -> bool {
    candidate_auxiliary_of(&read_config(dir))
}

/// 从已读出来的键值表里取 `candidate_auxiliary`；没写过就是**关**。
fn candidate_auxiliary_of(config: &BTreeMap<String, String>) -> bool {
    config
        .get(KEY_CANDIDATE_AUXILIARY)
        .is_some_and(|value| flag_of(value))
}

/// `use_input_scheme.cfg` 里的开关写法：写的是 `on` / `off`，读的时候也认 `1` / `0` / `true` / `false`。
fn flag_of(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "on" | "true" | "yes"
    )
}

/// 开关的写法。
fn flag_key(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// 读 `use_input_scheme.cfg` 的 `key_map_reference`（什么时候显示键位映射参考）。
pub fn read_key_map_reference(dir: &Path) -> KeyMapReference {
    key_map_reference_of(&read_config(dir))
}

/// 从已读出来的键值表里取 `key_map_reference`。
fn key_map_reference_of(config: &BTreeMap<String, String>) -> KeyMapReference {
    config
        .get(KEY_MAP_REFERENCE)
        .map_or(KeyMapReference::Off, |value| KeyMapReference::of(value))
}

/// 读 `use_input_scheme.cfg` 的 `key_map_hotkey`（哪套组合键）；没写过就是 `Ctrl + F1`。
pub fn read_key_map_hotkey(dir: &Path) -> KeyMapHotkey {
    key_map_hotkey_of(&read_config(dir))
}

/// 从已读出来的键值表里取 `key_map_hotkey`。
fn key_map_hotkey_of(config: &BTreeMap<String, String>) -> KeyMapHotkey {
    config
        .get(KEY_MAP_HOTKEY)
        .map_or_else(KeyMapHotkey::default, |value| KeyMapHotkey::of(value))
}

/// 当前方案的「什么时候显示键位映射参考」；拿不到方案目录就是关。
pub fn key_map_reference() -> KeyMapReference {
    double_keys_dir().map_or(KeyMapReference::Off, |dir| read_key_map_reference(&dir))
}

/// 当前方案的双键拼音「中英混合输入」；拿不到方案目录就是缺省（开）。
pub fn mixture_input() -> bool {
    double_keys_dir().is_none_or(|dir| read_mixture_input(&dir))
}

/// 当前的「在候选项显示辅助码」；拿不到方案目录就是缺省（关）。
pub fn candidate_auxiliary() -> bool {
    double_keys_dir().is_some_and(|dir| read_candidate_auxiliary(&dir))
}

/// 当前方案的「键位映射参考组合键」；拿不到方案目录就是缺省（`Ctrl + F1`）。
pub fn key_map_hotkey() -> KeyMapHotkey {
    double_keys_dir().map_or_else(KeyMapHotkey::default, |dir| read_key_map_hotkey(&dir))
}

/// 把六项一起写进 `use_input_scheme.cfg`。
fn write_config(
    dir: &Path,
    scheme: Option<&str>,
    auxiliary: Option<&str>,
    mixture_input: bool,
    candidate_auxiliary: bool,
    key_map_reference: KeyMapReference,
    key_map_hotkey: KeyMapHotkey,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = format!(
        "# 当前使用的双键拼音方案：scheme = 方案文件名（不含后缀），改这一行即可换方案。\n\
         # auxiliary = AuxiliaryCodes\\ 下的辅助码文件名（含后缀）；留空 = 不用辅助码。\n\
         # mixture_input = on / off：双键拼音下要不要按敲的按键匹配英文词（中英混合输入）；缺省 on。\n\
         # candidate_auxiliary = on / off：候选项里要不要显示辅助码（候选词右边一个空格加 [码]）；缺省 off，\n\
         #   只在「使用辅助码」开着、而且选的那份辅助码表读得出来时才有效。\n\
         # key_map_reference = off / hotkey / always：什么时候显示「键位映射参考」（新手对着敲用）——\n\
         #   off 关闭；hotkey 组句里按下面那套组合键弹出 / 收起；always 候选窗口一出现就跟着显示。\n\
         # key_map_hotkey = 上面那种方式用的组合键：修饰键 ctrl / ctrl+alt / ctrl+shift / alt+shift，\n\
         #   键 f1–f12，如 ctrl+f1。\n\
         {KEY_SCHEME} = {}\n\
         {KEY_AUXILIARY} = {}\n\
         {KEY_MIXTURE_INPUT} = {}\n\
         {KEY_CANDIDATE_AUXILIARY} = {}\n\
         {KEY_MAP_REFERENCE} = {}\n\
         {KEY_MAP_HOTKEY} = {}\n",
        scheme.unwrap_or_default(),
        auxiliary.unwrap_or_default(),
        flag_key(mixture_input),
        flag_key(candidate_auxiliary),
        key_map_reference.key(),
        key_map_hotkey.value()
    );
    cloudime_core::storage::write_atomic_str(&dir.join(CURRENT_FILE), &body)
}

/// 换当前方案（另外几行原样留着）。
pub fn set_current(dir: &Path, id: &str) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        Some(id),
        config.get(KEY_AUXILIARY).map(String::as_str),
        mixture_input_of(&config),
        candidate_auxiliary_of(&config),
        key_map_reference_of(&config),
        key_map_hotkey_of(&config),
    )
}

/// 换辅助码方案（另外几行原样留着）；`None` = 留空（不用辅助码）。
pub fn set_auxiliary(dir: &Path, file: Option<&str>) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        config.get(KEY_SCHEME).map(String::as_str),
        file,
        mixture_input_of(&config),
        candidate_auxiliary_of(&config),
        key_map_reference_of(&config),
        key_map_hotkey_of(&config),
    )
}

/// 开关双键拼音的「中英混合输入」（另外几行原样留着）。
pub fn set_mixture_input(dir: &Path, on: bool) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        config.get(KEY_SCHEME).map(String::as_str),
        config.get(KEY_AUXILIARY).map(String::as_str),
        on,
        candidate_auxiliary_of(&config),
        key_map_reference_of(&config),
        key_map_hotkey_of(&config),
    )
}

/// 开关「在候选项显示辅助码」（另外几行原样留着）。
pub fn set_candidate_auxiliary(dir: &Path, on: bool) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        config.get(KEY_SCHEME).map(String::as_str),
        config.get(KEY_AUXILIARY).map(String::as_str),
        mixture_input_of(&config),
        on,
        key_map_reference_of(&config),
        key_map_hotkey_of(&config),
    )
}

/// 改「什么时候显示键位映射参考」（另外几行原样留着）。
pub fn set_key_map_reference(dir: &Path, mode: KeyMapReference) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        config.get(KEY_SCHEME).map(String::as_str),
        config.get(KEY_AUXILIARY).map(String::as_str),
        mixture_input_of(&config),
        candidate_auxiliary_of(&config),
        mode,
        key_map_hotkey_of(&config),
    )
}

/// 改「键位映射参考组合键」（另外几行原样留着）。
pub fn set_key_map_hotkey(dir: &Path, hotkey: KeyMapHotkey) -> std::io::Result<()> {
    let config = read_config(dir);
    write_config(
        dir,
        config.get(KEY_SCHEME).map(String::as_str),
        config.get(KEY_AUXILIARY).map(String::as_str),
        mixture_input_of(&config),
        candidate_auxiliary_of(&config),
        key_map_reference_of(&config),
        hotkey,
    )
}

/// `AuxiliaryCodes\` 下的辅助码文件（`.tsv` / `.txt`，按文件名升序、大小写不敏感）。
pub fn list_auxiliary(dir: &Path) -> Vec<String> {
    let Ok(files) = std::fs::read_dir(dir.join(AUXILIARY_DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for file in files.flatten() {
        let path = file.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !AUXILIARY_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            names.push(name.to_owned());
        }
    }
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

/// 定下当前方案：`.cfg` 里指得到的就用它；否则取**第一个能读出来的**方案，并**重建** `.cfg`
/// （设计文档 §2）。目录里一个方案文件都没有 → `None`。
pub fn resolve_current(dir: &Path, entries: &[SchemeEntry]) -> Option<String> {
    let first = entries
        .iter()
        .find(|entry| entry.scheme.is_ok())
        .or_else(|| entries.first())?;
    let config = read_config(dir);
    let wanted = config
        .get(KEY_SCHEME)
        .map(String::as_str)
        .unwrap_or_default();
    match entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(wanted.trim()))
    {
        Some(entry) => Some(entry.id.clone()),
        None => {
            // `.cfg` 不在 / 是老的单行格式 / 指的方案找不到 → 重建。
            // `auxiliary` 沿用原值（`auxiliary =` 空着就是空着），从来没写过才给默认的雨棠。
            let auxiliary = config
                .get(KEY_AUXILIARY)
                .map(String::as_str)
                .or(Some(DEFAULT_AUXILIARY));
            let _ = write_config(
                dir,
                Some(&first.id),
                auxiliary,
                mixture_input_of(&config),
                candidate_auxiliary_of(&config),
                key_map_reference_of(&config),
                key_map_hotkey_of(&config),
            );
            Some(first.id.clone())
        }
    }
}

/// 目录不存在、或一个方案文件都没有时，写出**全部内置样例**（首道双拼 + 自然码）。写了就返回第一个的路径。
///
/// [`SAMPLES`] 按 id 升序，所以写完之后 [`resolve_current`] 选出来的默认方案就是首道双拼。
pub fn ensure_sample(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    // 辅助码目录顺手建出来（设置页的「辅助码方案」下拉要往里看）
    std::fs::create_dir_all(dir.join(AUXILIARY_DIR))?;
    if !list(dir).is_empty() {
        return Ok(None);
    }
    let mut first = None;
    for (id, text) in SAMPLES {
        let path = dir.join(format!("{id}.toml"));
        cloudime_core::storage::write_atomic_str(&path, text)?;
        first.get_or_insert(path);
    }
    Ok(first)
}

/// **当前方案**的键位表：读 `use_input_scheme.cfg` 指的那一份，转成 Core 解码器要的表。
/// 目录不在、一个方案都没有、当前那份读不出来 → `None`（调用方退回全键拼音）。
pub fn current_table() -> Option<cloudime_core::shuangpin::Table> {
    let dir = double_keys_dir()?;
    // 空目录先把内置样例写出来：设置页没打开过、或者 Server 先起来，也不至于一个方案都没有
    if let Err(error) = ensure_sample(&dir) {
        tracing::warn!(%error, "写双键拼音样例失败");
    }
    let entries = list(&dir);
    let current = resolve_current(&dir, &entries)?;
    let entry = entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(&current))?;
    match &entry.scheme {
        Ok(scheme) => Some(scheme.table()),
        Err(errors) => {
            let reasons: Vec<String> = errors.iter().map(|error| error.to_string()).collect();
            tracing::warn!(
                id = %entry.id,
                "双键拼音方案读不出来，这一轮退回全键拼音：{}",
                reasons.join("；")
            );
            None
        }
    }
}

/// **当前方案**的辅助码表：方案说了用辅助码、且 `use_input_scheme.cfg` 指的辅助码文件读得出来才有
/// （读不出来记一条日志、这一轮不筛）。
pub fn current_auxiliary() -> Option<AuxiliaryCodes> {
    let dir = double_keys_dir()?;
    let entries = list(&dir);
    let current = resolve_current(&dir, &entries)?;
    let entry = entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(&current))?;
    let scheme = entry.scheme.as_ref().ok()?;
    if !scheme.using_auxiliary_codes {
        return None;
    }
    let name = read_auxiliary(&dir)?;
    let path = dir.join(AUXILIARY_DIR).join(&name);
    let codes = AuxiliaryCodes::load(&path);
    if codes.is_none() {
        tracing::warn!(path = %path.display(), "辅助码文件读不出来，这一轮不做辅助码筛选");
    }
    codes
}

/// [`current_auxiliary`] 包成 Core 要的 trait 对象（Server / CLI 直接用）。
pub fn current_auxiliary_boxed() -> Option<Box<dyn cloudime_core::shuangpin::AuxiliaryTable>> {
    current_auxiliary()
        .map(|codes| Box::new(codes) as Box<dyn cloudime_core::shuangpin::AuxiliaryTable>)
}

/// **当前方案**的键位映射参考版面（「辅助窗口」画它）；方案目录不在 / 一个方案都没有 / 当前那份读不出来
/// 就是空版面。
pub fn key_map_layout() -> KeyMapLayout {
    let Some(dir) = double_keys_dir() else {
        return KeyMapLayout::default();
    };
    let entries = list(&dir);
    let Some(current) = resolve_current(&dir, &entries) else {
        return KeyMapLayout::default();
    };
    entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(&current))
        .and_then(|entry| entry.scheme.as_ref().ok())
        .map(DoubleKeysScheme::key_map_layout)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例一个自己的目录（先清掉上一次的残留）。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cloudime-input-schemes-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 一份最小可用的方案（`name` 用来断言排序与显示）。
    fn scheme_text(name: &str) -> String {
        format!(
            "[Info]\nName = \"{name}\"\nVersion = \"1.0\"\n\n[Shengmu]\na = \"AA\"\n\n[Remapping]\nv = \"zh,un,vn,uan,van,ue,ve\"\n"
        )
    }

    /// 扫描：按文件名升序、忽略大小写；同名只认 toml；别的后缀不收。
    #[test]
    fn scanning_prefers_toml_and_sorts_by_name() {
        let dir = temp_dir("scan");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.toml"), scheme_text("乙")).unwrap();
        std::fs::write(dir.join("A.ini"), scheme_text("甲-旧")).unwrap();
        std::fs::write(dir.join("a.toml"), scheme_text("甲-新")).unwrap();
        std::fs::write(dir.join("readme.txt"), "不是方案").unwrap();

        let entries = list(&dir);
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "按文件名升序、忽略大小写，txt 不收");
        assert_eq!(entries[0].label(), "甲-新", "同名只认 toml");
        assert!(
            entries[0]
                .path
                .extension()
                .is_some_and(|value| value == "toml")
        );
        assert_eq!(entries[1].label(), "乙");
        assert!(entries.iter().all(|entry| entry.scheme.is_ok()));
        assert!(entries.iter().all(|entry| entry.problems().is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 读不出来的方案也列出来，但带上原因（设置页要红字显示）。
    #[test]
    fn broken_schemes_are_listed_with_their_reasons() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.toml"), "[Info]\nName = \"缺版本\"\n").unwrap();

        let entries = list(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label(), "bad", "读不出来就用文件名兜底");
        assert!(
            entries[0].problems().contains("必填"),
            "{}",
            entries[0].problems()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 当前方案：没有 `.cfg` → 第一个 + 重建（辅助码给默认的雨棠）；指错了 → 回到第一个并重建；指对了 → 不动。
    #[test]
    fn current_falls_back_to_the_first_and_rebuilds_the_file() {
        let dir = temp_dir("current");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.toml"), scheme_text("乙")).unwrap();
        std::fs::write(dir.join("a.toml"), scheme_text("甲")).unwrap();
        let entries = list(&dir);

        assert_eq!(resolve_current(&dir, &entries).as_deref(), Some("a"));
        assert_eq!(read_current(&dir).as_deref(), Some("a"));
        assert_eq!(
            read_auxiliary(&dir).as_deref(),
            Some(DEFAULT_AUXILIARY),
            "重建时辅助码给默认的雨棠"
        );

        set_current(&dir, "没有了").unwrap();
        assert_eq!(resolve_current(&dir, &entries).as_deref(), Some("a"));
        assert_eq!(read_current(&dir).as_deref(), Some("a"));

        set_current(&dir, "B").unwrap();
        assert_eq!(
            resolve_current(&dir, &entries).as_deref(),
            Some("b"),
            "大小写不敏感"
        );
        assert_eq!(read_current(&dir).as_deref(), Some("B"), "指对了就不动文件");
        // 换方案不会把辅助码那行弄丢
        assert_eq!(read_auxiliary(&dir).as_deref(), Some(DEFAULT_AUXILIARY));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `use_input_scheme.cfg`：键值行、大小写不敏感、容忍 BOM；老那种单行格式不再认。
    #[test]
    fn the_current_file_is_key_value_lines() {
        let dir = temp_dir("current-bom");
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("# 注释\nScheme = b\nAuxiliary = yutang.tsv\n".as_bytes());
        std::fs::write(dir.join(CURRENT_FILE), bytes).unwrap();
        assert_eq!(
            read_current(&dir).as_deref(),
            Some("b"),
            "BOM + 大小写不敏感"
        );
        assert_eq!(read_auxiliary(&dir).as_deref(), Some("yutang.tsv"));

        // 老的单行格式（只有方案 id）读不出来 → 视为无效
        std::fs::write(dir.join(CURRENT_FILE), "b\n").unwrap();
        assert_eq!(read_current(&dir), None);
        assert_eq!(read_auxiliary(&dir), None);

        // `auxiliary =` 留空 = 不用辅助码
        std::fs::write(dir.join(CURRENT_FILE), "scheme = b\nauxiliary =\n").unwrap();
        assert_eq!(read_current(&dir).as_deref(), Some("b"));
        assert_eq!(read_auxiliary(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 辅助码方案：列 `AuxiliaryCodes\` 下的 `.tsv` / `.txt`（按名升序），换它不动 `scheme` 那行。
    #[test]
    fn auxiliary_files_are_listed_and_switched() {
        let dir = temp_dir("auxiliary");
        std::fs::create_dir_all(dir.join(AUXILIARY_DIR)).unwrap();
        std::fs::write(dir.join(AUXILIARY_DIR).join("Yutang.tsv"), "一\tD\n").unwrap();
        std::fs::write(dir.join(AUXILIARY_DIR).join("zb.txt"), "二\tE\n").unwrap();
        std::fs::write(dir.join(AUXILIARY_DIR).join("readme.md"), "不收").unwrap();
        std::fs::create_dir_all(dir.join(AUXILIARY_DIR).join("sub")).unwrap();

        assert_eq!(
            list_auxiliary(&dir),
            ["Yutang.tsv", "zb.txt"],
            "只有 tsv / txt，按名升序"
        );

        set_current(&dir, "shoudao").unwrap();
        set_auxiliary(&dir, Some("zb.txt")).unwrap();
        assert_eq!(read_current(&dir).as_deref(), Some("shoudao"));
        assert_eq!(read_auxiliary(&dir).as_deref(), Some("zb.txt"));

        set_auxiliary(&dir, None).unwrap();
        assert_eq!(read_auxiliary(&dir), None, "留空 = 不用辅助码");
        assert_eq!(
            read_current(&dir).as_deref(),
            Some("shoudao"),
            "scheme 那行没动"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 双键拼音的「中英混合输入」（`use_input_scheme.cfg` 的 `mixture_input`）：缺省开、写 / 读、
    /// 认 `1` / `0` / `true` / `false`，换别的设置（方案 / 辅助码 / 键位映射参考）都不该把它弄丢。
    #[test]
    fn mixture_input_round_trips_and_survives_other_writes() {
        let dir = temp_dir("mixture");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_mixture_input(&dir), "没写过 = 开");

        set_current(&dir, "shoudao").unwrap();
        set_mixture_input(&dir, false).unwrap();
        assert!(!read_mixture_input(&dir));

        // 别的写入不该把它抹回缺省
        set_auxiliary(&dir, Some("zb.tsv")).unwrap();
        set_key_map_reference(&dir, KeyMapReference::Always).unwrap();
        assert!(!read_mixture_input(&dir), "关着还是关着");
        assert_eq!(read_current(&dir).as_deref(), Some("shoudao"));

        set_mixture_input(&dir, true).unwrap();
        assert!(read_mixture_input(&dir));
        assert_eq!(
            read_auxiliary(&dir).as_deref(),
            Some("zb.tsv"),
            "别的行没动"
        );

        // 手写的 `0` / `true` 也认
        for (written, expected) in [("0", false), ("1", true), ("true", true), ("false", false)] {
            std::fs::write(
                dir.join(CURRENT_FILE),
                format!("scheme = shoudao\nmixture_input = {written}\n"),
            )
            .unwrap();
            assert_eq!(
                read_mixture_input(&dir),
                expected,
                "mixture_input = {written}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 双键拼音的「在候选项显示辅助码」（`use_input_scheme.cfg` 的 `candidate_auxiliary`）：缺省关、
    /// 写 / 读、别的行动它不丢，也认 `1` / `0` / `true` / `false`。
    #[test]
    fn candidate_auxiliary_round_trips_and_survives_other_writes() {
        let dir = temp_dir("candidate-auxiliary");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!read_candidate_auxiliary(&dir), "没写过 = 关");

        set_current(&dir, "shoudao").unwrap();
        set_candidate_auxiliary(&dir, true).unwrap();
        assert!(read_candidate_auxiliary(&dir));

        set_auxiliary(&dir, Some("zb.tsv")).unwrap();
        set_mixture_input(&dir, false).unwrap();
        assert!(read_candidate_auxiliary(&dir), "别的写入不该把它抹掉");
        assert!(!read_mixture_input(&dir));

        for (written, expected) in [("on", true), ("off", false), ("1", true), ("false", false)] {
            std::fs::write(
                dir.join(CURRENT_FILE),
                format!("scheme = shoudao\ncandidate_auxiliary = {written}\n"),
            )
            .unwrap();
            assert_eq!(
                read_candidate_auxiliary(&dir),
                expected,
                "candidate_auxiliary = {written}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 「什么时候显示键位映射参考」与「用哪套组合键」：写 / 读，且换方案 / 换辅助码方案时都不该把它弄丢。
    #[test]
    fn key_map_settings_round_trip_and_survive_other_writes() {
        let dir = temp_dir("key-map");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            read_key_map_reference(&dir),
            KeyMapReference::Off,
            "没写过 = 关"
        );
        assert_eq!(
            read_key_map_hotkey(&dir),
            KeyMapHotkey::default(),
            "没写过 = Ctrl + F1"
        );

        set_current(&dir, "shoudao").unwrap();
        set_key_map_reference(&dir, KeyMapReference::Hotkey).unwrap();
        assert_eq!(read_key_map_reference(&dir), KeyMapReference::Hotkey);
        set_key_map_hotkey(
            &dir,
            KeyMapHotkey {
                modifiers: KeyMapModifiers::AltShift,
                key: KeyMapKey::F7,
            },
        )
        .unwrap();
        assert_eq!(
            read_key_map_hotkey(&dir),
            KeyMapHotkey {
                modifiers: KeyMapModifiers::AltShift,
                key: KeyMapKey::F7
            }
        );

        set_current(&dir, "xiaohe").unwrap();
        assert_eq!(
            read_key_map_reference(&dir),
            KeyMapReference::Hotkey,
            "换方案不该把它弄丢"
        );
        assert_eq!(read_current(&dir).as_deref(), Some("xiaohe"));

        set_auxiliary(&dir, Some("zb.tsv")).unwrap();
        assert_eq!(
            read_key_map_reference(&dir),
            KeyMapReference::Hotkey,
            "换辅助码方案不该把它弄丢"
        );
        assert_eq!(
            read_key_map_hotkey(&dir).value(),
            "alt+shift+f7",
            "组合键也不该被别的写入抹掉"
        );
        assert_eq!(read_auxiliary(&dir).as_deref(), Some("zb.tsv"));

        set_key_map_reference(&dir, KeyMapReference::Always).unwrap();
        assert_eq!(read_key_map_reference(&dir), KeyMapReference::Always);
        assert_eq!(
            read_key_map_hotkey(&dir).value(),
            "alt+shift+f7",
            "改显示方式不该把组合键抹掉"
        );
        assert_eq!(
            read_current(&dir).as_deref(),
            Some("xiaohe"),
            "scheme 那行没动"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 加这两个设置之前写的是 `key_map_reference = 1` / `0`：老文件仍读得出来（`1` = 按下组合键时）。
    #[test]
    fn the_old_boolean_key_map_reference_still_reads() {
        let dir = temp_dir("key-map-legacy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(CURRENT_FILE),
            "scheme = shoudao\nauxiliary = yutang.tsv\nkey_map_reference = 1\n",
        )
        .unwrap();
        assert_eq!(read_key_map_reference(&dir), KeyMapReference::Hotkey);
        assert_eq!(read_key_map_hotkey(&dir), KeyMapHotkey::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空目录才写内置样例；样例本身必须都是合法方案，默认（第一个）是首道双拼。
    #[test]
    fn the_sample_is_written_only_when_there_is_no_scheme() {
        let dir = temp_dir("sample");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(ensure_sample(&dir).unwrap().is_some(), "空目录要写出样例");

        let entries = list(&dir);
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            ["shoudao", "xiaohe", "ziranma"],
            "三份内置样例，按文件名升序"
        );
        assert!(
            entries.iter().all(|entry| entry.scheme.is_ok()),
            "样例得是合法方案：{:?}",
            entries
                .iter()
                .map(|entry| (entry.id.as_str(), entry.problems()))
                .collect::<Vec<_>>()
        );
        assert_eq!(entries[0].label(), "首道双拼");
        assert_eq!(entries[1].label(), "小鹤双拼");
        assert_eq!(entries[2].label(), "自然码");
        assert_eq!(
            resolve_current(&dir, &entries).as_deref(),
            Some(DEFAULT_SAMPLE_ID),
            "没写 .cfg 时默认取排第一的首道双拼"
        );

        assert!(ensure_sample(&dir).unwrap().is_none(), "已经有方案就不再写");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
