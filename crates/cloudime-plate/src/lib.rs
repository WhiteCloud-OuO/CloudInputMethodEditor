//! 车牌对照表：`省份简称拼音 + 字母` → 车牌 / 完整所在地 / 短地名。
//!
//! 文件用 [`cloudime_format`] 的 `.qj` 容器（种类 [`Kind::Plate`]），打开就是 mmap：
//! - `PLAT`：定长 [`PlateRecord`] 表，按 key 排好序，key 相同的记录相邻；
//! - `TEXT`：所有字符串 arena，记录里存偏移与长度。
//!
//! key 例：`sua`（苏 A）、`jinga`（京 A）。查询拿 key 精确匹配或前缀匹配。

use std::path::Path;

use cloudime_format::{Container, FormatError, Kind, Metadata, Table, Text, Writer};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// 一条记录在 `TEXT` arena 里的四段字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, Immutable, KnownLayout, IntoBytes)]
#[repr(C)]
pub struct PlateRecord {
    pub key_offset: u32,
    pub key_len: u32,
    pub plate_offset: u32,
    pub plate_len: u32,
    pub full_offset: u32,
    pub full_len: u32,
    pub short_offset: u32,
    pub short_len: u32,
}

/// 写文件前的一条明文记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlateRow {
    /// 省份简称拼音 + 字母，小写 ASCII（`sua`）。
    pub key: String,

    /// 车牌本身（`苏A`）。
    pub plate: String,

    /// 完整所在地（`江苏省南京市`）。
    pub full: String,

    /// 短地名（`江苏南京`）。
    pub short: String,
}

/// 一条查出来的车牌对照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlateEntry<'a> {
    pub key: &'a str,
    pub plate: &'a str,
    pub full: &'a str,
    pub short: &'a str,
}

/// 打开的车牌表（mmap）。
pub struct PlateTable {
    records: Table<PlateRecord>,
    text: Text,
    metadata: Metadata,
}

impl PlateTable {
    /// 打开并校验文件类型、记录范围和所有字符串 UTF-8。
    pub fn open(path: &Path) -> Result<Self, PlateError> {
        let container = Container::open(path, Kind::Plate)?;
        let records = container.table::<PlateRecord>(*b"PLAT")?;
        let text = container.text(*b"TEXT")?;
        let metadata = container.metadata().clone();
        let table = Self {
            records,
            text,
            metadata,
        };
        table.validate()?;
        Ok(table)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<PlateEntry<'_>> {
        self.records
            .get(index)
            .and_then(|record| self.entry(record))
    }

    /// key 完全相同的记录（key 排序、相同 key 相邻）。
    pub fn lookup<'a>(&'a self, key: &'a str) -> impl Iterator<Item = PlateEntry<'a>> + 'a {
        let start = self
            .records
            .partition_point(|record| self.key(record).is_some_and(|record_key| record_key < key));
        self.records[start..]
            .iter()
            .take_while(move |record| self.key(record).is_some_and(|record_key| record_key == key))
            .filter_map(move |record| self.entry(record))
    }

    /// key 以 `prefix` 开头的记录，最多 `limit` 条。输入到一半时用。
    pub fn prefix<'a>(
        &'a self,
        prefix: &'a str,
        limit: usize,
    ) -> impl Iterator<Item = PlateEntry<'a>> + 'a {
        let start = self.records.partition_point(|record| {
            self.key(record)
                .is_some_and(|record_key| record_key < prefix)
        });
        self.records[start..]
            .iter()
            .take_while(move |record| {
                self.key(record)
                    .is_some_and(|record_key| record_key.starts_with(prefix))
            })
            .take(limit)
            .filter_map(move |record| self.entry(record))
    }

    fn validate(&self) -> Result<(), PlateError> {
        let text_len = self.text.len();
        for (index, record) in self.records.iter().enumerate() {
            for (offset, len) in [
                (record.key_offset, record.key_len),
                (record.plate_offset, record.plate_len),
                (record.full_offset, record.full_len),
                (record.short_offset, record.short_len),
            ] {
                let end = (offset as usize)
                    .checked_add(len as usize)
                    .ok_or(PlateError::BadRange(index))?;
                if end > text_len {
                    return Err(PlateError::BadRange(index));
                }
            }
            if self.key(record).is_none() {
                return Err(PlateError::BadText(index));
            }
        }
        Ok(())
    }

    fn entry(&self, record: &PlateRecord) -> Option<PlateEntry<'_>> {
        Some(PlateEntry {
            key: self.slice(record.key_offset, record.key_len)?,
            plate: self.slice(record.plate_offset, record.plate_len)?,
            full: self.slice(record.full_offset, record.full_len)?,
            short: self.slice(record.short_offset, record.short_len)?,
        })
    }

    fn key<'a>(&'a self, record: &PlateRecord) -> Option<&'a str> {
        self.slice(record.key_offset, record.key_len)
    }

    fn slice(&self, offset: u32, len: u32) -> Option<&str> {
        let bytes = self
            .text
            .as_bytes()
            .get(offset as usize..(offset as usize).checked_add(len as usize)?)?;
        std::str::from_utf8(bytes).ok()
    }
}

/// 写出一份车牌 `.qj`。记录会按 key / 车牌 / 地名排序并去重；`entries` 自动填成记录数。
pub fn write_qj(
    path: &Path,
    rows: impl IntoIterator<Item = PlateRow>,
    mut metadata: Metadata,
) -> Result<(), PlateError> {
    let mut rows: Vec<PlateRow> = rows.into_iter().collect();
    rows.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.plate.cmp(&right.plate))
            .then_with(|| left.full.cmp(&right.full))
            .then_with(|| left.short.cmp(&right.short))
    });
    rows.dedup();

    let mut text = Vec::new();
    let mut records = Vec::with_capacity(rows.len());
    for row in &rows {
        let key = push_text(&mut text, &row.key)?;
        let plate = push_text(&mut text, &row.plate)?;
        let full = push_text(&mut text, &row.full)?;
        let short = push_text(&mut text, &row.short)?;
        records.push(PlateRecord {
            key_offset: key.0,
            key_len: key.1,
            plate_offset: plate.0,
            plate_len: plate.1,
            full_offset: full.0,
            full_len: full.1,
            short_offset: short.0,
            short_len: short.1,
        });
    }
    metadata.entries = records.len() as u64;
    Writer::new(Kind::Plate, &metadata)?
        .section(*b"PLAT", records.as_bytes())
        .section(*b"TEXT", &text)
        .write_to(path)?;
    Ok(())
}

fn push_text(text: &mut Vec<u8>, value: &str) -> Result<(u32, u32), PlateError> {
    let offset = u32::try_from(text.len()).map_err(|_| PlateError::TooLarge)?;
    let len = u32::try_from(value.len()).map_err(|_| PlateError::TooLarge)?;
    text.extend_from_slice(value.as_bytes());
    Ok((offset, len))
}

#[derive(Debug, thiserror::Error)]
pub enum PlateError {
    #[error(transparent)]
    Format(#[from] FormatError),

    /// 记录里的字符串范围越过 `TEXT` 分节。
    #[error("车牌记录 {0} 的字符串范围越界")]
    BadRange(usize),

    /// 记录里的字符串不是 UTF-8。
    #[error("车牌记录 {0} 的字符串不是 UTF-8")]
    BadText(usize),

    /// 数据文件超过 4 GiB，偏移 / 长度放不进 `u32`。
    #[error("车牌数据太大，偏移或长度超过 u32")]
    TooLarge,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("cloudime-plate-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}-{}.qj", std::process::id()))
    }

    #[test]
    fn round_trips_and_looks_up_by_exact_key_and_prefix() {
        let file = path("round-trip");
        let rows = vec![
            PlateRow {
                key: "sua".into(),
                plate: "苏A".into(),
                full: "江苏省南京市".into(),
                short: "江苏南京".into(),
            },
            PlateRow {
                key: "sub".into(),
                plate: "苏B".into(),
                full: "江苏省无锡市".into(),
                short: "江苏无锡".into(),
            },
            PlateRow {
                key: "jinga".into(),
                plate: "京A".into(),
                full: "北京市".into(),
                short: "北京".into(),
            },
        ];
        let metadata = Metadata {
            name: "测试车牌".into(),
            ..Metadata::default()
        };
        write_qj(&file, rows, metadata).unwrap();
        let table = PlateTable::open(&file).unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table.metadata().name, "测试车牌");
        assert_eq!(table.metadata().entries, 3);

        let sua: Vec<_> = table.lookup("sua").collect();
        assert_eq!(sua.len(), 1);
        assert_eq!(sua[0].plate, "苏A");
        assert_eq!(sua[0].full, "江苏省南京市");
        assert_eq!(sua[0].short, "江苏南京");
        assert!(table.lookup("nope").next().is_none());

        let prefix: Vec<_> = table.prefix("su", 8).collect();
        assert_eq!(prefix.len(), 2);
        assert_eq!(prefix[0].key, "sua");
        assert_eq!(prefix[1].key, "sub");
        assert_eq!(table.prefix("su", 1).count(), 1);

        let _ = std::fs::remove_file(&file);
    }
}
