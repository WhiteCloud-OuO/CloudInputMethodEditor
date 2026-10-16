//! 辅助码文件：两列 `词 \t 辅助码` 的 TSV。
//!
//! 规范见 `docs/design/input-schemes.md` §4：
//! - 只认 `词 \t 辅助码` 两列；**大小写不敏感**（文件里大写小写都行，组句时**必须大写**才算辅助码）。
//! - **一个词多个码就写多行** —— 任一个都能选中它。
//! - 辅助码**最长 4 位**；超了 / 格式不对的行跳过并记一条日志（数据文件按「坏行跳过」处理）。
//! - 其它格式（`txt` / `dat` / `csv` / `yaml`）暂不支持，见设计文档 §9。

use std::collections::BTreeMap;
use std::path::Path;

use cloudime_core::auxiliary::AuxiliaryMode;
use cloudime_core::shuangpin::MAX_AUXILIARY;

/// 「辅助码怎么对」这四种方式（设置页下拉的顺序就是它）。
pub const MODES: [AuxiliaryMode; 4] = [
    AuxiliaryMode::PerChar,
    AuxiliaryMode::DoubleHead,
    AuxiliaryMode::DoubleTail,
    AuxiliaryMode::FirstLast,
];

/// `use_input_scheme.cfg` 里 `auxiliary_mode` 的写法。
pub fn mode_key(mode: AuxiliaryMode) -> &'static str {
    match mode {
        AuxiliaryMode::PerChar => "per_char",
        AuxiliaryMode::DoubleHead => "double_head",
        AuxiliaryMode::DoubleTail => "double_tail",
        AuxiliaryMode::FirstLast => "first_last",
    }
}

/// 从配置里读；认不出来就是缺省的逐字（不因为一行写错就让整份配置失效）。
pub fn mode_of(value: &str) -> AuxiliaryMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "double_head" => AuxiliaryMode::DoubleHead,
        "double_tail" => AuxiliaryMode::DoubleTail,
        "first_last" => AuxiliaryMode::FirstLast,
        _ => AuxiliaryMode::PerChar,
    }
}

/// 界面上的名字。
pub fn mode_label(mode: AuxiliaryMode) -> &'static str {
    match mode {
        AuxiliaryMode::PerChar => "逐字辅助码",
        AuxiliaryMode::DoubleHead => "半字辅助码（双前）",
        AuxiliaryMode::DoubleTail => "半字辅助码（双后）",
        AuxiliaryMode::FirstLast => "半字辅助码（首尾）",
    }
}

/// 一张辅助码表：`词 → 若干个码`（都按大写存）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuxiliaryCodes {
    codes: BTreeMap<String, Vec<String>>,
}

impl AuxiliaryCodes {
    /// 读一份辅助码文件；文件不在 / 读不动 / 不是 UTF-8 都算读不出来（调用方按「没开辅助码」退）。
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        Some(Self::parse(&text))
    }

    /// 解析（坏行跳过）。
    pub fn parse(text: &str) -> Self {
        let mut codes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut skipped = 0usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((word, code)) = line.split_once('\t') else {
                skipped += 1;
                continue;
            };
            let word = word.trim();
            let code = code.trim().to_uppercase();
            let usable = !word.is_empty()
                && !code.is_empty()
                && code.chars().count() <= MAX_AUXILIARY
                && code.chars().all(|letter| letter.is_ascii_uppercase());
            if !usable {
                skipped += 1;
                continue;
            }
            let entry = codes.entry(word.to_owned()).or_default();
            if !entry.contains(&code) {
                entry.push(code);
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "辅助码文件里有跳过（格式不对 / 码太长）的行");
        }
        Self { codes }
    }

    /// 这个词的码（都按大写）；没有这个词就是 `None`。
    pub fn codes(&self, word: &str) -> Option<&[String]> {
        self.codes.get(word).map(Vec::as_slice)
    }

    /// 这个词有没有这个码（大小写不敏感；一个词多个码时任一个相同就算有）。
    pub fn matches(&self, word: &str, code: &str) -> bool {
        let code = code.to_uppercase();
        self.codes(word).is_some_and(|codes| codes.contains(&code))
    }

    /// 表里有几个词。
    pub fn len(&self) -> usize {
        self.codes.len()
    }

    /// 表是空的吗。
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
}

impl cloudime_core::shuangpin::AuxiliaryTable for AuxiliaryCodes {
    fn codes(&self, word: &str) -> Option<&[String]> {
        self.codes.get(word).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_columns_and_ignores_case() {
        let codes = AuxiliaryCodes::parse("词典\tcd\n词法\tcF\n\n# 注释\n");
        assert_eq!(codes.len(), 2);
        assert_eq!(
            codes.codes("词典").map(|codes| codes.join(",")),
            Some("CD".to_owned()),
            "文件里小写也收，存大写"
        );
        assert!(codes.matches("词典", "cd"), "查的时候大小写不敏感");
        assert!(codes.matches("词法", "CF"));
        assert!(!codes.matches("词法", "cd"));
        assert!(codes.codes("没有这个词").is_none());
    }

    /// 一个词多个码：多行；任一个都算命中。
    #[test]
    fn several_codes_for_one_word() {
        let codes = AuxiliaryCodes::parse("词典\tCD\n词典\tDC\n");
        assert_eq!(codes.codes("词典").unwrap().len(), 2);
        assert!(codes.matches("词典", "dc"));
        assert!(codes.matches("词典", "CD"));
        assert!(!codes.matches("词典", "CC"));
    }

    /// 坏行跳过：少一列、空码、超过 4 位、带非字母。
    #[test]
    fn broken_lines_are_skipped() {
        let codes = AuxiliaryCodes::parse("只有一列\n词典\t\n词典\tABCDE\n词典\tC1\n好的\tAB\n");
        assert_eq!(codes.len(), 1, "只留 `好的`");
        assert_eq!(
            codes.codes("好的").map(|codes| codes.join(",")),
            Some("AB".to_owned())
        );
    }

    /// 同一行重复的码只留一份。
    #[test]
    fn duplicates_collapse() {
        let codes = AuxiliaryCodes::parse("词\tAB\n词\tab\n");
        assert_eq!(codes.codes("词").unwrap().len(), 1);
    }

    /// 读不到文件时 `None`（调用方按没开辅助码退）。
    #[test]
    fn missing_file_is_none() {
        assert!(AuxiliaryCodes::load(Path::new("这个文件不存在.tsv")).is_none());
    }
}
