//! 读源文件、按格式拆行，得到去重与过滤后的词目。
//!
//! 每行 `词<TAB>拼音或编码<TAB>权重`：
//! - 中文（词里含汉字）：`text` = 第 1 列，`pinyin` = 第 2 列（`'` → 空格、压掉多余空白、转小写）。
//! - 英文（否则）：`text` = 第 2 列的小写（查词编码），`pinyin` = 第 1 列（原样写法）。
//! - 权重：第 3 列；没有或不是数字时，若只有两列且第 2 列本身是数字就当权重，
//!   否则用缺省权重 [`DEFAULT_WEIGHT`]（100，普通组的下限，不会被打成生僻）。

use std::collections::HashMap;
use std::path::Path;

use clap::ValueEnum;

use crate::cli::Args;
use crate::error::Error;

/// 权重列没给 / 读不出来时的权重。
const DEFAULT_WEIGHT: u32 = 100;

/// 一条词目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// 词（中文）或小写编码（英文）。
    pub(crate) text: String,

    /// 拼音（中文，音节空格分隔）或原样写法（英文）。
    pub(crate) pinyin: String,

    /// 权重。
    pub(crate) weight: u32,

    /// 语言。
    pub(crate) language: Language,
}

/// 词目按哪种语言存（也是 `--exclude_lang` 的取值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Language {
    /// 词里含汉字。
    Chinese,

    /// 其余（ASCII 字母、数字等）。
    English,
}

impl Language {
    /// 数据库 `language` 列里的值，与 `cloudime-dictionary` 一致。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "中文",
            Self::English => "英文",
        }
    }
}

/// 读源文件，解析 + 去重 + 过滤。
pub(crate) fn load(args: &Args) -> Result<Vec<Entry>, Error> {
    let raw = std::fs::read_to_string(&args.input).map_err(|source| Error::Read {
        path: args.input.clone(),
        source,
    })?;
    // UTF-8 BOM 会贴在第一个字段前面，先剥掉。
    let text = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let lines = data_lines(text, is_yaml(&args.input));
    let entries = retain(parse_lines(&lines, args.ignore_head), args);
    if entries.is_empty() {
        return Err(Error::Empty(args.input.clone()));
    }
    Ok(entries)
}

/// 按扩展名认格式：`.yaml` / `.yml` 当 Rime 词典。
fn is_yaml(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
    })
}

/// 取数据行：yaml 先跳过 `---` 开头的 YAML 头（跳到第一行带制表符的），其余格式从头开始。
fn data_lines(text: &str, yaml: bool) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    if !yaml {
        return lines;
    }
    match lines.iter().position(|line| line.trim() == "---") {
        Some(start) => lines[start..]
            .iter()
            .skip_while(|line| !line.contains('\t'))
            .copied()
            .collect(),
        None => lines,
    }
}

/// 逐行解析，按 `(词, 拼音)` 去重（权重大的胜出，与数据库的 upsert 一致）。
fn parse_lines(lines: &[&str], ignore_head: bool) -> Vec<Entry> {
    let mut entries: HashMap<(String, String), Entry> = HashMap::new();
    let mut skipped = 0usize;
    let mut head_done = !ignore_head;
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        if !head_done {
            // 开头的非格式文本一律丢掉，直到第一行像数据的（至少两列）。
            if fields.len() < 2 {
                continue;
            }
            head_done = true;
        }
        let Some(entry) = parse_entry(&fields) else {
            skipped += 1;
            continue;
        };
        let key = (entry.text.clone(), entry.pinyin.clone());
        let heavier = entries
            .get(&key)
            .is_some_and(|existing| existing.weight >= entry.weight);
        if !heavier {
            entries.insert(key, entry);
        }
    }
    if skipped > 0 {
        tracing::warn!(skipped, "有行认不出来，已跳过（列之间要用制表符）");
    }
    let mut entries: Vec<Entry> = entries.into_values().collect();
    entries.sort_by(|a, b| a.text.cmp(&b.text).then_with(|| a.pinyin.cmp(&b.pinyin)));
    entries
}

/// 一行字段 → 一条词目；认不出返回 `None`。
fn parse_entry(fields: &[&str]) -> Option<Entry> {
    let word = *fields.first()?;
    if word.is_empty() || fields.len() < 2 {
        return None;
    }
    let second = fields[1];
    // 只有两列且第 2 列是数字：第 2 列是权重，没有单独的拼音 / 编码列（英文词表常见 `词<TAB>词频`）。
    let (reading, weight) = if fields.len() == 2 && second.parse::<u32>().is_ok() {
        (None, second.parse().unwrap_or(DEFAULT_WEIGHT))
    } else {
        (
            Some(second),
            fields
                .get(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_WEIGHT),
        )
    };
    let (text, pinyin, language) = if is_chinese(word) {
        (
            word.to_owned(),
            normalize_pinyin(reading?),
            Language::Chinese,
        )
    } else {
        // 英文：第 2 列是查词编码（小写），第 1 列是原样写法。
        (
            reading.unwrap_or(word).to_ascii_lowercase(),
            word.to_owned(),
            Language::English,
        )
    };
    if text.is_empty() || pinyin.is_empty() {
        return None;
    }
    Some(Entry {
        text,
        pinyin,
        weight,
        language,
    })
}

/// 词里含汉字就算中文。
fn is_chinese(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{3400}'..='\u{4DBF}'       // 扩展 A
                | '\u{4E00}'..='\u{9FFF}' // 基本区
                | '\u{F900}'..='\u{FAFF}' // 兼容表意文字
                | '\u{20000}'..='\u{2FA1F}' // 扩展 B 及以后
        )
    })
}

/// 中文拼音：`'` 当音节分隔符换成空格，压掉多余空白，转小写。
fn normalize_pinyin(reading: &str) -> String {
    reading
        .replace('\'', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 按 `--exclude_below`（严格小于）与 `--exclude_lang` 过滤。
fn retain(entries: Vec<Entry>, args: &Args) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|entry| args.exclude_below.is_none_or(|below| entry.weight >= below))
        .filter(|entry| args.exclude_lang != Some(entry.language))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn yaml_header_is_skipped() {
        let text =
            "# Rime\n---\nname: test\nversion: \"1\"\nsort: by_weight\n...\n开发\tkai fa\t9000\n";
        let data = data_lines(text, true);
        assert_eq!(data, ["开发\tkai fa\t9000"]);
    }

    #[test]
    fn chinese_keeps_the_word_and_normalizes_the_pinyin() {
        let entry = parse_entry(&["开发", "KAI'FA", "9000"]).unwrap();
        assert_eq!(entry.text, "开发");
        assert_eq!(entry.pinyin, "kai fa");
        assert_eq!(entry.weight, 9000);
        assert_eq!(entry.language, Language::Chinese);
    }

    #[test]
    fn english_uses_the_code_as_text_and_keeps_the_spelling() {
        let entry = parse_entry(&["GitHub", "github", "700"]).unwrap();
        assert_eq!(entry.text, "github");
        assert_eq!(entry.pinyin, "GitHub");
        assert_eq!(entry.language, Language::English);
        // 两列且第 2 列是数字：第 2 列当权重
        let two = parse_entry(&["GitHub", "700"]).unwrap();
        assert_eq!(two.text, "github");
        assert_eq!(two.weight, 700);
    }

    #[test]
    fn missing_weight_falls_back_to_the_default() {
        let entry = parse_entry(&["开发", "kai fa"]).unwrap();
        assert_eq!(entry.weight, DEFAULT_WEIGHT);
    }

    #[test]
    fn ignore_head_drops_leading_junk() {
        let data = lines("前言第一行\n第二行\n开发\tkai fa\t9000\n");
        assert_eq!(parse_lines(&data, true).len(), 1);
        // 不加 --ignore_head：前面的行没有制表符，认不出来被跳过；带制表符的行照收
        let noisy = lines("a\tb\n开发\tkai fa\t9000\n");
        assert_eq!(parse_lines(&noisy, false).len(), 2);
    }

    #[test]
    fn deduplicates_keeping_the_heavier_weight() {
        let data = lines("开发\tkai fa\t1\n开发\tkai fa\t9000\n");
        let entries = parse_lines(&data, false);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].weight, 9000);
    }

    #[test]
    fn filters_by_weight_and_language() {
        let args = |exclude_below, exclude_lang| Args {
            input: std::path::PathBuf::from("in.tsv"),
            output: std::path::PathBuf::from("out.db"),
            ignore_head: false,
            exclude_below,
            exclude_lang,
            remove_source: 0,
        };
        let entries = [
            Entry {
                text: "开发".to_owned(),
                pinyin: "kai fa".to_owned(),
                weight: 9000,
                language: Language::Chinese,
            },
            Entry {
                text: "龘".to_owned(),
                pinyin: "da".to_owned(),
                weight: 1,
                language: Language::Chinese,
            },
            Entry {
                text: "github".to_owned(),
                pinyin: "github".to_owned(),
                weight: 100,
                language: Language::English,
            },
        ];
        // 严格小于：权重 1 被删，权重 100 留下
        let kept = retain(entries.to_vec(), &args(Some(100), None));
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|entry| entry.weight >= 100));
        // 排除英文只剩中文两条
        let kept = retain(entries.to_vec(), &args(None, Some(Language::English)));
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|entry| entry.language == Language::Chinese));
    }
}
