//! 把聚合结果写成 `.qj`：`TEXT` / `KEYS` / `HASH` / `ENTR`。

use std::path::Path;

use cloudime_format::{Kind, Metadata, Writer, hash};
use cloudime_ids::{
    COMPONENT_TAG, Component, ENTR_TAG, HASH_TAG, IdsEntry, IdsKey, KEYS_TAG, TEXT_TAG,
};
use zerocopy::IntoBytes;

use crate::BuildError;
use crate::read::Rows;

/// 写出条数，给命令行打印。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub keys: usize,
    pub characters: usize,
    pub entries: usize,
    pub components: usize,
}

/// 写出一份 `ids.qj`；同一个输入两次生成的字节完全一致。
pub fn write(
    path: &Path,
    rows: &Rows,
    components: &[Component],
    mut metadata: Metadata,
) -> Result<Stats, BuildError> {
    if rows.keys.is_empty() {
        return Err(BuildError::Empty);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| BuildError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut text: Vec<u8> = Vec::new();
    let mut keys: Vec<IdsKey> = Vec::with_capacity(rows.keys.len());
    let mut entries: Vec<IdsEntry> = Vec::new();
    let mut ids: Vec<&str> = Vec::with_capacity(rows.keys.len());

    for (ids_text, characters) in &rows.keys {
        let start = u32::try_from(text.len()).map_err(|_| BuildError::TooLarge)?;
        let len = u16::try_from(ids_text.len()).map_err(|_| BuildError::TooLarge)?;
        text.extend_from_slice(ids_text.as_bytes());

        let first_entry = u32::try_from(entries.len()).map_err(|_| BuildError::TooLarge)?;
        for (character, entry) in characters {
            entries.push(IdsEntry {
                codepoint: *character as u32,
                sources: entry.sources,
                flags: entry.flags,
            });
        }
        let entry_count = u16::try_from(characters.len()).map_err(|_| BuildError::TooLarge)?;

        keys.push(IdsKey {
            start,
            first_entry,
            len,
            entry_count,
            reserved: 0,
            reserved2: 0,
        });
        ids.push(ids_text.as_str());
    }

    let hash = hash::build(ids.len(), |id| ids[id as usize]);
    metadata.entries = rows.characters.len() as u64;

    let mut writer = Writer::new(Kind::Ids, &metadata)?
        .section(TEXT_TAG, &text)
        .section(KEYS_TAG, keys.as_slice().as_bytes())
        .section(HASH_TAG, hash.as_slice().as_bytes())
        .section(ENTR_TAG, entries.as_slice().as_bytes());
    if !components.is_empty() {
        writer = writer.section(COMPONENT_TAG, components.as_bytes());
    }
    writer.write_to(path)?;

    Ok(Stats {
        keys: keys.len(),
        characters: rows.characters.len(),
        entries: entries.len(),
        components: components.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cloudime_ids::IdsDatabase;

    use super::*;
    use crate::read;

    const SAMPLE: &str = "U+6797\t林\t^⿰木木$(GHTJKPV)\n\
        U+6D77\t海\t^⿰氵每$(GHTKPV)\t^⿰氵毎$(J)\n\
        U+4E03\t七\t^〾⿻一乚$(GHJKPV)\n";

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cloudime-ids-build-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}-{}.qj", std::process::id()))
    }

    fn metadata() -> Metadata {
        Metadata {
            name: "IDS 测试".to_owned(),
            ..Metadata::default()
        }
    }

    #[test]
    fn writes_a_file_that_the_reader_can_query() {
        let rows = read::parse(SAMPLE).unwrap();
        let file = path("round-trip");
        let stats = write(&file, &rows, &[], metadata()).unwrap();
        assert_eq!(stats.characters, 3);
        assert_eq!(stats.keys, 4);
        assert_eq!(stats.components, 0);

        let database = IdsDatabase::open(&file).unwrap();
        assert!(!database.has_components());
        let matches = database.lookup_exact("⿰木木").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].character, '林');
        assert_eq!(database.metadata().entries, 3);
    }

    #[test]
    fn writes_the_optional_component_table() {
        let rows = read::parse(SAMPLE).unwrap();
        let components = [Component {
            codepoint: '木' as u32,
            roles: [0, 0, 1000, 0, 0],
            stroke: 4,
            primary: 2,
            flags: 0,
            reserved: 0,
        }];
        let file = path("components");
        let stats = write(&file, &rows, &components, metadata()).unwrap();
        assert_eq!(stats.components, 1);

        let database = IdsDatabase::open(&file).unwrap();
        assert!(database.has_components());
        assert_eq!(database.component('木').unwrap().stroke, 4);
    }

    #[test]
    fn output_is_byte_identical_across_runs() {
        let rows = read::parse(SAMPLE).unwrap();
        let first = path("deterministic-a");
        let second = path("deterministic-b");
        write(&first, &rows, &[], metadata()).unwrap();
        write(&second, &rows, &[], metadata()).unwrap();
        assert_eq!(
            std::fs::read(&first).unwrap(),
            std::fs::read(&second).unwrap()
        );
    }

    #[test]
    fn empty_input_is_rejected() {
        let rows = read::parse("# 只有注释\n").unwrap();
        assert!(matches!(
            write(&path("empty"), &rows, &[], metadata()),
            Err(BuildError::Empty)
        ));
    }
}
