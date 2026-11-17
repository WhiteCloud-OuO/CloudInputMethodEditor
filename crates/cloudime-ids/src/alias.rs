//! 常见部件等价写法：把用户输入的「水 / 手 / 火」换成 IDS 里常见的「氵 / 扌 / 灬」。
//!
//! 只在用户打开「替换」开关时用于查询，不改数据文件本身。每一组的第一个字符是规范形。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::ParseError;
use crate::parser::{IdsNode, parse};

/// 等价部件组；第一个字符是规范形。
pub const GROUPS: &[&str] = &[
    "氵水氺",
    "扌手",
    "忄心㣺",
    "灬火",
    "亻人",
    "讠言",
    "饣食飠",
    "钅金",
    "纟糸糹",
    "犭犬",
    "礻示",
    "衤衣",
    "阝阜邑",
    "刂刀",
    "艹艸",
    "辶辵",
    "爫爪",
    "⺮竹",
    "𧾷足",
];

/// 把一个部件换成它的规范形；不在表里就原样返回。
pub fn canonical_char(character: char) -> char {
    static MAP: OnceLock<HashMap<char, char>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for group in GROUPS {
            let mut characters = group.chars();
            let Some(canonical) = characters.next() else {
                continue;
            };
            for variant in characters {
                map.insert(variant, canonical);
            }
        }
        map
    });
    map.get(&character).copied().unwrap_or(character)
}

/// 把整个 IDS 表达式里的部件换成规范形。
pub fn canonicalize(input: &str) -> Result<String, ParseError> {
    let mut node = parse(input)?;
    canonicalize_node(&mut node);
    Ok(node.to_ids())
}

fn canonicalize_node(node: &mut IdsNode) {
    match node {
        IdsNode::Leaf(character) => *character = canonical_char(*character),
        IdsNode::Op { children, .. } => {
            for child in children {
                canonicalize_node(child);
            }
        }
        IdsNode::Unencoded(_) | IdsNode::Wildcard => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_component_forms() {
        assert_eq!(canonical_char('水'), '氵');
        assert_eq!(canonical_char('手'), '扌');
        assert_eq!(canonical_char('火'), '灬');
        assert_eq!(canonical_char('木'), '木');
    }

    #[test]
    fn canonicalizes_the_whole_expression_and_is_idempotent() {
        assert_eq!(canonicalize("⿰水木").unwrap(), "⿰氵木");
        assert_eq!(canonicalize("LR手木").unwrap(), "⿰扌木");
        assert_eq!(canonicalize("⿱艸早").unwrap(), "⿱艹早");
        let once = canonicalize("⿰水手").unwrap();
        assert_eq!(canonicalize(&once).unwrap(), once);
    }

    #[test]
    fn leaves_unknown_parts_and_unencoded_markers_alone() {
        assert_eq!(canonicalize("⿱{1}木").unwrap(), "⿱{1}木");
        assert_eq!(canonicalize("⿰木？").unwrap(), "⿰木？");
    }
}
