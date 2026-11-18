//! 自动候选：从 IDS 数据找「出现得多、但词库打不出来」的部件。
//!
//! 只写候选清单，不直接改 `components.tsv`；人工挑完再并进去。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

use cloudime_ids::{ComponentRole, IdsNode, parse};

use crate::BuildError;
use crate::classify::Classification;
use crate::read::Rows;
use crate::stroke::{self, Unihan};

/// 一个候选部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub character: char,
    pub occurrences: usize,
    pub stroke: u8,
    pub primary: u8,
    pub roles: [u16; 5],
    pub typeable: bool,
}

/// 读 `dict.tsv`，把所有单字词条收进来：这些字能用拼音打出来。
pub fn typeable_from_dict(path: &Path) -> Result<HashSet<char>, BuildError> {
    let file = std::fs::File::open(path).map_err(|source| BuildError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut typeable = HashSet::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|source| BuildError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let word = line.split('\t').next().unwrap_or_default();
        if let Some(character) = single_character(word) {
            typeable.insert(character);
        }
    }
    Ok(typeable)
}

/// 统计每个部件出现在多少个不同的字里，过滤掉已收录的与太少的。
pub fn derive(
    rows: &Rows,
    existing: &HashSet<char>,
    typeable: &HashSet<char>,
    unihan: &Unihan,
    classifications: &HashMap<char, Classification>,
    minimum: usize,
    include_typeable: bool,
) -> Vec<Candidate> {
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut seen: HashSet<(char, char)> = HashSet::new();
    for (ids, characters) in &rows.keys {
        let Ok(tree) = parse(ids) else {
            continue;
        };
        let mut leaves = HashSet::new();
        collect_leaves(&tree, &mut leaves);
        for character in characters.keys() {
            for leaf in &leaves {
                if seen.insert((*character, *leaf)) {
                    *counts.entry(*leaf).or_default() += 1;
                }
            }
        }
    }

    let ids_by_character = rows.first_ids_by_character();
    let mut candidates: Vec<Candidate> = counts
        .into_iter()
        .filter(|(character, count)| {
            *count >= minimum
                && !existing.contains(character)
                && (include_typeable || !typeable.contains(character))
        })
        .map(|(character, occurrences)| {
            let classification = classifications.get(&character).copied();
            let primary = classification
                .map(|item| item.primary)
                .unwrap_or(ComponentRole::Other.index());
            let roles = classification.map_or_else(
                || {
                    let mut roles = [0u16; 5];
                    roles[primary as usize] = 1000;
                    roles
                },
                |item| item.roles,
            );
            let stroke = unihan
                .stroke(character)
                .map(|value| value.min(cloudime_ids::MAX_STROKE))
                .or_else(|| stroke::from_ids(character, &ids_by_character, unihan))
                .unwrap_or(0);
            Candidate {
                character,
                occurrences,
                stroke,
                primary,
                roles,
                typeable: typeable.contains(&character),
            }
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.typeable
            .cmp(&right.typeable)
            .then_with(|| right.occurrences.cmp(&left.occurrences))
            .then_with(|| left.character.cmp(&right.character))
    });
    candidates
}

/// 写候选清单；父目录不存在会建。
pub fn write(path: &Path, candidates: &[Candidate]) -> Result<(), BuildError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| BuildError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, render(candidates)).map_err(|source| BuildError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// 候选清单正文（纯函数，测试直接断言）。
pub fn render(candidates: &[Candidate]) -> String {
    let mut text = String::new();
    text.push_str("# 难输入部件自动候选（出现得多、词库里打不出来）\n");
    text.push_str("# 人工挑选后并入 assets/ids/components.tsv；出现字数 = 用到它的不同字的个数\n");
    text.push_str("字符\t出现字数\t笔画\t主分类\t围\t头\t旁\t底\t其它\t可打出\n");
    for candidate in candidates {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            candidate.character,
            candidate.occurrences,
            candidate.stroke,
            ComponentRole::from_index(candidate.primary).map_or("其它", |role| role.name()),
            candidate.roles[0],
            candidate.roles[1],
            candidate.roles[2],
            candidate.roles[3],
            candidate.roles[4],
            if candidate.typeable { "是" } else { "否" },
        ));
    }
    text
}

fn collect_leaves(node: &IdsNode, leaves: &mut HashSet<char>) {
    match node {
        IdsNode::Leaf(character) => {
            leaves.insert(*character);
        }
        IdsNode::Op { children, .. } => {
            for child in children {
                collect_leaves(child, leaves);
            }
        }
        IdsNode::Unencoded(_) | IdsNode::Wildcard => {}
    }
}

fn single_character(field: &str) -> Option<char> {
    let mut characters = field.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("cloudime-ids-build-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}-{}.tsv", std::process::id()))
    }

    #[test]
    fn reads_single_character_words_from_the_dictionary() {
        let path = temp("dict");
        std::fs::write(&path, "木\ta\t100\n林\tlin\t50\n木木\ta\t1\n# 注释\n").unwrap();
        let typeable = typeable_from_dict(&path).unwrap();
        assert!(typeable.contains(&'木'));
        assert!(typeable.contains(&'林'));
        assert_eq!(typeable.len(), 2);
    }

    #[test]
    fn derives_candidates_and_skips_existing_ones() {
        let rows = crate::read::parse(
            "U+6797\t林\t^⿰木木$(G)\n\
             U+68EE\t森\t^⿱木林$(G)\n\
             U+695A\t楚\t^⿱林疋$(G)\n\
             U+9716\t霖\t^⿱雨林$(G)\n\
             U+68F1\t棱\t^⿰木夌$(G)\n\
             U+758B\t疋\t^⿰疋夌$(G)\n",
        )
        .unwrap();
        let classifications = crate::classify::classify(&rows);
        let existing = HashSet::from(['木']);
        let typeable = HashSet::from(['夌', '疋']);
        let candidates = derive(
            &rows,
            &existing,
            &typeable,
            &Unihan::parse("U+6728\tkTotalStrokes\t4\nU+590C\tkTotalStrokes\t8\n"),
            &classifications,
            2,
            false,
        );
        let characters: Vec<char> = candidates.iter().map(|item| item.character).collect();
        // 木出现 3 次但已收录；林 3 次；夌 / 疋 各 2 次但可打出，默认不列。
        assert_eq!(characters, vec!['林']);
        assert_eq!(candidates[0].occurrences, 3);
        assert!(!candidates[0].typeable);
        assert_eq!(candidates[0].primary, ComponentRole::Bottom.index());

        let with_typeable = derive(
            &rows,
            &existing,
            &typeable,
            &Unihan::default(),
            &classifications,
            2,
            true,
        );
        let characters: HashSet<char> = with_typeable.iter().map(|item| item.character).collect();
        assert_eq!(characters, HashSet::from(['林', '夌', '疋']));
    }

    #[test]
    fn renders_header_and_rows() {
        let candidates = [Candidate {
            character: '𠂹',
            occurrences: 42,
            stroke: 8,
            primary: 1,
            roles: [0, 800, 100, 50, 50],
            typeable: false,
        }];
        let text = render(&candidates);
        assert!(text.contains("字符\t出现字数\t笔画"));
        assert!(text.contains("𠂹\t42\t8\t头\t0\t800\t100\t50\t50\t否\n"));
    }
}
