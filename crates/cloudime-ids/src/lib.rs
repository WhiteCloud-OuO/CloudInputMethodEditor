//! 汉字 IDS 组字索引：把 `⿰木木` 这样的结构表达式换成 Unicode 字。
//!
//! 数据文件用 [`cloudime_format`] 的 `.qj` 容器（种类 [`Kind::Ids`](cloudime_format::Kind::Ids)），
//! 打开就是 mmap：
//! - `TEXT`：规范化 IDS 首尾相接；
//! - `KEYS`：定长 [`IdsKey`]，按 IDS 字节序排序；
//! - `HASH`：开放寻址表，槽里是 `KEYS` 下标；
//! - `ENTR`：定长 [`IdsEntry`]，一个 IDS 对应的字 / 来源 / 标记。
//!
//! 查询严格按 IDS 匹配，不做「水 → 氵」这类部件替换。

mod component;
mod database;
mod error;
mod parser;

pub use component::{
    COMPONENT_FLAG_DERIVED, COMPONENT_FLAG_STROKE_ESTIMATED, COMPONENT_FLAG_TYPEABLE,
    COMPONENT_TAG, Component, ComponentRole, MAX_STROKE,
};
pub use database::{
    ENTR_TAG, HASH_TAG, IdsDatabase, IdsEntry, IdsKey, IdsMatch, KEYS_TAG, TEXT_TAG,
    WildcardMatches,
};
pub use error::{IdsError, ParseError};
pub use parser::{IdsNode, IdsOp, is_variation, normalize, parse};

/// BabelStone 的来源字母，按文件里的顺序排（G/H/M/T/J/K/P/V/U/S/B/X/Z）。
pub const SOURCE_LETTERS: [char; 13] = [
    'G', 'H', 'M', 'T', 'J', 'K', 'P', 'V', 'U', 'S', 'B', 'X', 'Z',
];

/// 原 IDS 带 `〾`：部件与实际字形有细微差异。
pub const FLAG_VARIATION: u16 = 1;

/// 原 IDS 含 `{n}` 未编码部件。
pub const FLAG_UNENCODED: u16 = 1 << 1;

/// 原 IDS 含 `？` 不可表示部件。
pub const FLAG_WILDCARD: u16 = 1 << 2;

/// 来源字母对应的位。
pub fn source_bit(letter: char) -> Option<u16> {
    SOURCE_LETTERS
        .iter()
        .position(|item| *item == letter)
        .map(|index| 1 << index)
}

/// `UCS2003`：Unicode 13.0 码表里 Ext. B 的旧字形，不在上面的区域字母里，单独一位。
pub const SOURCE_UCS2003: u16 = 1 << SOURCE_LETTERS.len();

/// 解析 `GHTJKPV` / `G[B]` / `GJ[K][U]` / `UCS2003` 这种来源写法。
///
/// 方括号里是「虚拟字形」的来源，与括号外一样计入位图；遇到不认识的字符返回它。
pub fn parse_sources(text: &str) -> Result<u16, char> {
    const UCS2003: [char; 7] = ['U', 'C', 'S', '2', '0', '0', '3'];
    let characters: Vec<char> = text.chars().collect();
    let mut bits = 0u16;
    let mut index = 0;
    while index < characters.len() {
        if characters[index..].starts_with(&UCS2003) {
            bits |= SOURCE_UCS2003;
            index += UCS2003.len();
            continue;
        }
        let current = characters[index];
        if current == '[' || current == ']' {
            index += 1;
            continue;
        }
        let Some(bit) = source_bit(current) else {
            return Err(current);
        };
        bits |= bit;
        index += 1;
    }
    Ok(bits)
}

/// 把来源位图写成 `G/T/J` 这样的标签；没有来源给空串。
pub fn source_label(sources: u16) -> String {
    let mut label = String::new();
    for (bit, letter) in SOURCE_LETTERS.iter().enumerate() {
        if sources & (1 << bit) == 0 {
            continue;
        }
        if !label.is_empty() {
            label.push('/');
        }
        label.push(*letter);
    }
    if sources & SOURCE_UCS2003 != 0 {
        if !label.is_empty() {
            label.push('/');
        }
        label.push_str("UCS2003");
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bracketed_sources() {
        let expected = source_bit('G').unwrap() | source_bit('B').unwrap();
        assert_eq!(parse_sources("G[B]"), Ok(expected));
        assert_eq!(parse_sources("UCS2003"), Ok(SOURCE_UCS2003));
        assert_eq!(
            parse_sources("GJ[K][U]"),
            Ok(source_bit('G').unwrap()
                | source_bit('J').unwrap()
                | source_bit('K').unwrap()
                | source_bit('U').unwrap())
        );
        assert_eq!(parse_sources("Q"), Err('Q'));
        assert_eq!(source_label(expected), "G/B");
        assert_eq!(source_label(SOURCE_UCS2003), "UCS2003");
    }
}
