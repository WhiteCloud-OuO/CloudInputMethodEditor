//! 精确查询与 `？` 通配查询。

use std::collections::{BTreeMap, HashMap};

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
        if let Some(id) = hash::find(&self.hash, &normalized, |id| {
            self.key_str(id).unwrap_or_default()
        }) {
            return self.matches_in_key(id, &normalized);
        }
        self.expanded_matches(&node)
    }

    /// 精确匹配没命中时，把数据里的部件递归展开再比一次。
    ///
    /// 例如数据里的「品」是 `⿱口吅`，用户摆出 `⿱口⿰口口` 也能命中。
    fn expanded_matches(&self, query: &IdsNode) -> Result<Vec<IdsMatch>, IdsError> {
        let mut found: BTreeMap<char, IdsMatch> = BTreeMap::new();
        let mut cache: HashMap<char, Option<IdsNode>> = HashMap::new();
        for id in 0..self.len() {
            let Some(text) = self.key_str(id as u32) else {
                continue;
            };
            let Ok(haystack) = parse(text) else {
                continue;
            };
            if !matches_expanded(query, &haystack, self, &mut cache, 0) {
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
                        ids: text.to_owned(),
                        sources: entry.sources,
                        flags: entry.flags,
                    });
            }
        }
        Ok(found.into_values().collect())
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

/// 递归展开匹配：数据里的叶子如果本身还有 IDS，就展开后再比。
fn matches_expanded(
    query: &IdsNode,
    haystack: &IdsNode,
    database: &IdsDatabase,
    cache: &mut HashMap<char, Option<IdsNode>>,
    depth: usize,
) -> bool {
    if depth > 12 {
        return false;
    }
    match (query, haystack) {
        (IdsNode::Wildcard, _) => true,
        (IdsNode::Leaf(expected), IdsNode::Leaf(actual)) => expected == actual,
        (IdsNode::Unencoded(expected), IdsNode::Unencoded(actual)) => expected == actual,
        (
            IdsNode::Op {
                op: query_op,
                children: query_children,
            },
            IdsNode::Op {
                op: haystack_op,
                children: haystack_children,
            },
        ) if query_op == haystack_op && query_children.len() == haystack_children.len() => {
            query_children
                .iter()
                .zip(haystack_children)
                .all(|(query, haystack)| matches_expanded(query, haystack, database, cache, depth))
        }
        (_, IdsNode::Leaf(character)) => {
            let expanded = if let Some(cached) = cache.get(character) {
                cached.clone()
            } else {
                let value = database
                    .first_ids_for(*character)
                    .and_then(|ids| parse(ids).ok());
                cache.insert(*character, value.clone());
                value
            };
            expanded.as_ref().is_some_and(|expanded| {
                matches_expanded(query, expanded, database, cache, depth + 1)
            })
        }
        _ => false,
    }
}
