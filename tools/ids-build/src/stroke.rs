//! 笔画数：`components.tsv` > Unihan `kTotalStrokes` > IDS 递归 > 未知。
//!
//! 已收录的部件在 TSV 里都带笔画，Unihan 只在 `--derive` 与新部件补全时用；
//! 两样都没有时按 IDS 树把叶子笔画加起来，算不出来就返回 `None`。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use cloudime_ids::{IdsNode, MAX_STROKE, parse};

use crate::BuildError;

/// Unihan 里 `kTotalStrokes` 的子集。
#[derive(Debug, Default)]
pub struct Unihan {
    strokes: HashMap<char, u8>,
}

impl Unihan {
    /// 从 `Unihan_IRGSources.txt` 读；不存在的字段跳过。
    pub fn load(path: &Path) -> Result<Self, BuildError> {
        let text = std::fs::read_to_string(path).map_err(|source| BuildError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self::parse(&text))
    }

    /// 解析 Unihan 行：`U+4E00\tkTotalStrokes\t1`；多区域值取第一个。
    pub fn parse(text: &str) -> Self {
        let mut strokes = HashMap::new();
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let mut fields = line.split('\t');
            let Some(codepoint) = fields.next() else {
                continue;
            };
            if fields.next() != Some("kTotalStrokes") {
                continue;
            }
            let Some(value) = fields.next() else {
                continue;
            };
            let Some(character) = codepoint
                .strip_prefix("U+")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .and_then(char::from_u32)
            else {
                continue;
            };
            let Some(strokes_value) = value
                .split_whitespace()
                .find_map(|item| item.parse::<u8>().ok())
            else {
                continue;
            };
            strokes.insert(character, strokes_value);
        }
        Self { strokes }
    }

    /// 这个字的笔画数（Unihan 原值，可能大于 13）。
    pub fn stroke(&self, character: char) -> Option<u8> {
        self.strokes.get(&character).copied()
    }
}

/// 用 IDS 树递归数笔画；叶子优先查 Unihan，再认笔画字符，最后看它自己的 IDS。
///
/// 返回的已经夹到 `13`（13+）；算不出来返回 `None`。
pub fn from_ids(
    character: char,
    ids_by_character: &BTreeMap<char, &str>,
    unihan: &Unihan,
) -> Option<u8> {
    let mut memo = HashMap::new();
    count(character, ids_by_character, unihan, &mut memo, 0)
}

fn count(
    character: char,
    ids_by_character: &BTreeMap<char, &str>,
    unihan: &Unihan,
    memo: &mut HashMap<char, u8>,
    depth: usize,
) -> Option<u8> {
    if let Some(value) = unihan.stroke(character) {
        return Some(value.min(MAX_STROKE));
    }
    if is_stroke(character) {
        return Some(1);
    }
    if depth >= 32 {
        return None;
    }
    if let Some(value) = memo.get(&character) {
        return Some(*value);
    }
    let ids = ids_by_character.get(&character)?;
    let node = parse(ids).ok()?;
    let total = count_node(&node, ids_by_character, unihan, memo, depth + 1)?;
    let value = total.min(u16::from(MAX_STROKE)) as u8;
    memo.insert(character, value);
    Some(value)
}

fn count_node(
    node: &IdsNode,
    ids_by_character: &BTreeMap<char, &str>,
    unihan: &Unihan,
    memo: &mut HashMap<char, u8>,
    depth: usize,
) -> Option<u16> {
    match node {
        IdsNode::Leaf(character) => Some(u16::from(count(
            *character,
            ids_by_character,
            unihan,
            memo,
            depth,
        )?)),
        IdsNode::Op { children, .. } => {
            let mut total = 0u16;
            for child in children {
                total =
                    total.checked_add(count_node(child, ids_by_character, unihan, memo, depth)?)?;
            }
            Some(total)
        }
        IdsNode::Unencoded(_) | IdsNode::Wildcard => None,
    }
}

/// 单个笔画字符自己就是一画。
fn is_stroke(character: char) -> bool {
    matches!(character, '\u{31C0}'..='\u{31E5}')
        || matches!(
            character,
            '一' | '丨' | '丶' | '丿' | '乙' | '亅' | '乀' | '乁' | '乚' | '乛' | '㇆' | '㇇'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unihan_and_takes_the_first_value() {
        let text =
            "# 注释\nU+6797\tkTotalStrokes\t8\nU+4E00\tkOther\t1\nU+4E01\tkTotalStrokes\t2 3\n";
        let unihan = Unihan::parse(text);
        assert_eq!(unihan.stroke('林'), Some(8));
        assert_eq!(unihan.stroke('丁'), Some(2));
        assert_eq!(unihan.stroke('一'), None);
    }

    #[test]
    fn counts_strokes_by_walking_the_ids_tree() {
        let mut ids = BTreeMap::new();
        ids.insert('林', "⿰木木");
        ids.insert('木', "⿻十人");
        let mut unihan = Unihan::default();
        unihan.strokes.insert('十', 2);
        unihan.strokes.insert('人', 2);
        assert_eq!(from_ids('木', &ids, &unihan), Some(4));
        assert_eq!(from_ids('林', &ids, &unihan), Some(8));
        // 认不出的部件算不出来，不瞎猜。
        ids.insert('森', "⿱木林");
        unihan.strokes.clear();
        assert_eq!(from_ids('森', &ids, &unihan), None);
    }

    #[test]
    fn clamps_to_thirteen_and_counts_stroke_characters_as_one() {
        let ids = BTreeMap::from([('㐀', "⿰一一")]);
        let unihan = Unihan::default();
        assert_eq!(from_ids('㐀', &ids, &unihan), Some(2));

        let mut unihan = Unihan::default();
        unihan.strokes.insert('鞭', 20);
        assert_eq!(from_ids('鞭', &BTreeMap::new(), &unihan), Some(MAX_STROKE));
    }
}
