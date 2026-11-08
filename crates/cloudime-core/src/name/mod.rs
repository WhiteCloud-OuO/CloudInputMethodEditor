//! V 模式「取名」用的姓氏 / 名字表。
//!
//! 随安装目录 `data\` 提供两份 TSV：`姓氏大全.tsv`（汉字、拼音、排名）与 `名字大全.tsv`（汉字、拼音）。
//! 平台壳把文件读成文本后交给 [`NameTables::from_tsv`]；Core 不读文件。
//!
//! 拼音统一成小写、去掉空格与撇号（复姓 `shang guan` 存成 `shangguan`），查询一律按这个形式。

use std::collections::HashSet;

/// 取名模式的固定前缀。
pub const NAME_PREFIX: &str = "v#";

/// 姓氏表里的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurnameEntry {
    /// 姓（`王`、`欧阳`）。
    pub text: String,

    /// 去掉空格 / 撇号的小写拼音（`wang`、`ouyang`）。
    pub pinyin: String,

    /// 排名，越小越靠前。
    pub rank: u32,
}

/// 名字表里的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEntry {
    /// 名字用字（`伟`、`娜`）。
    pub text: String,

    /// 小写拼音（`wei`、`na`）。
    pub pinyin: String,
}

/// 两份表合起来。姓氏表是主表，名字表空着时只在名字上退回词库。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameTables {
    surnames: Vec<SurnameEntry>,
    names: Vec<NameEntry>,
}

impl NameTables {
    /// 解析两份 TSV。空行与 `#` 开头的行跳过；缺列、没拼音的行跳过；同一汉字只留第一次出现的那条。
    pub fn from_tsv(surnames: &str, names: &str) -> Result<Self, NameTableError> {
        let surnames = parse_surnames(surnames)?;
        if surnames.is_empty() {
            return Err(NameTableError::Empty);
        }
        Ok(Self {
            surnames,
            names: parse_names(names),
        })
    }

    pub fn surnames(&self) -> &[SurnameEntry] {
        &self.surnames
    }

    pub fn names(&self) -> &[NameEntry] {
        &self.names
    }

    pub fn surname_count(&self) -> usize {
        self.surnames.len()
    }

    pub fn name_count(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surnames.is_empty()
    }

    /// 拼音以 `prefix` 开头的姓氏（还没打完的姓氏，`ouy` → `欧阳`）。按排名排。
    pub fn surnames_with_prefix<'a>(&'a self, prefix: &str) -> Vec<&'a SurnameEntry> {
        let mut hits: Vec<_> = self
            .surnames
            .iter()
            .filter(|entry| entry.pinyin.starts_with(prefix))
            .collect();
        hits.sort_by_key(|entry| entry.rank);
        hits
    }

    /// 拼音正好是 `body` 前缀的姓氏（打完了的姓氏，`wangwei` → `王` + `wei`）。
    /// 复姓在前：`ouyangna` 先给 `欧阳`，再给 `欧`。
    pub fn surnames_leading<'a>(&'a self, body: &str) -> Vec<&'a SurnameEntry> {
        let mut hits: Vec<_> = self
            .surnames
            .iter()
            .filter(|entry| body.starts_with(&entry.pinyin))
            .collect();
        hits.sort_by(|left, right| {
            right
                .pinyin
                .len()
                .cmp(&left.pinyin.len())
                .then_with(|| left.rank.cmp(&right.rank))
        });
        hits
    }

    /// 名字字的拼音匹配：`partial` 为真时按前缀（还没打完的那个音节），否则要整音节相同。
    /// 保持表内顺序。
    pub fn name_chars<'a>(&'a self, pinyin: &str, partial: bool) -> Vec<&'a NameEntry> {
        self.names
            .iter()
            .filter(|entry| {
                if partial {
                    entry.pinyin.starts_with(pinyin)
                } else {
                    entry.pinyin == pinyin
                }
            })
            .collect()
    }

    /// 表内前 `limit` 个字（按表内顺序）。
    pub fn top_name_chars(&self, limit: usize) -> Vec<&NameEntry> {
        self.names.iter().take(limit).collect()
    }
}

fn parse_surnames(text: &str) -> Result<Vec<SurnameEntry>, NameTableError> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').map(str::trim).collect();
        let Some(text) = columns.first().copied().filter(|text| !text.is_empty()) else {
            continue;
        };
        let Some(pinyin) = columns.get(1).map(|pinyin| normalize_pinyin(pinyin)) else {
            continue;
        };
        if pinyin.is_empty() {
            continue;
        }
        if !seen.insert(text.to_owned()) {
            continue;
        }
        let rank = columns
            .get(2)
            .and_then(|rank| rank.parse().ok())
            .unwrap_or((index + 1) as u32);
        entries.push(SurnameEntry {
            text: text.to_owned(),
            pinyin,
            rank,
        });
    }
    Ok(entries)
}

fn parse_names(text: &str) -> Vec<NameEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').map(str::trim).collect();
        let Some(text) = columns.first().copied().filter(|text| !text.is_empty()) else {
            continue;
        };
        let pinyin = columns
            .get(1)
            .map(|pinyin| normalize_pinyin(pinyin))
            .unwrap_or_default();
        if pinyin.is_empty() || !seen.insert(text.to_owned()) {
            continue;
        }
        entries.push(NameEntry {
            text: text.to_owned(),
            pinyin,
        });
    }
    entries
}

/// 拼音统一成小写、去掉空格 / 撇号 / 其他非 ASCII 字母。
fn normalize_pinyin(pinyin: &str) -> String {
    pinyin
        .chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// 两份表都解析不出姓氏时，`v#` 取名不可用。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameTableError {
    /// 姓氏表里一条有效记录都没有。
    #[error("姓氏表没有可用的记录")]
    Empty,
}

impl std::fmt::Display for NameTables {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "姓氏 {} 条 / 名字 {} 条",
            self.surnames.len(),
            self.names.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURNAMES: &str = "王\twang\t1\n汪\twang\t50\n欧\tou\t60\n欧阳\tou yang\t401\n";
    const NAMES: &str = "伟\twei\n卫\twei\n建\tjian\n国\tguo\n国\tguo\n";

    #[test]
    fn parses_and_normalizes() {
        let tables = NameTables::from_tsv(SURNAMES, NAMES).unwrap();
        assert_eq!(tables.surname_count(), 4);
        assert_eq!(tables.name_count(), 4);
        let ouyang = tables.surnames().iter().find(|s| s.text == "欧阳").unwrap();
        assert_eq!(ouyang.pinyin, "ouyang");
        assert_eq!(ouyang.rank, 401);
    }

    #[test]
    fn surnames_prefer_longer_pinyin_then_rank() {
        let tables = NameTables::from_tsv(SURNAMES, NAMES).unwrap();
        let leading: Vec<&str> = tables
            .surnames_leading("ouyangna")
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(leading, ["欧阳", "欧"]);

        let wang: Vec<&str> = tables
            .surnames_with_prefix("wang")
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(wang, ["王", "汪"]);
    }

    #[test]
    fn name_chars_match_exact_and_prefix_in_table_order() {
        let tables = NameTables::from_tsv(SURNAMES, NAMES).unwrap();
        let exact: Vec<&str> = tables
            .name_chars("wei", false)
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(exact, ["伟", "卫"]);
        let partial: Vec<&str> = tables
            .name_chars("w", true)
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(partial, ["伟", "卫"]);
    }

    #[test]
    fn empty_surname_table_is_rejected() {
        assert_eq!(NameTables::from_tsv("", NAMES), Err(NameTableError::Empty));
    }
}
