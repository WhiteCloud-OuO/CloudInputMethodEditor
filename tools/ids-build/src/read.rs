//! 解析 BabelStone `IDS.TXT`。

use std::collections::{BTreeMap, BTreeSet};

use cloudime_ids::{
    FLAG_UNENCODED, FLAG_VARIATION, FLAG_WILDCARD, is_variation, normalize, parse_sources,
};

use crate::BuildError;

/// 一个键下的一个字。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    pub sources: u16,
    pub flags: u16,
}

/// 解析结果：按规范化 IDS 聚合，值里按码位排字。
#[derive(Debug, Default)]
pub struct Rows {
    /// 文件里出现过的所有字（含只有未编码部件的）。
    pub characters: BTreeSet<char>,

    /// 规范化 IDS → 字 → 来源 / 标记。
    pub keys: BTreeMap<String, BTreeMap<char, Entry>>,
}

/// 解析整个文件。`line` 从 1 起，报错带行号。
pub fn parse(source: &str) -> Result<Rows, BuildError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut rows = Rows::default();
    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let codepoint = fields
            .next()
            .ok_or_else(|| bad_line(line_number, "缺码位列"))?;
        let character = fields
            .next()
            .ok_or_else(|| bad_line(line_number, "缺字符列"))?;
        let character = single_character(character)
            .ok_or_else(|| bad_line(line_number, "第二列不是单个字符"))?;
        let expected =
            parse_codepoint(codepoint).ok_or_else(|| bad_line(line_number, "码位列不是 U+XXXX"))?;
        if u32::from(character) != expected {
            return Err(bad_line(
                line_number,
                &format!("码位 {codepoint} 与字符 {character} 不一致"),
            ));
        }
        rows.characters.insert(character);
        for sequence in fields {
            let sequence = sequence.trim();
            // 第三列起也可能出现注释（`*U+4E8C≠U+2011E…`），不是 IDS 就跳过。
            if !sequence.starts_with('^') {
                continue;
            }
            insert_sequence(&mut rows, line_number, character, sequence)?;
        }
    }
    Ok(rows)
}

impl Rows {
    /// 每个字取一个 IDS（字节序最小的那个），笔画递归与派生候选用。
    pub fn first_ids_by_character(&self) -> BTreeMap<char, &str> {
        let mut map = BTreeMap::new();
        for (ids, characters) in &self.keys {
            for character in characters.keys() {
                map.entry(*character).or_insert(ids.as_str());
            }
        }
        map
    }
}

fn insert_sequence(
    rows: &mut Rows,
    line: usize,
    character: char,
    sequence: &str,
) -> Result<(), BuildError> {
    let body = sequence
        .strip_prefix('^')
        .ok_or_else(|| bad_line(line, "IDS 列不以 ^ 开头"))?;
    let (ids, sources) = body
        .split_once('$')
        .ok_or_else(|| bad_line(line, "IDS 列没有 $ 结束符"))?;
    let sources = sources
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| bad_line(line, "IDS 列末尾不是 (来源)"))?;

    let bits = parse_sources(sources)
        .map_err(|letter| bad_line(line, &format!("未知来源字母：{letter}")))?;

    let variation = is_variation(ids);
    let normalized = normalize(ids).map_err(|source| BuildError::Parse { line, source })?;
    let mut flags = 0u16;
    if variation {
        flags |= FLAG_VARIATION;
    }
    if normalized.contains('{') {
        flags |= FLAG_UNENCODED;
    }
    if normalized.contains('？') {
        flags |= FLAG_WILDCARD;
    }

    let entry = rows
        .keys
        .entry(normalized)
        .or_default()
        .entry(character)
        .or_default();
    entry.sources |= bits;
    entry.flags |= flags;
    Ok(())
}

fn single_character(text: &str) -> Option<char> {
    let mut characters = text.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

fn parse_codepoint(text: &str) -> Option<u32> {
    let hex = text.strip_prefix("U+")?;
    u32::from_str_radix(hex, 16).ok()
}

fn bad_line(line: usize, reason: &str) -> BuildError {
    BuildError::BadLine {
        line,
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use cloudime_ids::source_bit;

    use super::*;

    const SAMPLE: &str = "# 注释\n\
        \n\
        U+6797\t林\t^⿰木木$(GHTJKPV)\n\
        U+6D77\t海\t^⿰氵每$(GHTKPV)\t^⿰氵毎$(J)\n\
        U+4E03\t七\t^〾⿻一乚$(GHJKPV)\n\
        U+4E0E\t与\t^⿹{22}一$(GHTKV)\n";

    #[test]
    fn groups_multiple_sources_and_flags() {
        let rows = parse(SAMPLE).unwrap();
        assert_eq!(rows.characters.len(), 4);
        assert_eq!(rows.keys.len(), 5);

        let wood = &rows.keys["⿰木木"][&'林'];
        assert_eq!(
            wood.sources & source_bit('G').unwrap(),
            source_bit('G').unwrap()
        );
        assert_eq!(wood.flags, 0);

        // 同一个字的两种来源写法落进两个键。
        assert_eq!(rows.keys["⿰氵每"][&'海'].sources, {
            let mut bits = 0;
            for letter in "GHTKPV".chars() {
                bits |= source_bit(letter).unwrap();
            }
            bits
        });
        assert_eq!(rows.keys["⿰氵毎"][&'海'].sources, source_bit('J').unwrap());

        assert_eq!(rows.keys["⿻一乚"][&'七'].flags, FLAG_VARIATION);
        assert_eq!(rows.keys["⿹{22}一"][&'与'].flags, FLAG_UNENCODED);
    }

    #[test]
    fn rejects_bad_lines_with_line_numbers() {
        let mismatch = "U+6797\t明\t^⿰日月$(G)\n";
        assert!(matches!(
            parse(mismatch),
            Err(BuildError::BadLine { line: 1, .. })
        ));

        let bad_source = "U+6797\t林\t^⿰木木$(Q)\n";
        assert!(matches!(
            parse(bad_source),
            Err(BuildError::BadLine { line: 1, .. })
        ));

        let bad_ids = "U+6797\t林\t^⿰木$(G)\n";
        assert!(matches!(
            parse(bad_ids),
            Err(BuildError::Parse { line: 1, .. })
        ));
    }

    #[test]
    fn accepts_bom_and_crlf() {
        let text = "\u{feff}U+6797\t林\t^⿰木木$(G)\r\n";
        let rows = parse(text).unwrap();
        assert_eq!(rows.keys.len(), 1);
    }
}
