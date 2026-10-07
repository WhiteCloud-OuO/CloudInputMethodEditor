//! `.db` 本地词典：SQLite，一张总表 `words` 加一张副表 `contents`。
//!
//! ```sql
//! -- 总表：所有词条。一个中文词可以有多条记录、每条只有一个译词；词性可复合
//! CREATE TABLE words (
//!     id          INTEGER PRIMARY KEY,   -- 索引值
//!     word        TEXT    NOT NULL,      -- 中文词
//!     translation TEXT    NOT NULL,      -- 英文翻译（一条记录一个）
//!     pos         TEXT,                  -- 词性（可复合，如 `n. adj.`）
//!     reading     TEXT                   -- 译词的读音（日语假名，如 `かいはつ`）；没有就空
//! );
//! CREATE INDEX words_by_word ON words (word);
//!
//! -- 副表：索引，查词先走它
//! CREATE TABLE contents (
//!     id       INTEGER PRIMARY KEY,      -- 索引值
//!     initials TEXT,                     -- 中文词拼音的首字母（云朵 → y）
//!     pinyin   TEXT,                     -- 中文词拼音
//!     word     TEXT NOT NULL,            -- 中文词
//!     reading  TEXT                      -- 中文词的读音（与 pinyin 同值，副表自带一份）
//! );
//! CREATE INDEX contents_by_word ON contents (word);
//! ```
//!
//! `words.reading` 是给 Tip 显示用的（`n. 開発(かいはつ)`）；旧结构没有这一列时读取端照常工作、
//! 只是没有读音（见 [`Db::has_reading`] 的降级）。
//!
//! 查询按「先副表、再总表」走：副表里没有这个词就直接算查不到（省一次总表扫描，副表就是那份索引）；
//! 只有副表**读不动**（表不在 / 语句出错）时才退回总表，别让整个词条因为索引坏掉而消失。
//! 副表要列全所有词条——它是快路径，漏登记的词查不到。
//!
//! 学习状态不在这张库里（词典文件是共享的、可能只读），在用户目录的 `translate.db`，见 [`crate::Learning`]。

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::error::TranslateError;
use crate::model::Sense;

/// 只读连着的 `.db` 词典。
pub(super) struct Db {
    connection: Connection,

    /// `words` 表有没有 `reading` 列：按旧结构（没有这一列）建的词典照样能查，只是没有读音。
    has_reading: bool,
}

impl Db {
    pub(super) fn open(path: &Path) -> Result<Self, TranslateError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // 只读词典。这里**不开** `PRAGMA mmap_size`：实测（`glossary/mod.rs` 的 `lookup_latency` 基准）
        // 开了只快约 12%（命中 42→37 µs、未命中 20→18 µs），说明开销在 SQLite 的每次查询机制上、不在读盘，
        // 而代价是常驻内存跟着被碰到的页涨到整份文件。翻译 Tip 每帧只查一次（高亮那一个候选），
        // 这点差别看不出来，宁可留住小内存；真要提速再把它打开。
        // 两张表都得在，缺了在打开时就报出来，别等查词时才失败
        for table in ["words", "contents"] {
            connection
                .prepare(&format!("SELECT * FROM {table} LIMIT 1"))
                .map_err(|error| {
                    tracing::warn!(path = %path.display(), %error, table, "词典 .db 里没有这张表");
                    error
                })?;
        }
        let has_reading = column_exists(&connection, "words", "reading")?;
        Ok(Self {
            connection,
            has_reading,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.connection
            .query_row("SELECT COUNT(DISTINCT word) FROM words", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_or(0, |count| usize::try_from(count).unwrap_or(0))
    }

    pub(super) fn lookup(&self, word: &str) -> Option<Vec<Sense>> {
        // 先看副表：它登记了这个词，才回总表取它的译词
        if !self.indexed(word) {
            return None;
        }
        self.translations(word).filter(|senses| !senses.is_empty())
    }

    /// 副表里有没有这个词。读不动（表不在 / 语句出错）时返回 `true`，退回总表查——
    /// 索引坏了顶多慢一次，不该让查到得到的东西查不到。
    fn indexed(&self, word: &str) -> bool {
        let Ok(mut statement) = self
            .connection
            .prepare_cached("SELECT 1 FROM contents WHERE word = ?1 LIMIT 1")
        else {
            tracing::warn!("词典副表的语句建不起来，直接回总表");
            return true;
        };
        match statement.query_row([word], |_| Ok(())) {
            Ok(()) => true,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(error) => {
                tracing::warn!(%error, "词典副表查不动，直接回总表");
                true
            }
        }
    }

    /// 总表里这个词的全部译词，按索引值排。
    fn translations(&self, word: &str) -> Option<Vec<Sense>> {
        let mut statement = self
            .connection
            .prepare_cached(self.translation_query())
            .ok()?;
        let rows = statement
            .query_map([word], |row| {
                Ok(Sense {
                    pos: normalize(row.get::<_, Option<String>>(1)?),
                    text: row.get(0)?,
                    reading: normalize(row.get::<_, Option<String>>(2)?),
                })
            })
            .ok()?;
        Some(rows.filter_map(Result::ok).collect())
    }

    /// 取译词的语句：老结构没有 `reading` 列时补一个 `NULL`，其余一字不差。
    fn translation_query(&self) -> &'static str {
        if self.has_reading {
            "SELECT translation, pos, reading FROM words WHERE word = ?1 ORDER BY id"
        } else {
            "SELECT translation, pos, NULL FROM words WHERE word = ?1 ORDER BY id"
        }
    }

    /// 全部词条（中文词 + 译词），按词排。转换工具用。
    pub(super) fn entries(&self) -> Vec<(String, Vec<Sense>)> {
        let sql = if self.has_reading {
            "SELECT word, translation, pos, reading FROM words ORDER BY word, id"
        } else {
            "SELECT word, translation, pos, NULL FROM words ORDER BY word, id"
        };
        let Ok(mut statement) = self.connection.prepare(sql) else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Sense {
                    pos: normalize(row.get::<_, Option<String>>(2)?),
                    text: row.get(1)?,
                    reading: normalize(row.get::<_, Option<String>>(3)?),
                },
            ))
        }) else {
            return Vec::new();
        };
        let mut entries: Vec<(String, Vec<Sense>)> = Vec::new();
        for (word, sense) in rows.flatten() {
            match entries.last_mut() {
                Some((last, senses)) if *last == word => senses.push(sense),
                _ => entries.push((word, vec![sense])),
            }
        }
        entries
    }
}

/// 表里有没有这一列（`PRAGMA table_info`）。
fn column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 空串当「没有」。
fn normalize(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::Glossary;

    /// 按定稿的字段结构建一份小词典：一个词多条译词、词性可复合、译词带读音。
    fn write_sample(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE words (id INTEGER PRIMARY KEY, word TEXT NOT NULL, translation TEXT NOT NULL, pos TEXT, reading TEXT);
                 CREATE TABLE contents (id INTEGER PRIMARY KEY, initials TEXT, pinyin TEXT, word TEXT NOT NULL, reading TEXT);
                 INSERT INTO words (id, word, translation, pos, reading) VALUES
                     (1, '悲伤', 'sad', 'adj.', NULL),
                     (2, '悲伤', 'sorrow', 'n.', NULL),
                     (3, '云朵', 'cloud', 'n. adj.', NULL),
                     (4, '开发', 'develop', 'v.', 'かいはつ');
                 INSERT INTO contents (id, initials, pinyin, word, reading) VALUES
                     (1, 'b', 'bei shang', '悲伤', 'bei shang'),
                     (2, 'y', 'yun duo', '云朵', 'yun duo'),
                     (3, 'k', 'kai fa', '开发', 'kai fa');",
            )
            .unwrap();
    }

    #[test]
    fn looks_up_every_translation_of_a_word() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-translate-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dict.db");
        write_sample(&path);

        let glossary = Glossary::open(&path).unwrap();
        assert_eq!(glossary.len(), 3, "三个中文词");
        let senses = glossary.lookup("悲伤").unwrap();
        assert_eq!(senses.len(), 2, "一个词两条译词");
        assert_eq!(senses[0].text, "sad");
        assert_eq!(senses[0].pos.as_deref(), Some("adj."));
        assert_eq!(senses[1].text, "sorrow");
        // 词性可复合：原样给出来（显示时整段斜体）
        assert_eq!(
            glossary.lookup("云朵").unwrap()[0].pos.as_deref(),
            Some("n. adj.")
        );
        // 译词的读音（日语假名）也搬过来了
        let sense = glossary.lookup("开发").unwrap().remove(0);
        assert_eq!(sense.reading.as_deref(), Some("かいはつ"));
        // 副表里没有的词：查不到
        assert!(glossary.lookup("没有这个词").is_none());
        // 连接还开着时 Windows 删不掉文件，先放掉
        drop(glossary);
        std::fs::remove_file(&path).unwrap();
    }

    /// 老结构（没有 `reading` 列）的词典照样能查，只是没有读音。
    #[test]
    fn opens_a_dictionary_without_the_reading_column() {
        let dir =
            std::env::temp_dir().join(format!("cloudime-translate-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE words (id INTEGER PRIMARY KEY, word TEXT NOT NULL, translation TEXT NOT NULL, pos TEXT);
                 CREATE TABLE contents (id INTEGER PRIMARY KEY, initials TEXT, pinyin TEXT, word TEXT NOT NULL);
                 INSERT INTO words VALUES (1, '云朵', 'cloud', 'n.');
                 INSERT INTO contents VALUES (1, 'y', 'yun duo', '云朵');",
            )
            .unwrap();
        drop(connection);

        let glossary = Glossary::open(&path).unwrap();
        let sense = glossary.lookup("云朵").unwrap().remove(0);
        assert_eq!(sense.text, "cloud");
        assert_eq!(sense.reading, None);
        drop(glossary);
        std::fs::remove_file(&path).unwrap();
    }
}
