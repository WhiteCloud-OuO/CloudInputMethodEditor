//! `.qj` 释义表：与青简同一种容器（`Kind::Glossary`，`TEXT` / `ENTR` / `SENS` / `HASH` 四节）。
//!
//! 词按字节序排在词条表里，词与释义的字符串都进 `TEXT` arena，查词走 `HASH` 的开放寻址索引。
//! 打开即映射，不解析、不复制。

use std::path::Path;

use cloudime_format::{Container, Kind, Table, Text, hash};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::error::TranslateError;
use crate::model::Sense;

const TEXT_TAG: [u8; 4] = *b"TEXT";
const ENTRIES_TAG: [u8; 4] = *b"ENTR";
const SENSES_TAG: [u8; 4] = *b"SENS";
const HASH_TAG: [u8; 4] = *b"HASH";

/// 一条词条：词在 arena 里的位置 + 它的释义在释义表里的一段。12 字节、无填充。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct EntryRecord {
    word_start: u32,
    sense_start: u32,
    word_len: u16,
    sense_count: u16,
}

/// 一条释义：译文 / 读音 / 词性三段在 arena 里的位置。20 字节、无填充。
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct SenseRecord {
    text_start: u32,
    reading_start: u32,
    pos_start: u32,
    text_len: u16,
    reading_len: u16,
    pos_len: u16,
    reserved: u16,
}

/// 映射着的 `.qj` 释义表。
pub(super) struct Qj {
    text: Text,
    entries: Table<EntryRecord>,
    senses: Table<SenseRecord>,
    index: Table<u32>,
}

impl Qj {
    pub(super) fn open(path: &Path) -> Result<Self, TranslateError> {
        let container = Container::open(path, Kind::Glossary)?;
        let text = container.text(TEXT_TAG)?;
        let entries: Table<EntryRecord> = container.table(ENTRIES_TAG)?;
        let senses: Table<SenseRecord> = container.table(SENSES_TAG)?;
        let index: Table<u32> = container.table(HASH_TAG)?;
        // 偏移一律校验落在 arena 内：坏文件宁可打不开，也别读到别的节的字节。
        let inside = |start: u32, len: u16| {
            text.get(start as usize..start as usize + usize::from(len))
                .is_some()
        };
        for entry in entries.iter() {
            if !inside(entry.word_start, entry.word_len)
                || entry.sense_start as usize + usize::from(entry.sense_count) > senses.len()
            {
                return Err(TranslateError::Corrupt("词条指向了释义表 / arena 之外"));
            }
        }
        for sense in senses.iter() {
            if !inside(sense.text_start, sense.text_len)
                || !inside(sense.reading_start, sense.reading_len)
                || !inside(sense.pos_start, sense.pos_len)
            {
                return Err(TranslateError::Corrupt("释义指向了 arena 之外"));
            }
        }
        if !hash::is_valid(&index, entries.len()) {
            return Err(TranslateError::Corrupt("哈希索引与词条表对不上"));
        }
        Ok(Self {
            text,
            entries,
            senses,
            index,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// 第 `id` 个词。
    fn word(&self, id: u32) -> Option<&str> {
        let entry = self.entries.get(id as usize)?;
        self.slice(entry.word_start, entry.word_len)
    }

    fn slice(&self, start: u32, len: u16) -> Option<&str> {
        self.text
            .get(start as usize..start as usize + usize::from(len))
    }

    /// arena 里的一段；空串当「没有」。
    fn part(&self, start: u32, len: u16) -> Option<Option<String>> {
        let text = self.slice(start, len)?;
        Some((!text.is_empty()).then(|| text.to_owned()))
    }

    pub(super) fn lookup(&self, word: &str) -> Option<Vec<Sense>> {
        let id = hash::find(&self.index, word, |id| self.word(id).unwrap_or(""))?;
        let senses = self.senses_of(id)?;
        (!senses.is_empty()).then_some(senses)
    }

    /// 全部词条（词 + 释义），按词条表顺序（写入时按字节序排过）。
    pub(super) fn entries(&self) -> Vec<(String, Vec<Sense>)> {
        (0..self.entries.len() as u32)
            .filter_map(|id| Some((self.word(id)?.to_owned(), self.senses_of(id)?)))
            .collect()
    }

    /// 第 `id` 条词条的释义。
    fn senses_of(&self, id: u32) -> Option<Vec<Sense>> {
        let entry = self.entries.get(id as usize)?;
        let start = entry.sense_start as usize;
        let records = self
            .senses
            .get(start..start + usize::from(entry.sense_count))?;
        records
            .iter()
            .map(|record| {
                Some(Sense {
                    pos: self.part(record.pos_start, record.pos_len)?,
                    text: self.slice(record.text_start, record.text_len)?.to_owned(),
                    reading: self.part(record.reading_start, record.reading_len)?,
                })
            })
            .collect::<Option<Vec<_>>>()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Qj;

    /// 仓库根 `LocalDictionary\glossary-en.qj`：随仓库带的成品词典，不在就跳过（比如 CI 没拉数据）。
    fn shipped() -> Option<PathBuf> {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../LocalDictionary/glossary-en.qj");
        path.is_file().then_some(path)
    }

    #[test]
    fn opens_the_shipped_dictionary_and_looks_up_its_own_entries() {
        let Some(path) = shipped() else {
            eprintln!("跳过：LocalDictionary/glossary-en.qj 不在");
            return;
        };
        let qj = Qj::open(&path).unwrap();
        assert!(qj.len() > 10_000, "词条数只有 {}", qj.len());
        // 抽查几条真实词条：词能从 arena 取出来、哈希索引也命得中
        for id in [0, 1, qj.len() as u32 / 2, qj.len() as u32 - 1] {
            let word = qj
                .word(id)
                .unwrap_or_else(|| panic!("第 {id} 条词条指向了 arena 之外"))
                .to_owned();
            let senses = qj
                .lookup(&word)
                .unwrap_or_else(|| panic!("「{word}」查不到"));
            assert!(!senses.is_empty(), "「{word}」没有释义");
        }
        assert!(qj.lookup("这个词肯定不在任何词典里").is_none());
    }
}
