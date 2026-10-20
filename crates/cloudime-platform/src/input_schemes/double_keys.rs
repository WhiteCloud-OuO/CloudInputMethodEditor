//! 双键拼音的**方案文件**：格式、解析与校验。
//!
//! 规范见 `docs/design/input-schemes.md`。这里只做「读一份文件 → 一份可用的方案」，
//! 不碰目录扫描与 `use_input_scheme.cfg`（那在下一步）。
//!
//! 两种格式（`.toml` / `.ini`）先各自读成**同一张大小写不敏感的键值表**，再走同一套 [`validate`]，
//! 保证两种写法行为一致；只有 `[Exchange]` 是 TOML 独有（INI 里出现 `[Exchange]` 判无效）。
//! 文件一律 UTF-8（容忍 BOM），否则该方案无效。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::key_map::{KEYBOARD_ROWS, KeyCap, KeyMapLayout};

/// 占键的翘舌声母。`Remapping` 的值里出现这几个之一，那一项就当声母，其余当韵母。
const INITIALS: [&str; 3] = ["zh", "ch", "sh"];

/// 合法韵母（`v` = ü）。用来抓拼音表的笔误：写错的韵母会让整个方案无效。
///
/// `v` 那一族的五个（`v` = ü、`ve` = üe、`vn` = ün、`van` = üan）与 `u` 那一族的
/// （`u`、`ue`、`un`、`uan`）**各自独立**，不做互相转换（见 `docs/design/input-schemes.md` §5）。
const FINALS: [&str; 36] = [
    "a", "o", "e", "i", "u", "v", "ai", "ei", "ui", "ao", "ou", "iu", "ie", "ve", "ue", "er", "an",
    "en", "in", "un", "vn", "ang", "eng", "ing", "ong", "iong", "ia", "ua", "uo", "uai", "iao",
    "ian", "iang", "uan", "van", "uang",
];

/// 认得的节（小写）。
const SECTIONS: [&str; 5] = ["info", "auxiliary", "exchange", "remapping", "shengmu"];

/// 能单独成一个音节的韵母（零声母）：`[Shengmu]` 的左边（键）只认这些。
const ZERO_FINALS: [&str; 12] = [
    "a", "o", "e", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "er",
];
/// 一个键的映射：声母与韵母分开收。同键多义时韵母**全都留着**（两个都匹配，让词库决定）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMapping {
    /// 声母（`zh` / `ch` / `sh`）；通常为空。
    pub initials: Vec<String>,

    /// 韵母，按文件里写的顺序。
    pub finals: Vec<String>,
}

/// 一份双键拼音方案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleKeysScheme {
    /// 不含后缀的文件名（方案 id）；文件本身不写这一项。
    pub id: String,

    /// 界面上的方案名称（`[Info] Name`）。
    pub name: String,

    /// 版本（`[Info] Version`）：迁移判据。
    pub version: String,

    /// 作者（选填）。
    pub author: Option<String>,

    /// 零声母：`[Shengmu]` 里「零声母韵母 → 要按的两键」转成的「两键 → 韵母」（键已归一成小写）。
    pub zero_initials: BTreeMap<(char, char), String>,

    /// 词库文件（相对方案目录）；`None` = 用软件默认词库。
    pub word_bank: Option<PathBuf>,

    /// 这个方案开不开辅助码（方案文件的 `[Auxiliary] UsingAuxiliaryCodes`）。
    ///
    /// **没有「辅助码位置」这一项**：辅助码**逐字对应** —— 每个字一位、最多 [`MAX_AUXILIARY`] 位，
    /// 敲在它那个音节后面（`aiPD` = 「爱」+ 它的码 `PD`）。
    /// **用哪个辅助码文件也不在这里** —— 那是用户级的，记在 `use_input_scheme.cfg` 的 `auxiliary` 行、
    /// 由设置页的「辅助码方案」下拉来改（文件都放方案目录下的 `AuxiliaryCodes\`）。
    pub using_auxiliary_codes: bool,

    /// 物理键互换（`y-z` = y 与 z 互换）；禁止链式。
    pub exchange: Vec<(char, char)>,

    /// 键 → 声母 / 韵母。
    pub remapping: BTreeMap<char, KeyMapping>,
}

impl DoubleKeysScheme {
    /// `Remapping` 里有没有用到 `;` —— 有才把它吃进组句（微软 / 搜狗的 `ing`）。
    pub fn uses_semicolon(&self) -> bool {
        self.remapping.contains_key(&';')
    }

    /// 键位映射参考的版面：三排键盘格子（每格「左上角那颗键 + 右上角的声母 + 中间几行韵母」，
    /// 按 [`KEYBOARD_ROWS`] 的顺序）+ 零声母表（`[Shengmu]` 的每一项一格：两键 + 韵母）。
    pub fn key_map_layout(&self) -> KeyMapLayout {
        let rows = KEYBOARD_ROWS
            .iter()
            .map(|row| {
                row.chars()
                    .map(|key| {
                        let mapping = self.remapping.get(&key);
                        KeyCap {
                            key: key.to_ascii_uppercase().to_string(),
                            initial: mapping
                                .map(|mapping| mapping.initials.join("/"))
                                .unwrap_or_default(),
                            mapping: mapping
                                .map(|mapping| {
                                    mapping
                                        .finals
                                        .iter()
                                        .map(|final_| display_final(final_))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                                .unwrap_or_default(),
                            shifted: String::new(),
                        }
                    })
                    .collect()
            })
            .collect();
        let zero_initials = self
            .zero_initials
            .iter()
            .map(|((first, second), final_)| KeyCap {
                key: format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    second.to_ascii_uppercase()
                ),
                initial: String::new(),
                mapping: display_final(final_),
                shifted: String::new(),
            })
            .collect();
        KeyMapLayout {
            rows,
            zero_initials,
            footer: None,
            symbol: false,
            fallback_family: None,
            fallback_first: false,
        }
    }

    /// 转成 Core 解码器要的那张表（Core 不认识方案文件，只认识这张表）。
    pub fn table(&self) -> cloudime_core::shuangpin::Table {
        cloudime_core::shuangpin::Table {
            zero_initials: self.zero_initials.clone(),
            remapping: self
                .remapping
                .iter()
                .map(|(key, mapping)| {
                    (
                        *key,
                        cloudime_core::shuangpin::KeyMapping {
                            initials: mapping.initials.clone(),
                            finals: mapping.finals.clone(),
                        },
                    )
                })
                .collect(),
            exchange: self.exchange.clone(),
            auxiliary: self.using_auxiliary_codes,
        }
    }
}

/// 韵母在辅助窗口里的写法：`v`（ü）写成 `ü`（`van` → `üan`、`ve` → `üe`、`v` → `ü`）。
///
/// 方案文件里一律写 `v`（与词库一致），显示成 ü 才是拼音的写法。
fn display_final(final_: &str) -> String {
    final_.replace('v', "ü")
}

/// 方案文件读不出来 / 不合规的原因。一次列全，交给设置页用红字展示。
#[derive(Debug, thiserror::Error)]
pub enum SchemeError {
    /// 文件读不动。
    #[error("读不了 {path}：{source}")]
    Read {
        /// 文件路径。
        path: PathBuf,
        /// 底层错误。
        source: std::io::Error,
    },

    /// 不是 UTF-8。
    #[error("{path} 不是 UTF-8 编码")]
    NotUtf8 {
        /// 文件路径。
        path: PathBuf,
    },

    /// 后缀不认识。
    #[error("{path} 的后缀不认识（只认 .toml / .ini）")]
    UnknownFormat {
        /// 文件路径。
        path: PathBuf,
    },

    /// 语法有错（INI / TOML 本身读不动）。
    #[error("{path} 语法有错：{message}")]
    Syntax {
        /// 文件路径。
        path: PathBuf,
        /// 具体哪一行 / 哪个键。
        message: String,
    },

    /// 必填项没写。
    #[error("[{section}] 的 {key} 必填")]
    Missing {
        /// 节名。
        section: &'static str,
        /// 键名。
        key: &'static str,
    },

    /// 取值不对。
    #[error("[{section}] 的 {key} 取值不对：{message}")]
    BadValue {
        /// 节名。
        section: &'static str,
        /// 键名。
        key: String,
        /// 为什么不对。
        message: String,
    },

    /// 其它不合规（未知节 / 未知键 / 交换键链式…）。
    #[error("{0}")]
    Invalid(String),
}

/// 一份文件的原始内容：`[节(小写)] → { 键(小写) → 值 }`。
type RawSections = BTreeMap<String, BTreeMap<String, String>>;

/// 读一份方案文件：按后缀选 TOML / INI，读成同一张表再校验。
pub fn load(path: &Path) -> Result<DoubleKeysScheme, Vec<SchemeError>> {
    let bytes = std::fs::read(path).map_err(|source| {
        vec![SchemeError::Read {
            path: path.to_owned(),
            source,
        }]
    })?;
    let text = String::from_utf8(strip_bom(bytes)).map_err(|_| {
        vec![SchemeError::NotUtf8 {
            path: path.to_owned(),
        }]
    })?;
    let syntax = |message: String| {
        vec![SchemeError::Syntax {
            path: path.to_owned(),
            message,
        }]
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (format, raw) = match extension.as_str() {
        "toml" => (Format::Toml, flatten_toml(&text).map_err(syntax)?),
        "ini" => (Format::Ini, flatten_ini(&text).map_err(syntax)?),
        _ => {
            return Err(vec![SchemeError::UnknownFormat {
                path: path.to_owned(),
            }]);
        }
    };
    validate(scheme_id(path), format, raw)
}

/// 不含后缀的文件名 = 方案 id（后缀 `.toml` / `.ini` 大小写都认）。
pub fn scheme_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// 改写方案文件里的 `[Auxiliary] UsingAuxiliaryCodes`（设置页那个开关用它），**其余内容原样不动**。
///
/// 有 `[Auxiliary]` 节就改它里面的那一行、没有就插在节标题后面；连节都没有就在文件末尾补一节。
/// 值按后缀来：TOML 写 `true` / `false`，INI 写 `1` / `0`（两种写法解析时互相也认）。
pub fn set_using_auxiliary_codes(path: &Path, on: bool) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let ini = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ini"));
    let value = match (ini, on) {
        (true, true) => "1",
        (true, false) => "0",
        (false, true) => "true",
        (false, false) => "false",
    };
    cloudime_core::storage::write_atomic_str(path, &rewrite_auxiliary_flag(&text, value))
}

/// 纯文本改写：只动 `[Auxiliary]` 节里的 `UsingAuxiliaryCodes` 一行（节名 / 键名大小写不敏感），
/// 换行符是 `\r\n` 就照 `\r\n`，注释与其它行一律不动。
fn rewrite_auxiliary_flag(text: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    // 是节标题就给出小写的节名（`[Auxiliary]` → `auxiliary`）。
    let section_name = |line: &str| -> Option<String> {
        let name = line.trim().strip_prefix('[')?.strip_suffix(']')?;
        Some(name.trim().to_ascii_lowercase())
    };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let entry = format!("UsingAuxiliaryCodes = {value}");
    match lines
        .iter()
        .position(|line| section_name(line).as_deref() == Some("auxiliary"))
    {
        Some(header) => {
            let end = lines
                .iter()
                .enumerate()
                .skip(header + 1)
                .find_map(|(index, line)| section_name(line).is_some().then_some(index))
                .unwrap_or(lines.len());
            let flag = (header + 1..end).find(|index| {
                lines[*index]
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("UsingAuxiliaryCodes"))
            });
            match flag {
                Some(index) => lines[index] = entry,
                None => lines.insert(header + 1, entry),
            }
        }
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[Auxiliary]".to_owned());
            lines.push(entry);
        }
    }
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

/// 文件格式（只有 `[Exchange]` 一节按它区分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Toml,
    Ini,
}

/// 去掉 UTF-8 BOM（`toml` 认，但 INI 是自己解析的）。
fn strip_bom(bytes: Vec<u8>) -> Vec<u8> {
    match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => rest.to_vec(),
        None => bytes,
    }
}

/// INI → 同一张表。节名与键名一律转小写；只支持整行注释（`#` / `;`）。
fn flatten_ini(text: &str) -> Result<RawSections, String> {
    let mut sections: RawSections = BTreeMap::new();
    let mut current: Option<String> = None;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let name = name.trim().to_ascii_lowercase();
            if name.is_empty() {
                return Err(format!("第 {} 行：节名是空的", index + 1));
            }
            sections.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("第 {} 行既不是 `[节]` 也不是 `键 = 值`", index + 1));
        };
        let Some(section) = current.clone() else {
            return Err(format!(
                "第 {} 行：`{}` 不在任何 `[节]` 里",
                index + 1,
                key.trim()
            ));
        };
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() {
            return Err(format!("第 {} 行：键名是空的", index + 1));
        }
        let value = unquote(value.trim());
        let table = sections.get_mut(&section).expect("刚建过的节");
        if table.insert(key.clone(), value).is_some() {
            return Err(format!("[{section}] 的 {key} 写了两次"));
        }
    }
    Ok(sections)
}

/// 去掉值外面的一层引号（`"自然码"` / `'自然码'` 都行）。
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
}

/// TOML → 同一张表。数组按逗号拼成字符串（`Exchange` 的 `swap` 用它）。
fn flatten_toml(text: &str) -> Result<RawSections, String> {
    let value: toml::Value =
        toml::from_str(text).map_err(|error: toml::de::Error| error.to_string())?;
    let table = value
        .as_table()
        .ok_or_else(|| "顶层不是一个表".to_owned())?;
    let mut sections: RawSections = BTreeMap::new();
    for (section, body) in table {
        let body = body
            .as_table()
            .ok_or_else(|| format!("[{section}] 不是一个表"))?;
        let mut entries = BTreeMap::new();
        for (key, value) in body {
            let value = flatten_toml_value(value)
                .ok_or_else(|| format!("[{section}] 的 {key} 只能是标量或字符串数组"))?;
            entries.insert(key.to_ascii_lowercase(), value);
        }
        sections.insert(section.to_ascii_lowercase(), entries);
    }
    Ok(sections)
}

/// TOML 标量 → 字符串；数组按逗号拼接。
fn flatten_toml_value(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(text) => Some(text.clone()),
        toml::Value::Boolean(flag) => Some(flag.to_string()),
        toml::Value::Integer(number) => Some(number.to_string()),
        toml::Value::Float(number) => Some(number.to_string()),
        toml::Value::Datetime(time) => Some(time.to_string()),
        toml::Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                parts.push(flatten_toml_value(item)?);
            }
            Some(parts.join(","))
        }
        toml::Value::Table(_) => None,
    }
}

/// 校验并组装。一次把问题列全，调用方（设置页）逐条显示。
fn validate(
    id: String,
    format: Format,
    raw: RawSections,
) -> Result<DoubleKeysScheme, Vec<SchemeError>> {
    let mut errors: Vec<SchemeError> = Vec::new();

    // 未知的节 / 键不静默吞掉：用户多半是拼错了（`Remapping` / `Shengmu` 的键是任意的，不看）。
    for (section, keys) in &raw {
        if !SECTIONS.contains(&section.as_str()) {
            errors.push(SchemeError::Invalid(format!(
                "不认识的节 [{section}]（只认 Info / Auxiliary / Exchange / Remapping / Shengmu）"
            )));
            continue;
        }
        if section == "remapping" || section == "shengmu" {
            continue;
        }
        for key in keys.keys() {
            if !known_keys(section).contains(&key.as_str()) {
                errors.push(SchemeError::Invalid(format!(
                    "[{section}] 里不认识的键 {key}"
                )));
            }
        }
    }

    let name = required(&raw, "Info", "Name", &mut errors);
    let version = required(&raw, "Info", "Version", &mut errors);
    let author = optional(&raw, "info", "author");
    let word_bank = optional(&raw, "info", "wordbank")
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from);

    let using_auxiliary_codes = auxiliary(&raw, &mut errors);
    let exchange = exchange(format, &raw, &mut errors);
    let remapping = remapping(&raw, &mut errors);
    let zero_initials = shengmu(&raw, &mut errors);
    required_finals(&remapping, &mut errors);

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(DoubleKeysScheme {
        id,
        name: name.expect("校验过"),
        version: version.expect("校验过"),
        author,
        zero_initials,
        word_bank,
        using_auxiliary_codes,
        exchange,
        remapping,
    })
}

/// 某个节里认得的键（`Remapping` 的键是任意的，不在表里）。
fn known_keys(section: &str) -> &'static [&'static str] {
    match section {
        "info" => &["name", "version", "author", "wordbank"],
        "auxiliary" => &["usingauxiliarycodes"],
        "exchange" => &["swap"],
        _ => &[],
    }
}

/// 取一个必填的字符串。
fn required(
    raw: &RawSections,
    section: &'static str,
    key: &'static str,
    errors: &mut Vec<SchemeError>,
) -> Option<String> {
    let value = optional(
        raw,
        &section.to_ascii_lowercase(),
        &key.to_ascii_lowercase(),
    );
    match value.filter(|text| !text.trim().is_empty()) {
        Some(text) => Some(text.trim().to_owned()),
        None => {
            errors.push(SchemeError::Missing { section, key });
            None
        }
    }
}

/// 取一个选填的字符串。
fn optional(raw: &RawSections, section: &str, key: &str) -> Option<String> {
    raw.get(section)?.get(key).cloned()
}

/// 取一个布尔：TOML 的 `true`/`false` 与 INI 的 `0`/`1` 都认（大小写不敏感）。
fn flag(raw: &RawSections, section: &str, key: &str) -> Option<Result<bool, String>> {
    let text = optional(raw, section, key)?;
    let text = text.trim().to_ascii_lowercase();
    Some(match text.as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(text),
    })
}

/// `[Remapping]` 必须写全的六个韵母：`ü` 那一族（`vn` / `van` / `ve`）与 `u` 那一族（`un` / `uan` / `ue`）
/// 各自独立（见 `docs/design/input-schemes.md` §5），少一个就说不清这个方案怎么打那几个音 —— 判无效。
const REQUIRED_FINALS: [&str; 6] = ["un", "vn", "uan", "van", "ue", "ve"];

/// 六个韵母都要在 `[Remapping]` 里出现过（哪个键上不限，同键多义照旧允许）。
fn required_finals(remapping: &BTreeMap<char, KeyMapping>, errors: &mut Vec<SchemeError>) {
    let missing: Vec<&str> = REQUIRED_FINALS
        .iter()
        .copied()
        .filter(|final_| {
            !remapping
                .values()
                .any(|mapping| mapping.finals.iter().any(|value| value == final_))
        })
        .collect();
    if !missing.is_empty() {
        errors.push(SchemeError::Invalid(format!(
            "[Remapping] 里缺韵母 {}（ü 那一族 vn / van / ve 与 u 那一族 un / uan / ue 各自独立，六个都得写）",
            missing.join(" / ")
        )));
    }
}

/// `[Auxiliary]`：只管「用不用辅助码」（`UsingAuxiliaryCodes`）。**没有「辅助码位置」** ——
/// 辅助码逐字对应（敲在它那个音节后面），用哪个辅助码文件也不在这里（`use_input_scheme.cfg` 的 `auxiliary`）。
fn auxiliary(raw: &RawSections, errors: &mut Vec<SchemeError>) -> bool {
    match flag(raw, "auxiliary", "usingauxiliarycodes") {
        Some(Ok(value)) => value,
        Some(Err(text)) => {
            errors.push(SchemeError::BadValue {
                section: "Auxiliary",
                key: "UsingAuxiliaryCodes".to_owned(),
                message: format!("`{text}` 不是 0 / 1 / true / false"),
            });
            false
        }
        None => false,
    }
}

/// `[Exchange]`：物理键互换，仅 TOML；每个键最多出现在一对里（禁止链式）。
fn exchange(format: Format, raw: &RawSections, errors: &mut Vec<SchemeError>) -> Vec<(char, char)> {
    let Some(keys) = raw.get("exchange") else {
        return Vec::new();
    };
    if format == Format::Ini {
        errors.push(SchemeError::Invalid(
            "[Exchange] 只支持 TOML（INI 里写不了物理键互换）".to_owned(),
        ));
        return Vec::new();
    }
    let mut pairs = Vec::new();
    let text = keys.get("swap").cloned().unwrap_or_default();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let pair = part.split_once('-').and_then(|(left, right)| {
            let left = single_key(left.trim())?;
            let right = single_key(right.trim())?;
            (left != right).then_some((left, right))
        });
        let Some((left, right)) = pair else {
            errors.push(SchemeError::Invalid(format!(
                "[Exchange] 的 `{part}` 不是「一个键-另一个键」（如 `y-z`）"
            )));
            continue;
        };
        if pairs
            .iter()
            .any(|(a, b)| [*a, *b].contains(&left) || [*a, *b].contains(&right))
        {
            errors.push(SchemeError::Invalid(format!(
                "[Exchange] 的 `{part}` 与别的交换对撞了同一个键（禁止链式）"
            )));
            continue;
        }
        pairs.push((left, right));
    }
    pairs
}

/// 单个按键：`a`-`z` 或 `;`。**全角 `；` 也算 `;`** —— INI 里半角分号是注释，中文用户自然写全角那个。
fn single_key(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let letter = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let letter = if letter == '；' {
        ';'
    } else {
        letter.to_ascii_lowercase()
    };
    (letter.is_ascii_lowercase() || letter == ';').then_some(letter)
}

/// `[Shengmu]`：零声母韵母 → 要按的两键（`ang = AY` 就是依次敲 A、Y）。
/// 左边必是能单独成音节的韵母，右边必是两个字母；两键归一成小写。
fn shengmu(raw: &RawSections, errors: &mut Vec<SchemeError>) -> BTreeMap<(char, char), String> {
    let Some(keys) = raw.get("shengmu") else {
        errors.push(SchemeError::Invalid(
            "缺 [Shengmu] 节（零声母怎么打是必填的）".to_owned(),
        ));
        return BTreeMap::new();
    };
    let mut mapping: BTreeMap<(char, char), String> = BTreeMap::new();
    for (final_, value) in keys {
        let final_ = final_.trim().to_ascii_lowercase();
        if !ZERO_FINALS.contains(&final_.as_str()) {
            errors.push(SchemeError::BadValue {
                section: "Shengmu",
                key: final_,
                message: "不是能单独成音节的韵母（只认 a / o / e / ai / ei / ao / ou / an / en / ang / eng / er）"
                    .to_owned(),
            });
            continue;
        }
        let Some(pair) = two_keys(value.trim()) else {
            errors.push(SchemeError::BadValue {
                section: "Shengmu",
                key: final_,
                message: format!("`{}` 不是两个字母", value.trim()),
            });
            continue;
        };
        if let Some(previous) = mapping.insert(pair, final_.clone()) {
            errors.push(SchemeError::Invalid(format!(
                "[Shengmu] 的 `{final_}` 与 `{previous}` 用了同一组按键"
            )));
        }
    }
    mapping
}

/// 两键（`AY` → `('a','y')`）；不是正好两个字母就是 `None`。
fn two_keys(text: &str) -> Option<(char, char)> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if chars.next().is_some() || !first.is_ascii_alphabetic() || !second.is_ascii_alphabetic() {
        return None;
    }
    Some((first.to_ascii_lowercase(), second.to_ascii_lowercase()))
}

/// `[Remapping]`：键 → 声母 / 韵母。整节缺失、键不是单个字母、韵母拼错都算错。
fn remapping(raw: &RawSections, errors: &mut Vec<SchemeError>) -> BTreeMap<char, KeyMapping> {
    let Some(keys) = raw.get("remapping") else {
        errors.push(SchemeError::Invalid(
            "缺 [Remapping] 节（键位表是必填的）".to_owned(),
        ));
        return BTreeMap::new();
    };
    let mut mapping = BTreeMap::new();
    for (key, value) in keys {
        let Some(letter) = single_key(key) else {
            errors.push(SchemeError::Invalid(format!(
                "[Remapping] 的键 `{key}` 必须是单个字母（a-z）或 `;`"
            )));
            continue;
        };
        let mut entry = KeyMapping::default();
        for part in value.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            if INITIALS.contains(&part.as_str()) {
                entry.initials.push(part);
            } else if FINALS.contains(&part.as_str()) {
                entry.finals.push(part);
            } else {
                errors.push(SchemeError::BadValue {
                    section: "Remapping",
                    key: letter.to_string(),
                    message: format!("`{part}` 既不是声母（zh / ch / sh）也不是认识的韵母"),
                });
            }
        }
        mapping.insert(letter, entry);
    }
    mapping
}

/// 首次运行写出的内置样例（首道双拼；键位表与零声母表取自青简时期的 `shuangpin/table.rs`）。
pub const SAMPLE_SHOUDAO: &str = r#"# 双键拼音方案：首道双拼。字段说明见 docs/design/input-schemes.md。
# 改完点设置页的「应用方案」生效；同一目录下同名 .toml 优先于 .ini。

[Info]
Name = "首道双拼"
Version = "1.0"
Author = "云朵输入法"
WordBank = ""

[Auxiliary]
UsingAuxiliaryCodes = true

# 零声母：左边是韵母，右边是依次要按的两个键（`ang = AY` 就是打 A、Y）
[Shengmu]
a = "AA"
e = "UE"
o = "OO"
ai = "AI"
ei = "UI"
ou = "OU"
an = "AN"
en = "EN"
ang = "AY"
er = "ER"
ao = "AO"
eng = "UF"

[Remapping]
# 声母：v = zh、i = ch、e = sh（首道的 sh 在 e 键上）
v = "zh,ui,v"
i = "ch,i"
e = "sh,e"
# 其余键 = 韵母；一个键多个韵母用逗号分隔（两个都匹配）
a = "a"
b = "ve"
c = "in"
d = "ao"
f = "eng"
g = "uai,ing"
h = "ong,iong"
j = "an"
k = "en,ia"
l = "ai,ue"
m = "ei"
n = "ian"
o = "uo,o"
p = "iao"
q = "iu"
r = "ie"
s = "ou"
t = "uan,van"
u = "u"
w = "ua"
x = "iang,uang"
y = "ang"
z = "un,vn"
"#;

/// 首次运行写出的内置样例（小鹤双拼；键位表与零声母表取自青简时期的 `shuangpin/table.rs`）。
pub const SAMPLE_XIAOHE: &str = r#"# 双键拼音方案：小鹤双拼。字段说明见 docs/design/input-schemes.md。
# 改完点设置页的「应用方案」生效；同一目录下同名 .toml 优先于 .ini。

[Info]
Name = "小鹤双拼"
Version = "1.0"
Author = "云朵输入法"
WordBank = ""

[Auxiliary]
UsingAuxiliaryCodes = false

# 零声母：左边是韵母，右边是依次要按的两个键
[Shengmu]
a = "AA"
o = "OO"
e = "EE"
ai = "AI"
ei = "EI"
ao = "AO"
ou = "OU"
an = "AN"
en = "EN"
ang = "AH"
eng = "EG"
er = "ER"

[Remapping]
# 声母：v = zh、i = ch、u = sh（小鹤与自然码同用这三个键）
v = "zh,ui,v"
i = "ch,i"
u = "sh,u"
# 其余键 = 韵母（一个键多个韵母用逗号分隔，两个都匹配）
a = "a"
b = "in"
c = "ao"
d = "ai"
e = "e"
f = "en"
g = "eng"
h = "ang"
j = "an"
k = "ing,uai"
l = "iang,uang"
m = "ian"
n = "iao"
o = "uo,o"
p = "ie"
q = "iu"
r = "uan,van"
s = "iong,ong"
t = "ve,ue"
w = "ei"
x = "ia,ua"
y = "un,vn"
z = "ou"
"#;

/// 首次运行写出的全部内置样例：`(方案 id, 文件内容)`。**按 id 升序** ——
/// `use_input_scheme.cfg` 不存在时取「找到的第一个」，所以排第一的 `shoudao` 就是默认方案。
pub const SAMPLES: [(&str, &str); 3] = [
    ("shoudao", SAMPLE_SHOUDAO),
    ("xiaohe", SAMPLE_XIAOHE),
    ("ziranma", SAMPLE_ZIRANMA),
];

/// 首次运行写出的内置样例（自然码；键位表取自青简时期的 `shuangpin/table.rs`）。
pub const SAMPLE_ZIRANMA: &str = r#"# 双键拼音方案：自然码。字段说明见 docs/design/input-schemes.md。
# 改完立即生效；同一目录下同名 .toml 优先于 .ini。

[Info]
Name = "自然码"
Version = "1.0"
Author = "云朵输入法"
# 词库文件（相对本目录）。留空 = 用软件默认词库
WordBank = ""

[Auxiliary]
UsingAuxiliaryCodes = false

# 零声母：左边是韵母，右边是依次要按的两个键
[Shengmu]
a = "AA"
o = "OO"
e = "EE"
ai = "AI"
ei = "EI"
ao = "AO"
ou = "OU"
an = "AN"
en = "EN"
ang = "AH"
eng = "EG"
er = "ER"

[Remapping]
# 声母：值里出现 zh / ch / sh 的项即声母；同一个键按位置分别当声母与韵母
v = "zh,ui,v"
i = "ch,i"
u = "sh,u"
# 其余键 = 韵母；一个键多个韵母用逗号分隔（两个都匹配）
a = "a"
b = "ou"
c = "iao"
d = "iang,uang"
e = "e"
f = "en"
g = "eng"
h = "ang"
j = "an"
k = "ao"
l = "ai"
m = "ian"
n = "in"
o = "uo,o"
p = "un,vn"
q = "iu"
r = "uan,van"
s = "iong,ong"
t = "ve,ue"
w = "ia,ua"
x = "ie"
y = "uai,ing"
z = "ei"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// 六个必填韵母（校验要求都在 `[Remapping]` 里出现过）：测试的短方案统一补上，
    /// 挂在测试里基本用不到的 `q` / `w` / `e` / `r` / `t` / `y` 上（那几个键已经有内容就往后接）。
    fn with_finals(mut raw: RawSections) -> RawSections {
        let remapping = raw.entry("remapping".to_owned()).or_default();
        // 先看这六个是不是已经在某个键上了：在的就不动（免得同一个韵母挂两遍）
        let mut missing: Vec<(char, &str)> = Vec::new();
        for (key, final_) in [
            ('q', "un"),
            ('w', "vn"),
            ('e', "uan"),
            ('r', "van"),
            ('t', "ue"),
            ('y', "ve"),
        ] {
            let present = remapping
                .values()
                .any(|value| value.split(',').any(|value| value.trim() == final_));
            if !present {
                missing.push((key, final_));
            }
        }
        for (key, final_) in missing {
            let entry = remapping.entry(key.to_string()).or_default();
            if entry.trim().is_empty() {
                *entry = final_.to_owned();
            } else {
                entry.push(',');
                entry.push_str(final_);
            }
        }
        raw
    }

    /// 六个必填韵母（`un` / `vn` / `uan` / `van` / `ue` / `ve`）少一个就判无效 ——
    /// ü 那一族与 u 那一族各自独立，方案得说清这六个音怎么打（走 [`super::validate`]，不补韵母）。
    #[test]
    fn the_six_required_finals_must_all_be_written_out() {
        let complete = "[Info]\nName = \"s\"\nVersion = \"1\"\n\n[Shengmu]\na = \"AA\"\n\n[Remapping]\nz = \"un,vn\"\nt = \"uan,van\"\nl = \"ue,ve\"\n";
        assert!(
            super::validate(
                "s".to_owned(),
                Format::Toml,
                flatten_toml(complete).expect("TOML 合法")
            )
            .is_ok(),
            "六个写全就过"
        );

        // 少一个 `vn`：报出来，且只提它
        let missing = complete.replace(",vn", "");
        let errors = super::validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(&missing).expect("TOML 合法"),
        )
        .expect_err("缺 vn 该判无效");
        let text = errors
            .iter()
            .map(|error| error.to_string())
            .collect::<String>();
        assert!(
            text.starts_with("[Remapping] 里缺韵母 vn（"),
            "只提缺的那个 vn：{text}"
        );
    }

    /// 测试里调 [`super::validate`]：先把六个必填韵母补进去，短方案不必每个都写全。
    fn validate(
        id: String,
        format: Format,
        raw: RawSections,
    ) -> Result<DoubleKeysScheme, Vec<SchemeError>> {
        super::validate(id, format, with_finals(raw))
    }
    /// 内置样例必须能读出来，而且键位与青简那份表一致。
    #[test]
    fn the_built_in_sample_parses() {
        let scheme = validate(
            "ziranma".to_owned(),
            Format::Toml,
            flatten_toml(SAMPLE_ZIRANMA).expect("样例是合法 TOML"),
        )
        .expect("样例能过校验");
        assert_eq!(scheme.id, "ziranma");
        assert_eq!(scheme.name, "自然码");
        assert_eq!(scheme.version, "1.0");
        assert_eq!(scheme.author.as_deref(), Some("云朵输入法"));
        assert_eq!(scheme.zero_initials.len(), 12, "自然码 12 组零声母");
        assert_eq!(scheme.zero_initials[&('a', 'a')], "a");
        assert_eq!(scheme.zero_initials[&('a', 'h')], "ang");
        assert_eq!(scheme.zero_initials[&('o', 'u')], "ou");
        assert!(scheme.word_bank.is_none(), "留空 = 用默认词库");
        assert!(!scheme.using_auxiliary_codes, "没开辅助码");
        assert!(scheme.exchange.is_empty());
        assert!(!scheme.uses_semicolon(), "自然码不用 ;");
        assert_eq!(scheme.remapping.len(), 26, "26 个字母各一条");
        // 声母与韵母分开收，同一个键两义并存
        let v = &scheme.remapping[&'v'];
        assert_eq!(v.initials, ["zh"]);
        assert_eq!(v.finals, ["ui", "v"]);
        assert_eq!(scheme.remapping[&'i'].finals, ["i"]);
        // 同键多义两个都留着
        assert_eq!(scheme.remapping[&'t'].finals, ["ve", "ue"]);
        assert_eq!(
            scheme.remapping[&'p'].finals,
            ["un", "vn"],
            "ün 挂在 un 那个键上（与 un 同键多义）"
        );
        assert_eq!(scheme.remapping[&'r'].finals, ["uan", "van"]);
        assert_eq!(scheme.remapping[&'d'].finals, ["iang", "uang"]);
        // 没写的键就是没有
        assert!(scheme.remapping[&'f'].finals == ["en"]);
    }

    /// 首道（内置缺省的方案）**缺省就打开辅助码**：随包带了那两张首右辅助码，新用户装上就能用。
    #[test]
    fn the_shoudao_sample_opens_auxiliary_codes() {
        let scheme = validate(
            "shoudao".to_owned(),
            Format::Toml,
            flatten_toml(SAMPLE_SHOUDAO).expect("样例是合法 TOML"),
        )
        .expect("样例能过校验");
        assert!(scheme.using_auxiliary_codes, "首道缺省开辅助码");
        assert!(scheme.table().auxiliary);
    }

    /// 内置样例转成 Core 的解码器表后，直接就能解 `ypdo` → `yun'duo`。
    #[test]
    fn the_sample_becomes_a_usable_table() {
        let scheme = validate(
            "ziranma".to_owned(),
            Format::Toml,
            flatten_toml(SAMPLE_ZIRANMA).expect("样例是合法 TOML"),
        )
        .expect("样例能过校验");
        let table = scheme.table();
        assert_eq!(table.zero_initials[&('a', 'a')], "a");
        assert_eq!(table.zero_initials[&('a', 'h')], "ang");
        assert!(!table.auxiliary, "样例没开辅助码");
        assert!(table.exchange.is_empty());
        // 转完就能解：ypdo → yun'duo
        let pinyin: Vec<String> = table
            .expand("ypdo")
            .iter()
            .map(cloudime_core::shuangpin::Expansion::to_pinyin)
            .collect();
        assert!(pinyin.contains(&"yun'duo".to_owned()), "{pinyin:?}");
        // 零声母也跟着过去了：aa → a
        let zero: Vec<String> = table
            .expand("aa")
            .iter()
            .map(cloudime_core::shuangpin::Expansion::to_pinyin)
            .collect();
        assert!(zero.contains(&"a".to_owned()), "{zero:?}");
        // 声母也跟着过去了
        assert_eq!(table.remapping[&'v'].initials, ["zh"]);
    }

    /// 内置样例：小鹤的键位与零声母都对得上。
    #[test]
    fn the_xiaohe_sample_parses() {
        let scheme = validate(
            "xiaohe".to_owned(),
            Format::Toml,
            flatten_toml(SAMPLE_XIAOHE).expect("样例是合法 TOML"),
        )
        .expect("样例能过校验");
        assert_eq!(scheme.name, "小鹤双拼");
        let table = scheme.table();
        // 小鹤的键位：i = ch、u = sh、b = in、l = iang/uang、n = iao、m = ian、k = ing/uai
        assert_eq!(table.remapping[&'n'].finals, ["iao"]);
        assert_eq!(table.remapping[&'m'].finals, ["ian"]);
        assert_eq!(table.remapping[&'k'].finals, ["ing", "uai"]);
        let pinyin = |keys: &str| -> Vec<String> {
            table
                .expand(keys)
                .iter()
                .map(cloudime_core::shuangpin::Expansion::to_pinyin)
                .collect()
        };
        assert!(
            pinyin("pb").contains(&"pin".to_owned()),
            "{:?}",
            pinyin("pb")
        );
        assert!(
            pinyin("ul").contains(&"shuang".to_owned()),
            "{:?}",
            pinyin("ul")
        );
        // 零声母：`ai` 照原样、`ah` 是 ang
        assert!(
            pinyin("ai").contains(&"ai".to_owned()),
            "{:?}",
            pinyin("ai")
        );
        assert!(
            pinyin("ah").contains(&"ang".to_owned()),
            "{:?}",
            pinyin("ah")
        );
    }

    /// 键位映射参考的版面：三排键盘格子（按 QWERTY 的顺序，每格「键 + 这个键上的声母 / 韵母」）
    /// + 零声母表（`[Shengmu]` 的每一项一格，两键 + 韵母）。
    #[test]
    fn key_map_layout_is_a_keyboard_plus_zero_initials() {
        let scheme = validate(
            "ziranma".to_owned(),
            Format::Toml,
            flatten_toml(SAMPLE_ZIRANMA).expect("样例是合法 TOML"),
        )
        .expect("样例能过校验");
        let layout = scheme.key_map_layout();

        // 三排：10 / 10 / 7，键就是键盘上那些
        let rows: Vec<String> = layout
            .rows
            .iter()
            .map(|row| row.iter().map(|cap| cap.key.as_str()).collect())
            .collect();
        assert_eq!(rows, ["QWERTYUIOP", "ASDFGHJKL;", "ZXCVBNM"]);
        // 每格：声母在右上角、韵母一义一行在中间
        let cap = |key: &str| {
            layout
                .rows
                .iter()
                .flatten()
                .find(|cap| cap.key == key)
                .expect("键盘上有这个键")
        };
        assert_eq!(cap("V").initial, "zh", "声母在右上角");
        assert_eq!(cap("V").mapping, "ui\nü", "韵母一义一行，v 写成 ü");
        assert_eq!(cap("T").mapping, "üe\nue");
        assert_eq!(cap("P").mapping, "un\nün", "同一键上 un 与 ün 各占一行");
        assert_eq!(cap("R").mapping, "uan\nüan");
        assert_eq!(cap("A").mapping, "a");
        assert_eq!(cap("A").initial, "", "这个键上没有声母");
        // 零声母表：两键 + 韵母，数量跟 [Shengmu] 一致（样例 12 项 = 6 列 × 2 行）
        assert_eq!(layout.zero_initials.len(), scheme.zero_initials.len());
        let zero = |key: &str| {
            layout
                .zero_initials
                .iter()
                .find(|cap| cap.key == key)
                .expect("零声母表里有这一格")
        };
        assert_eq!(zero("AH").mapping, "ang");
        assert_eq!(zero("AH").key, "AH");
    }

    /// INI 写同样的东西也要能读（除了 Exchange）。
    #[test]
    fn ini_and_toml_agree() {
        let ini = "\
[Info]
Name = \"小鹤\"
Version = 1.0

[Shengmu]
a = AA

[Remapping]
v = zh,v
b = ou
";
        let scheme = validate(
            "flypy".to_owned(),
            Format::Ini,
            flatten_ini(ini).expect("INI 能解析"),
        )
        .expect("INI 也能过校验");
        assert_eq!(scheme.name, "小鹤");
        assert_eq!(scheme.version, "1.0", "INI 里没引号也当字符串");
        assert_eq!(scheme.zero_initials[&('a', 'a')], "a");
        assert_eq!(scheme.remapping[&'v'].initials, ["zh"]);
    }

    /// 必填缺失 / 取值不对 / 未知节与键，都要报出来（不是只报第一个）。
    #[test]
    fn problems_are_collected() {
        let text = "\
[Info]
Version = \"1.0\"

[Shengmu]
uo = \"AA\"
a = \"AAA\"

[Remapping]
b = \"ouu\"
q = \"iu\"

[Whatever]
x = \"1\"
";
        let errors = validate(
            "bad".to_owned(),
            Format::Toml,
            flatten_toml(text).expect("TOML 本身合法"),
        )
        .expect_err("应当报错");
        let messages: Vec<String> = errors.iter().map(|error| error.to_string()).collect();
        assert!(
            messages.iter().any(|text| text.contains("Name 必填")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|text| text.contains("不是能单独成音节的韵母")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|text| text.contains("不是两个字母")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|text| text.contains("ouu")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|text| text.contains("不认识的节 [whatever]")),
            "{messages:?}"
        );
    }

    /// `[Shengmu]`：缺节、左边不是零声母韵母、右边不是两键、两组撞键都要报出来。
    #[test]
    fn shengmu_rules() {
        let good = "\
[Info]
Name = \"s\"
Version = \"1\"

[Shengmu]
ang = \"AY\"

[Remapping]
v = \"zh\"
";
        let scheme = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(good).expect("TOML 合法"),
        )
        .expect("能过校验");
        assert_eq!(scheme.zero_initials[&('a', 'y')], "ang");

        let bad = "\
[Info]
Name = \"s\"
Version = \"1\"

[Shengmu]
uo = \"AA\"
a = \"ABC\"
an = \"BB\"
ang = \"BB\"

[Remapping]
v = \"zh\"
";
        let errors = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(bad).expect("TOML 合法"),
        )
        .expect_err("应当报错");
        let messages: Vec<String> = errors.iter().map(|error| error.to_string()).collect();
        assert!(
            messages
                .iter()
                .any(|text| text.contains("不是能单独成音节的韵母")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|text| text.contains("不是两个字母")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|text| text.contains("用了同一组按键")),
            "{messages:?}"
        );

        let missing = "[Info]\nName = \"s\"\nVersion = \"1\"\n[Remapping]\nv = \"zh\"\n";
        let errors = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(missing).expect("TOML 合法"),
        )
        .expect_err("缺 [Shengmu]");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("缺 [Shengmu]")),
            "{errors:?}"
        );
    }

    /// 布尔：0/1/true/false 都认，别的报错。
    #[test]
    fn booleans_accept_both_spellings() {
        for (text, expected) in [("0", false), ("1", true), ("true", true), ("FALSE", false)] {
            let raw = flatten_ini(&format!("[Auxiliary]\nUsingAuxiliaryCodes = {text}\n")).unwrap();
            assert_eq!(
                flag(&raw, "auxiliary", "usingauxiliarycodes"),
                Some(Ok(expected)),
                "{text}"
            );
        }
        let raw = flatten_ini("[Auxiliary]\nUsingAuxiliaryCodes = yes\n").unwrap();
        assert!(
            flag(&raw, "auxiliary", "usingauxiliarycodes")
                .unwrap()
                .is_err()
        );
    }

    /// `[Info] RawInput` 已搬到配置（`[input] show_raw_input`）—— 方案文件里写了算不认识的键。
    #[test]
    fn raw_input_is_no_longer_a_scheme_key() {
        let text = "[Info]\nName = \"s\"\nVersion = \"1\"\nRawInput = false\n\n[Shengmu]\na = \"AA\"\n\n[Remapping]\nv = \"zh\"\n";
        let errors = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(text).expect("TOML 合法"),
        )
        .expect_err("RawInput 不再是方案文件里的键");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("不认识的键 rawinput")),
            "{errors:?}"
        );
    }

    /// Exchange：TOML 合法；链式、坏写法、INI 里写都报错。
    #[test]
    fn exchange_rules() {
        let good = "[Exchange]\nswap = [\"y-z\"]\n[Remapping]\nv = \"zh\"\n";
        let scheme = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(good).expect("TOML 合法"),
        );
        // 缺必填会报错，但 Exchange 本身应当被读出来
        let errors = scheme.expect_err("缺 Info 必填");
        assert!(
            !errors
                .iter()
                .any(|error| error.to_string().contains("Exchange")),
            "{errors:?}"
        );

        let chained = "[Exchange]\nswap = [\"a-b\", \"b-c\"]\n[Remapping]\nv = \"zh\"\n";
        let errors = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(chained).expect("TOML 合法"),
        )
        .expect_err("链式应当报错");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("禁止链式")),
            "{errors:?}"
        );

        let in_ini = flatten_ini("[Exchange]\nswap = y-z\n[Remapping]\nv = zh\n").unwrap();
        let errors =
            validate("s".to_owned(), Format::Ini, in_ini).expect_err("INI 里不该有 Exchange");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("只支持 TOML")),
            "{errors:?}"
        );
    }

    /// `[Auxiliary]` 只认 `UsingAuxiliaryCodes`：`Location`（辅助码位置）与 `File` 都已经没有这一项了，
    /// 写了算不认识的键 —— 辅助码逐字对应（敲在音节后面），用哪份辅助码表在 `use_input_scheme.cfg`。
    #[test]
    fn auxiliary_only_has_the_using_flag() {
        let base = "[Info]\nName = \"s\"\nVersion = \"1\"\n\n[Shengmu]\na = \"AA\"\n\n[Auxiliary]\nUsingAuxiliaryCodes = 1\n";
        let scheme = validate(
            "s".to_owned(),
            Format::Ini,
            flatten_ini(base).expect("INI 合法"),
        )
        .expect("只要开了辅助码就是合法方案，不需要别的位置");
        assert!(scheme.using_auxiliary_codes);

        for gone in ["Location = \"tail\"", "File = \"AuxiliaryCodes/zb.tsv\""] {
            let text = format!("{base}{gone}\n");
            let errors = validate(
                "s".to_owned(),
                Format::Ini,
                flatten_ini(&text).expect("INI 合法"),
            )
            .expect_err("这两项已经不在方案文件里了");
            assert!(
                errors
                    .iter()
                    .any(|error| error.to_string().contains("不认识的键")),
                "{gone}：{errors:?}"
            );
        }
    }

    /// `;` 键只写在 TOML 里（TOML 的裸键不允许 `;`，得写成 `";"`），而且用了才吃分号。
    #[test]
    fn semicolon_comes_from_the_table() {
        let text = "\
[Info]
Name = \"s\"
Version = \"1\"

[Shengmu]
a = \"AA\"

[Remapping]
v = \"zh\"
i = \"ch\"
\";\" = \"ing\"
";
        let scheme = validate(
            "s".to_owned(),
            Format::Toml,
            flatten_toml(text).expect("TOML 合法"),
        )
        .expect("能过校验");
        assert!(scheme.uses_semicolon());
        assert_eq!(scheme.remapping[&';'].finals, ["ing"]);

        // 不用 `;` 的方案不吃分号
        let plain = "[Remapping]\nv = \"zh\"\n";
        let mapping = remapping(&flatten_toml(plain).unwrap(), &mut Vec::new());
        assert!(!mapping.contains_key(&';'));
    }

    /// INI 里半角 `;` 是注释，所以中文全角 `；` 也当半角分号用 —— 用到 `;` 的方案同样能写成 INI。
    #[test]
    fn full_width_semicolon_is_the_semicolon_key() {
        let text = "\
[Info]
Name = \"微软双拼\"
Version = \"1.0\"

[Shengmu]
a = AA

[Remapping]
v = zh
；= ing
";
        let scheme = validate(
            "mspy".to_owned(),
            Format::Ini,
            flatten_ini(text).expect("INI 合法"),
        )
        .expect("能过校验");
        assert!(scheme.uses_semicolon());
        assert_eq!(scheme.remapping[&';'].finals, ["ing"]);
    }

    /// 去掉 BOM、去掉值外面的引号。
    #[test]
    fn syntax_helpers() {
        assert_eq!(strip_bom(vec![0xEF, 0xBB, 0xBF, b'a']), vec![b'a']);
        assert_eq!(strip_bom(vec![b'a']), vec![b'a']);
        assert_eq!(unquote("\"自然码\""), "自然码");
        assert_eq!(unquote("'自然码'"), "自然码");
        assert_eq!(unquote("自然码"), "自然码");
        assert_eq!(unquote("\""), "\"");
        // INI 的整行注释与空行
        let raw = flatten_ini("# 头注释\n\n[Info]\n; 也当注释\nName = 全拼\n").unwrap();
        assert_eq!(raw["info"]["name"], "全拼");
        // 重复键报错
        assert!(flatten_ini("[Info]\nName = a\nName = b\n").is_err());
        // 不在节里的键报错
        assert!(flatten_ini("Name = a\n").is_err());
    }

    /// 设置页那个开关：改写 `[Auxiliary] UsingAuxiliaryCodes`，别的行（含注释）原样不动。
    #[test]
    fn rewriting_the_auxiliary_flag_keeps_the_rest() {
        // 已经有这一项 → 换掉那一行的值
        let text = "[Info]\nName = \"s\"\n\n[Auxiliary]\nUsingAuxiliaryCodes = false\nFile = \"a.tsv\"\n\n[Remapping]\nv = \"zh\"\n";
        let out = rewrite_auxiliary_flag(text, "true");
        assert!(out.contains("UsingAuxiliaryCodes = true"), "{out}");
        assert!(out.contains("File = \"a.tsv\""), "{out}");
        assert!(out.contains("[Remapping]"), "{out}");
        assert_eq!(out.matches("UsingAuxiliaryCodes").count(), 1, "{out}");

        // 节在、这一项不在 → 插在节标题后面
        let text =
            "[Info]\nName = \"s\"\n\n[Auxiliary]\nFile = \"a.tsv\"\n\n[Remapping]\nv = \"zh\"\n";
        let out = rewrite_auxiliary_flag(text, "true");
        let lines: Vec<&str> = out.lines().collect();
        let header = lines
            .iter()
            .position(|line| *line == "[Auxiliary]")
            .unwrap();
        assert_eq!(lines[header + 1], "UsingAuxiliaryCodes = true", "{out}");
        assert_eq!(lines[header + 2], "File = \"a.tsv\"", "{out}");

        // 连节都没有 → 末尾补一节
        let text = "[Info]\nName = \"s\"\n\n[Remapping]\nv = \"zh\"\n";
        let out = rewrite_auxiliary_flag(text, "false");
        assert!(
            out.ends_with("[Auxiliary]\nUsingAuxiliaryCodes = false\n"),
            "{out:?}"
        );

        // 节名 / 键名大小写不敏感，注释行不动
        let text = "[info]\nName = s\n\n[auxiliary]\n; 说明\nUSINGAUXILIARYCODES = 0\n\n[remapping]\nv = zh\n";
        let out = rewrite_auxiliary_flag(text, "1");
        assert!(out.contains("; 说明"), "{out}");
        assert!(out.contains("UsingAuxiliaryCodes = 1"), "{out}");
        assert!(!out.contains("USINGAUXILIARYCODES"), "{out}");

        // 原来是 `\r\n` 就照 `\r\n`
        let out = rewrite_auxiliary_flag("[Auxiliary]\r\nUsingAuxiliaryCodes = 0\r\n", "1");
        assert!(out.contains("UsingAuxiliaryCodes = 1\r\n"), "{out:?}");
    }

    /// 改完之后方案还能读：开的那个真开、关的那个真关。
    #[test]
    fn rewriting_the_flag_round_trips_through_parse() {
        let dir = std::env::temp_dir().join(format!("cloudime-scheme-flag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.toml");
        let base = "[Info]\nName = \"s\"\nVersion = \"1\"\n\n[Shengmu]\na = \"AA\"\n\n[Auxiliary]\n\n[Remapping]\nv = \"zh,un,vn,uan,van,ue,ve\"\n";
        std::fs::write(&path, base).unwrap();
        assert!(
            !load(&path).unwrap().using_auxiliary_codes,
            "没写 UsingAuxiliaryCodes = 关"
        );

        set_using_auxiliary_codes(&path, true).unwrap();
        assert!(
            load(&path).unwrap().using_auxiliary_codes,
            "开关写进去就读得出来"
        );

        set_using_auxiliary_codes(&path, false).unwrap();
        assert!(!load(&path).unwrap().using_auxiliary_codes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
