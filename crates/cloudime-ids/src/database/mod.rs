//! `ids.qj` 的读取与查询。

mod layout;
mod query;

use std::path::Path;

use cloudime_format::{Container, FormatError, Kind, Metadata, Table, Text, hash};

use crate::component::{
    COMPONENT_NO_SVG, COMPONENT_TAG, COMPONENT_TEXT_TAG, Component, ComponentRole, MAX_STROKE,
};
use crate::error::IdsError;

pub use layout::{ENTR_TAG, HASH_TAG, IdsEntry, IdsKey, KEYS_TAG, TEXT_TAG};

/// 一条查出来的字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdsMatch {
    /// 字本身。
    pub character: char,

    /// 命中的规范化 IDS。
    pub ids: String,

    /// 来源位图。
    pub sources: u16,

    /// 标记位图。
    pub flags: u16,
}

/// 通配查询的结果：前 `limit` 条 + 命中总数（按字去重）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WildcardMatches {
    /// 按码位升序截断后的结果。
    pub matches: Vec<IdsMatch>,

    /// 去重后的命中总数。
    pub total: usize,
}

/// 打开（mmap）的 IDS 组字库。
#[derive(Debug)]
pub struct IdsDatabase {
    text: Text,
    keys: Table<IdsKey>,
    hash: Table<u32>,
    entries: Table<IdsEntry>,
    components: Table<Component>,
    component_text: Option<Text>,
    metadata: Metadata,
}

impl IdsDatabase {
    /// 打开并校验文件类型、键表排序、哈希表与条目范围。
    pub fn open(path: &Path) -> Result<Self, IdsError> {
        let container = Container::open(path, Kind::Ids)?;
        // `COMP` 是第二版才有的可选分节：旧文件没有它，第一版功能照常。
        let components = match container.table::<Component>(COMPONENT_TAG) {
            Ok(table) => table,
            Err(FormatError::MissingSection(_)) => Table::default(),
            Err(error) => return Err(IdsError::Format(error)),
        };
        let component_text = match container.text(COMPONENT_TEXT_TAG) {
            Ok(text) => Some(text),
            Err(FormatError::MissingSection(_)) => None,
            Err(error) => return Err(IdsError::Format(error)),
        };
        let database = Self {
            text: container.text(TEXT_TAG)?,
            keys: container.table::<IdsKey>(KEYS_TAG)?,
            hash: container.table::<u32>(HASH_TAG)?,
            entries: container.table::<IdsEntry>(ENTR_TAG)?,
            components,
            component_text,
            metadata: container.metadata().clone(),
        };
        database.validate()?;
        Ok(database)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// IDS 键的数量（不是字数）。
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// 第 `id` 个键的 IDS 字符串。
    pub fn key_str(&self, id: u32) -> Option<&str> {
        self.key_text(self.keys.get(id as usize)?)
    }

    /// 第 `id` 个键对应的全部条目。
    pub fn entries_at(&self, id: u32) -> Option<&[IdsEntry]> {
        let key = self.keys.get(id as usize)?;
        self.entries_of(key, id as usize).ok()
    }

    /// 难输入部件表；文件里没有 `COMP` 时是空切片。
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    /// 文件里有没有部件表。
    pub fn has_components(&self) -> bool {
        !self.components.is_empty()
    }

    /// 按字符查部件（`COMP` 按码位排序，二分）。
    pub fn component(&self, character: char) -> Option<&Component> {
        let index = self
            .components
            .binary_search_by_key(&(character as u32), |item| item.codepoint)
            .ok()?;
        self.components.get(index)
    }

    /// 部件的内置 SVG（完整 `<svg>` 文档）。
    pub fn component_svg(&self, component: &Component) -> Option<&str> {
        if component.svg_off == COMPONENT_NO_SVG {
            return None;
        }
        let text = self.component_text.as_ref()?;
        let start = component.svg_off as usize;
        let end = start.checked_add(component.svg_len as usize)?;
        std::str::from_utf8(text.as_bytes().get(start..end)?).ok()
    }

    /// 按字符取内置 SVG。
    pub fn component_svg_for(&self, character: char) -> Option<&str> {
        self.component(character)
            .and_then(|component| self.component_svg(component))
    }

    /// 某个笔画档位的部件（顺序与 `COMP` 一致）。
    pub fn components_with_stroke(&self, stroke: u8) -> impl Iterator<Item = &Component> {
        self.components
            .iter()
            .filter(move |component| component.stroke == stroke)
    }

    /// 某个分类下分值达到 `minimum_permille` 的部件。
    ///
    /// 排序：该分类分值降序 → 主分类等于该分类优先 → 码位升序。
    pub fn components_in_role(
        &self,
        role: ComponentRole,
        minimum_permille: u16,
    ) -> Vec<&Component> {
        let index = role.index() as usize;
        let mut items: Vec<&Component> = self
            .components
            .iter()
            .filter(|component| component.roles[index] >= minimum_permille)
            .collect();
        items.sort_by(|left, right| {
            right.roles[index]
                .cmp(&left.roles[index])
                .then_with(|| (right.primary == role.index()).cmp(&(left.primary == role.index())))
                .then_with(|| left.codepoint.cmp(&right.codepoint))
        });
        items
    }

    fn key_text(&self, key: &IdsKey) -> Option<&str> {
        let start = key.start as usize;
        let end = start.checked_add(key.len as usize)?;
        std::str::from_utf8(self.text.as_bytes().get(start..end)?).ok()
    }

    fn entries_of(&self, key: &IdsKey, key_index: usize) -> Result<&[IdsEntry], IdsError> {
        let start = key.first_entry as usize;
        let end = start
            .checked_add(key.entry_count as usize)
            .ok_or(IdsError::BadEntryRange(key_index))?;
        self.entries
            .get(start..end)
            .ok_or(IdsError::BadEntryRange(key_index))
    }

    fn matches_in_key(&self, id: u32, ids: &str) -> Result<Vec<IdsMatch>, IdsError> {
        let index = id as usize;
        let key = self.keys.get(index).ok_or(IdsError::BadKey(index))?;
        let entries = self.entries_of(key, index)?;
        let mut matches = Vec::with_capacity(entries.len());
        for entry in entries {
            let character = char::from_u32(entry.codepoint).ok_or(IdsError::BadCodepoint(index))?;
            matches.push(IdsMatch {
                character,
                ids: ids.to_owned(),
                sources: entry.sources,
                flags: entry.flags,
            });
        }
        matches.sort_by_key(|item| item.character as u32);
        matches.dedup_by_key(|item| item.character);
        Ok(matches)
    }

    fn validate(&self) -> Result<(), IdsError> {
        if !hash::is_valid(&self.hash, self.keys.len()) {
            return Err(IdsError::BadHash);
        }
        let mut previous: Option<&str> = None;
        for (index, key) in self.keys.iter().enumerate() {
            let text = self.key_text(key).ok_or(IdsError::BadKey(index))?;
            if let Some(previous) = previous
                && previous >= text
            {
                return Err(IdsError::UnsortedKey(index));
            }
            previous = Some(text);

            let entries = self.entries_of(key, index)?;
            let mut last: Option<u32> = None;
            for entry in entries {
                if char::from_u32(entry.codepoint).is_none() {
                    return Err(IdsError::BadCodepoint(index));
                }
                if let Some(last) = last
                    && entry.codepoint <= last
                {
                    return Err(IdsError::UnsortedEntry(index));
                }
                last = Some(entry.codepoint);
            }
        }
        self.validate_components()
    }

    fn validate_components(&self) -> Result<(), IdsError> {
        let mut previous: Option<u32> = None;
        for (index, component) in self.components.iter().enumerate() {
            if char::from_u32(component.codepoint).is_none() {
                return Err(IdsError::BadComponent(index));
            }
            if let Some(previous) = previous
                && component.codepoint <= previous
            {
                return Err(IdsError::BadComponent(index));
            }
            previous = Some(component.codepoint);

            if component.stroke > MAX_STROKE
                || ComponentRole::from_index(component.primary).is_none()
            {
                return Err(IdsError::BadComponent(index));
            }
            if component.svg_off != COMPONENT_NO_SVG {
                let text = self
                    .component_text
                    .as_ref()
                    .ok_or(IdsError::BadComponent(index))?;
                let start = component.svg_off as usize;
                let end = start
                    .checked_add(component.svg_len as usize)
                    .ok_or(IdsError::BadComponent(index))?;
                let bytes = text
                    .as_bytes()
                    .get(start..end)
                    .ok_or(IdsError::BadComponent(index))?;
                if std::str::from_utf8(bytes).is_err() {
                    return Err(IdsError::BadComponent(index));
                }
            }
            let total: u32 = component.roles.iter().map(|score| u32::from(*score)).sum();
            if total != 1000 {
                return Err(IdsError::BadComponent(index));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cloudime_format::{Metadata, Writer};
    use zerocopy::IntoBytes;

    use super::*;

    /// 一个键下的若干条 `(字, 来源, 标记)`。
    type FixtureKey<'a> = (&'a str, &'a [(char, u16, u16)]);

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cloudime-ids-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}-{}.qj", std::process::id()))
    }

    fn write(name: &str, mut rows: Vec<FixtureKey<'_>>) -> PathBuf {
        rows.sort_by(|left, right| left.0.cmp(right.0));
        let mut text = String::new();
        let mut keys = Vec::new();
        let mut entries = Vec::new();
        let mut ids = Vec::new();
        for (ids_text, items) in &rows {
            let start = text.len() as u32;
            text.push_str(ids_text);
            let first_entry = entries.len() as u32;
            let mut items = items.to_vec();
            items.sort_by_key(|(character, _, _)| *character as u32);
            for (character, sources, flags) in items {
                entries.push(IdsEntry {
                    codepoint: character as u32,
                    sources,
                    flags,
                });
            }
            keys.push(IdsKey {
                start,
                first_entry,
                len: ids_text.len() as u16,
                entry_count: (entries.len() as u32 - first_entry) as u16,
                reserved: 0,
                reserved2: 0,
            });
            ids.push(*ids_text);
        }
        let hash = hash::build(ids.len(), |id| ids[id as usize]);
        let metadata = Metadata {
            name: "IDS 测试".to_owned(),
            ..Metadata::default()
        };
        let file = path(name);
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .write_to(&file)
            .unwrap();
        file
    }

    fn sample(name: &str) -> IdsDatabase {
        IdsDatabase::open(&write(
            name,
            vec![
                ("⿰木木", &[('林', 0b1, 0)]),
                ("⿰木水", &[('沐', 0b10, 0)]),
                ("⿰氵每", &[('海', 0b100, 0)]),
                ("⿰氵毎", &[('海', 0b1000, 0)]),
                ("⿱一一", &[('二', 1, 0), ('𠄞', 1, 0)]),
                ("⿱木林", &[('森', 1, 0)]),
            ],
        ))
        .unwrap()
    }

    #[test]
    fn exact_lookup_returns_every_character_of_the_key() {
        let database = sample("exact");
        let matches = database.lookup_exact("⿰木木").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].character, '林');
        assert_eq!(matches[0].ids, "⿰木木");

        let multiple = database.lookup_exact("⿱一一").unwrap();
        let characters: Vec<char> = multiple.iter().map(|item| item.character).collect();
        assert_eq!(characters, vec!['二', '𠄞']);

        // 同一个字有多条来源 IDS：两个键都能查到。
        let first = database.lookup_exact("⿰氵每").unwrap();
        let second = database.lookup_exact("⿰氵毎").unwrap();
        assert_eq!(first[0].character, '海');
        assert_eq!(second[0].character, '海');
        assert_ne!(first[0].sources, second[0].sources);
    }

    #[test]
    fn exact_lookup_accepts_abbreviations_and_rejects_wildcards() {
        let database = sample("exact-forms");
        assert_eq!(database.lookup_exact("LR木木").unwrap()[0].character, '林');
        assert!(matches!(
            database.lookup_exact("⿰木？"),
            Err(IdsError::WildcardNotAllowed)
        ));
        assert!(database.lookup_exact("⿰木火").unwrap().is_empty());
    }

    #[test]
    fn wildcard_lookup_matches_one_subtree_and_sorts_by_codepoint() {
        let database = sample("wildcard");
        let found = database.lookup_wildcard("⿰木？", 200).unwrap();
        let characters: Vec<char> = found.matches.iter().map(|item| item.character).collect();
        assert_eq!(characters, vec!['林', '沐']);
        assert_eq!(found.total, 2);

        // 同一个运算符、不同位置：`⿱木？` 只命中 `⿱木林`。
        let above = database.lookup_wildcard("⿱木？", 200).unwrap();
        let above_characters: Vec<char> = above.matches.iter().map(|item| item.character).collect();
        assert_eq!(above_characters, vec!['森']);

        // 结构不同（⿴ vs ⿰）不算命中。
        let none = database.lookup_wildcard("⿴木？", 200).unwrap();
        assert!(none.matches.is_empty());
        assert_eq!(none.total, 0);
    }

    #[test]
    fn wildcard_lookup_merges_the_same_character_and_respects_limit() {
        let database = sample("wildcard-merge");
        let found = database.lookup_wildcard("⿰氵？", 200).unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.matches[0].character, '海');
        assert_eq!(found.matches[0].sources, 0b1100);

        let limited = database.lookup_wildcard("⿰木？", 1).unwrap();
        assert_eq!(limited.total, 2);
        assert_eq!(limited.matches.len(), 1);
    }

    #[test]
    fn bare_wildcard_and_unencoded_queries_are_rejected() {
        let database = sample("reject");
        assert!(matches!(
            database.lookup_wildcard("？", 200),
            Err(IdsError::BareWildcard)
        ));
        assert!(matches!(
            database.lookup_wildcard("⿰木{1}", 200),
            Err(IdsError::Unencoded(1))
        ));
        assert!(matches!(
            database.lookup_exact("⿱{2}口"),
            Err(IdsError::Unencoded(2))
        ));
    }

    #[test]
    fn reads_optional_component_table() {
        let file = path("components");
        let metadata = Metadata::default();
        let text = "⿰木木";
        let keys = [IdsKey {
            start: 0,
            first_entry: 0,
            len: 9,
            entry_count: 1,
            reserved: 0,
            reserved2: 0,
        }];
        let entries = [IdsEntry {
            codepoint: '林' as u32,
            sources: 1,
            flags: 0,
        }];
        let components = [
            Component {
                codepoint: '木' as u32,
                roles: [0, 0, 900, 100, 0],
                stroke: 4,
                primary: 2,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
            Component {
                codepoint: '錒' as u32,
                roles: [0, 0, 0, 0, 1000],
                stroke: 13,
                primary: 4,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
        ];
        let hash = hash::build(1, |_| "⿰木木");
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .section(COMPONENT_TAG, components.as_slice().as_bytes())
            .write_to(&file)
            .unwrap();

        let database = IdsDatabase::open(&file).unwrap();
        assert!(database.has_components());
        assert_eq!(database.components().len(), 2);
        let wood = database.component('木').unwrap();
        assert_eq!(wood.stroke, 4);
        assert_eq!(wood.primary_role().name(), "旁");
        assert_eq!(wood.role_score(crate::ComponentRole::Side), 900);
        assert!(database.component('水').is_none());
        assert_eq!(database.components_with_stroke(4).count(), 1);
        assert_eq!(database.components_with_stroke(9).count(), 0);
    }

    #[test]
    fn components_in_role_filters_and_sorts() {
        let file = path("role-order");
        let metadata = Metadata::default();
        let text = "⿰木木";
        let keys = [IdsKey {
            start: 0,
            first_entry: 0,
            len: 9,
            entry_count: 1,
            reserved: 0,
            reserved2: 0,
        }];
        let entries = [IdsEntry {
            codepoint: '林' as u32,
            sources: 1,
            flags: 0,
        }];
        let components = [
            Component {
                codepoint: '一' as u32,
                roles: [0, 500, 500, 0, 0],
                stroke: 1,
                primary: 1,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
            Component {
                codepoint: '木' as u32,
                roles: [0, 0, 900, 100, 0],
                stroke: 4,
                primary: 2,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
            Component {
                codepoint: '目' as u32,
                roles: [0, 0, 600, 400, 0],
                stroke: 5,
                primary: 2,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
            Component {
                codepoint: '眼' as u32,
                roles: [0, 0, 600, 0, 400],
                stroke: 11,
                primary: 2,
                flags: 0,
                reserved: 0,
                svg_off: COMPONENT_NO_SVG,
                svg_len: 0,
                reserved2: 0,
            },
        ];
        let hash = hash::build(1, |_| "⿰木木");
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .section(COMPONENT_TAG, components.as_slice().as_bytes())
            .write_to(&file)
            .unwrap();

        let database = IdsDatabase::open(&file).unwrap();
        let side: Vec<char> = database
            .components_in_role(ComponentRole::Side, 100)
            .iter()
            .filter_map(|component| component.character())
            .collect();
        assert_eq!(side, vec!['木', '目', '眼', '一']);
        let top: Vec<char> = database
            .components_in_role(ComponentRole::Top, 100)
            .iter()
            .filter_map(|component| component.character())
            .collect();
        assert_eq!(top, vec!['一']);
        assert_eq!(
            database.components_in_role(ComponentRole::Side, 700).len(),
            1
        );
    }

    #[test]
    fn reads_component_svg_from_ctext() {
        let file = path("component-svg");
        let metadata = Metadata::default();
        let text = "⿰木木";
        let keys = [IdsKey {
            start: 0,
            first_entry: 0,
            len: 9,
            entry_count: 1,
            reserved: 0,
            reserved2: 0,
        }];
        let entries = [IdsEntry {
            codepoint: '林' as u32,
            sources: 1,
            flags: 0,
        }];
        let svg = "<svg></svg>";
        let components = [Component {
            codepoint: '木' as u32,
            roles: [0, 0, 1000, 0, 0],
            stroke: 4,
            primary: 2,
            flags: 0,
            reserved: 0,
            svg_off: 0,
            svg_len: svg.len() as u16,
            reserved2: 0,
        }];
        let hash = hash::build(1, |_| "⿰木木");
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .section(COMPONENT_TAG, components.as_slice().as_bytes())
            .section(COMPONENT_TEXT_TAG, svg.as_bytes())
            .write_to(&file)
            .unwrap();

        let database = IdsDatabase::open(&file).unwrap();
        assert_eq!(database.component_svg_for('木'), Some(svg));
        assert_eq!(database.component_svg_for('水'), None);
    }

    #[test]
    fn component_table_with_bad_scores_is_rejected() {
        let file = path("bad-components");
        let metadata = Metadata::default();
        let text = "⿰木木";
        let keys = [IdsKey {
            start: 0,
            first_entry: 0,
            len: 9,
            entry_count: 1,
            reserved: 0,
            reserved2: 0,
        }];
        let entries = [IdsEntry {
            codepoint: '林' as u32,
            sources: 1,
            flags: 0,
        }];
        let components = [Component {
            codepoint: '木' as u32,
            roles: [0, 0, 500, 0, 0],
            stroke: 4,
            primary: 2,
            flags: 0,
            reserved: 0,
            svg_off: COMPONENT_NO_SVG,
            svg_len: 0,
            reserved2: 0,
        }];
        let hash = hash::build(1, |_| "⿰木木");
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .section(COMPONENT_TAG, components.as_slice().as_bytes())
            .write_to(&file)
            .unwrap();
        assert!(matches!(
            IdsDatabase::open(&file),
            Err(IdsError::BadComponent(0))
        ));
    }

    #[test]
    fn files_without_a_component_table_still_open() {
        let database = sample("no-components");
        assert!(!database.has_components());
        assert!(database.components().is_empty());
        assert!(database.component('木').is_none());
    }

    #[test]
    fn unsorted_keys_are_rejected_on_open() {
        let file = path("unsorted");
        let metadata = Metadata::default();
        let text = "⿰木木⿰木水";
        let keys = [
            IdsKey {
                start: 9,
                first_entry: 1,
                len: 9,
                entry_count: 1,
                reserved: 0,
                reserved2: 0,
            },
            IdsKey {
                start: 0,
                first_entry: 0,
                len: 9,
                entry_count: 1,
                reserved: 0,
                reserved2: 0,
            },
        ];
        let entries = [
            IdsEntry {
                codepoint: '林' as u32,
                sources: 1,
                flags: 0,
            },
            IdsEntry {
                codepoint: '沐' as u32,
                sources: 1,
                flags: 0,
            },
        ];
        let hash = hash::build(2, |id| if id == 0 { "⿰木木" } else { "⿰木水" });
        Writer::new(Kind::Ids, &metadata)
            .unwrap()
            .section(TEXT_TAG, text.as_bytes())
            .section(KEYS_TAG, keys.as_slice().as_bytes())
            .section(HASH_TAG, hash.as_slice().as_bytes())
            .section(ENTR_TAG, entries.as_slice().as_bytes())
            .write_to(&file)
            .unwrap();
        assert!(matches!(
            IdsDatabase::open(&file),
            Err(IdsError::UnsortedKey(1))
        ));
    }
}
