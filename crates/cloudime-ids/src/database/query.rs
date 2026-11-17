//! 精确查询与 `？` 通配查询。

use std::collections::BTreeMap;

use cloudime_format::hash;

use crate::database::{IdsDatabase, IdsMatch, WildcardMatches};
use crate::error::IdsError;
use crate::parser::{IdsNode, parse};

impl IdsDatabase {
    /// 精确查询：`？` 与 `{n}` 都不接受，缩写会先展开。
    pub fn lookup_exact(&self, query: &str) -> Result<Vec<IdsMatch>, IdsError> {
        let node = parse(query)?;
        if node.contains_wildcard() {
            return Err(IdsError::WildcardNotAllowed);
        }
        if let Some(number) = node.first_unencoded() {
            return Err(IdsError::Unencoded(number));
        }
        let normalized = node.to_ids();
        let Some(id) = hash::find(&self.hash, &normalized, |id| {
            self.key_str(id).unwrap_or_default()
        }) else {
            return Ok(Vec::new());
        };
        self.matches_in_key(id, &normalized)
    }

    /// 通配查询：`？` 匹配任意单个子树，结果按码位升序、按字去重，最多 `limit` 条。
    pub fn lookup_wildcard(&self, query: &str, limit: usize) -> Result<WildcardMatches, IdsError> {
        let needle = parse(query)?;
        if needle == IdsNode::Wildcard {
            return Err(IdsError::BareWildcard);
        }
        if let Some(number) = needle.first_unencoded() {
            return Err(IdsError::Unencoded(number));
        }

        // 按码位排，天然满足「BMP 在前、扩展区在后」，同一个字只留一条。
        let mut found: BTreeMap<char, IdsMatch> = BTreeMap::new();
        for id in 0..self.len() {
            let Some(ids) = self.key_str(id as u32) else {
                continue;
            };
            let Ok(tree) = parse(ids) else {
                debug_assert!(false, "ids.qj 里的键应当都能解析：{ids}");
                continue;
            };
            if !matches_tree(&needle, &tree) {
                continue;
            }
            let Some(entries) = self.entries_at(id as u32) else {
                continue;
            };
            for entry in entries {
                let Some(character) = char::from_u32(entry.codepoint) else {
                    continue;
                };
                found
                    .entry(character)
                    .and_modify(|item| {
                        item.sources |= entry.sources;
                        item.flags |= entry.flags;
                    })
                    .or_insert_with(|| IdsMatch {
                        character,
                        ids: ids.to_owned(),
                        sources: entry.sources,
                        flags: entry.flags,
                    });
            }
        }

        let total = found.len();
        Ok(WildcardMatches {
            matches: found.into_values().take(limit).collect(),
            total,
        })
    }
}

/// 查询树能不能匹配一整个键树。
fn matches_tree(needle: &IdsNode, haystack: &IdsNode) -> bool {
    match needle {
        IdsNode::Wildcard => true,
        IdsNode::Leaf(expected) => matches!(haystack, IdsNode::Leaf(actual) if actual == expected),
        IdsNode::Unencoded(expected) => {
            matches!(haystack, IdsNode::Unencoded(actual) if actual == expected)
        }
        IdsNode::Op {
            op,
            children: needle_children,
        } => match haystack {
            IdsNode::Op {
                op: haystack_op,
                children: haystack_children,
            } => {
                op == haystack_op
                    && needle_children.len() == haystack_children.len()
                    && needle_children
                        .iter()
                        .zip(haystack_children)
                        .all(|(needle, haystack)| matches_tree(needle, haystack))
            }
            _ => false,
        },
    }
}
