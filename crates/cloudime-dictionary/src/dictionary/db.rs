//! 词库的 SQLite 存档（WordBank 下的 `.db`）。
//!
//! 每张表都是 `text`（输出的字 / 词）、`pinyin`（中文音节空格分隔、英文原样写法）、`language`
//! （`中文` / `英文`）、`weight`（权重），外加 `meta`（名称 / 许可证 / 署名 / 来源 / 格式版本 / 条数）。
//! `(text, pinyin, language)` 唯一，去重由数据库的 upsert 保证，越大的权重胜出。
//!
//! 中文行按 [`classify`] 的 first-match 判据拆成 6 张表，英文行进 `words_english`：
//! 生僻字 / 生僻词（`weight == 1`、`1 < weight < 100`）是**稀有组**，可整组跳过查询；其余是**普通组**。
//! 读的时候按语言分流：中文还原成 `mod.rs` 里那套内存结构（两个 arena + 键索引 + 词目），英文还原成 [`WordList`]；
//! 热路径不查库。
//!
//! 旧存档仍能读回来：`format` 2（单张 `words` 表带 `language`，中文行按判据拆两组）与
//! `format` 1（`keys` + `words(key_id, text, frequency)`，只有中文、没有分流）；新写出去的一律是 `format` 3。

use std::path::Path;

use cloudime_format::Metadata;
use rusqlite::{Connection, OpenFlags, params};

use super::{Dictionary, Row, Slot};
use crate::error::DictionaryError;
use crate::word_list::WordList;

/// SQLite 文件头。
const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// 当前存档格式版本。
const FORMAT: &str = "3";

/// 单张 `words` 表 + `language` 的旧存档格式，只读兼容。
const FORMAT_2: &str = "2";

/// `keys` + `words` 那套老存档格式，只读兼容。
const LEGACY_FORMAT: &str = "1";

/// 中文行的语言值。
const CHINESE: &str = "中文";

/// 英文行的语言值。
const ENGLISH: &str = "英文";

/// 英文表名（不在 `ChineseTable` 的分类里，单独处理）。
const ENGLISH_TABLE: &str = "words_english";

/// 7 张表的建表顺序（写出时也按它统计条数）。
const ALL_TABLES: [&str; 7] = [
    "words_rare_char",
    "words_hanzi",
    "words_english",
    "words_rare_word",
    "words_place",
    "words_long",
    "words_common",
];

/// 普通组：中文里常见、整句词图也用的部分。
const ORDINARY_TABLES: [&str; 4] = ["words_hanzi", "words_place", "words_long", "words_common"];

/// 稀有组：可整体开关的部分。
const RARE_TABLES: [&str; 2] = ["words_rare_char", "words_rare_word"];

/// 中文全部 6 张表（读单份中文词库时全读）。
const CHINESE_TABLES: [&str; 6] = [
    "words_rare_char",
    "words_hanzi",
    "words_rare_word",
    "words_place",
    "words_long",
    "words_common",
];

/// 7 张表共用的列定义与主键（`WITHOUT ROWID` 让主键自己就是那棵 B 树）。
const TABLE_COLUMNS: &str = " (
    text     TEXT    NOT NULL,
    pinyin   TEXT    NOT NULL,
    language TEXT    NOT NULL,
    weight   INTEGER NOT NULL,
    PRIMARY KEY (text, pinyin, language)
) WITHOUT ROWID;";

/// 中文条目的目标表（first-match，判据见 `docs/notes/crate-notes.md`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChineseTable {
    /// `weight == 1`：生僻字 / 方言词。
    RareChar,

    /// 恰好 1 个字符：单字 / 一级二级汉字。
    Hanzi,

    /// `1 < weight < 100`：生僻词。
    RareWord,

    /// 末字是地名后缀。
    Place,

    /// 字符数 ≥ 4。
    Long,

    /// 其余常见词。
    Common,
}

impl ChineseTable {
    /// 分类判据的判定顺序（first-match）。
    const ALL: [Self; 6] = [
        Self::RareChar,
        Self::Hanzi,
        Self::RareWord,
        Self::Place,
        Self::Long,
        Self::Common,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::RareChar => "words_rare_char",
            Self::Hanzi => "words_hanzi",
            Self::RareWord => "words_rare_word",
            Self::Place => "words_place",
            Self::Long => "words_long",
            Self::Common => "words_common",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|table| *table == self)
            .expect("分类表都在 ALL 里")
    }

    /// 是不是稀有组（[`RARE_TABLES`]）。
    fn is_rare(self) -> bool {
        matches!(self, Self::RareChar | Self::RareWord)
    }
}

/// 地名的常见末字（暂只按末字判，见 `docs/notes/crate-notes.md`）。
const PLACE_SUFFIXES: &[char] = &['省', '县', '乡', '镇', '市', '区', '州', '府', '旗'];

/// 中文条目按固定的 first-match 顺序归类，保证每行只进一张表。
fn classify(text: &str, weight: u32) -> ChineseTable {
    if weight == 1 {
        return ChineseTable::RareChar;
    }
    let characters = text.chars().count();
    if characters == 1 {
        return ChineseTable::Hanzi;
    }
    if weight < 100 {
        return ChineseTable::RareWord;
    }
    if text
        .chars()
        .last()
        .is_some_and(|character| PLACE_SUFFIXES.contains(&character))
    {
        return ChineseTable::Place;
    }
    if characters >= 4 {
        return ChineseTable::Long;
    }
    ChineseTable::Common
}

/// 这个文件是不是 SQLite 词库（看文件头，不看扩展名）。
pub(crate) fn is_db(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 16];
    file.read_exact(&mut head).is_ok() && &head == MAGIC
}

/// 打开 `.db`：只取中文行（普通组 + 稀有组合并），还原成内存词库（旧存档也认）。
pub(crate) fn open(path: &Path) -> Result<Dictionary, DictionaryError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let metadata = read_metadata(&connection)?;
    let mut dictionary = if has_table(&connection, "words_common")? {
        read_tables(&connection, &CHINESE_TABLES)?
    } else if has_column(&connection, "language")? {
        read_chinese(&connection)?
    } else {
        read_legacy(&connection)?
    };
    tracing::debug!(
        entries = dictionary.len(),
        name = %metadata.name,
        "词库已从 SQLite 读入"
    );
    dictionary.metadata = Some(metadata);
    Ok(dictionary)
}

/// 打开 `Dict.db`：普通中文 / 稀有中文 / 英文各还原成一份（旧存档只有中文）。
pub(crate) fn open_word_bank(
    path: &Path,
) -> Result<(Dictionary, Dictionary, WordList, Metadata), DictionaryError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let metadata = read_metadata(&connection)?;
    // 格式 3：7 张表各读各的
    if has_table(&connection, "words_common")? {
        let chinese = read_tables(&connection, &ORDINARY_TABLES)?;
        let rare = read_tables(&connection, &RARE_TABLES)?;
        let english = read_english(&connection)?;
        tracing::debug!(
            chinese = chinese.len(),
            rare = rare.len(),
            english = english.len(),
            name = %metadata.name,
            "词库已从 SQLite 读入（普通 / 稀有 / 英文）"
        );
        return Ok((chinese, rare, english, metadata));
    }
    // 格式 1：只有中文，没有分流
    if !has_column(&connection, "language")? {
        let dictionary = read_legacy(&connection)?;
        return Ok((
            dictionary,
            Dictionary::default(),
            WordList::default(),
            metadata,
        ));
    }
    // 格式 2：单张 words 表，中文行按判据拆普通 / 稀有
    let (chinese, rare, english) = read_split(&connection)?;
    tracing::debug!(
        chinese = chinese.len(),
        rare = rare.len(),
        english = english.len(),
        name = %metadata.name,
        "词库已从 SQLite 读入（中英分流）"
    );
    Ok((chinese, rare, english, metadata))
}

/// 写成 `.db`：只写中文行（导入的单份词库用），稀有组为空。
pub(crate) fn write(
    dictionary: &Dictionary,
    path: &Path,
    metadata: &Metadata,
) -> Result<(), DictionaryError> {
    write_word_bank(
        dictionary,
        &Dictionary::default(),
        &WordList::default(),
        path,
        metadata,
    )
}

/// 写成 `.db`：7 张分类表一起落盘（覆盖已有文件）。
///
/// 中文行先按 `(text, pinyin, language)` 去重（权重大的胜出），再按 [`classify`] 归表，
/// 所以同一 `(词, 拼音)` 不会因为权重跨判据而同时落在两张表里。
pub(crate) fn write_word_bank(
    chinese: &Dictionary,
    rare: &Dictionary,
    english: &WordList,
    path: &Path,
    metadata: &Metadata,
) -> Result<(), DictionaryError> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut connection = Connection::open(path)?;
    create_tables(&connection)?;
    let transaction = connection.transaction()?;
    {
        let mut staging = transaction.prepare(
            "INSERT INTO staging (text, pinyin, language, weight) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (text, pinyin, language) DO UPDATE SET weight = max(weight, excluded.weight)",
        )?;
        for entry in chinese.entries().chain(rare.entries()) {
            staging.execute(params![
                entry.text,
                entry.pinyin,
                CHINESE,
                i64::from(entry.frequency)
            ])?;
        }
        for (code, word, frequency) in english.entries() {
            staging.execute(params![code, word, ENGLISH, i64::from(frequency)])?;
        }
    }
    {
        let mut chinese_statements: Vec<rusqlite::Statement> = ChineseTable::ALL
            .iter()
            .map(|table| transaction.prepare(&insert_sql(table.name())))
            .collect::<Result<_, _>>()?;
        let mut english_statement = transaction.prepare(&insert_sql(ENGLISH_TABLE))?;
        let mut select =
            transaction.prepare("SELECT text, pinyin, language, weight FROM staging")?;
        let mut rows = select.query([])?;
        while let Some(row) = rows.next()? {
            let text: String = row.get(0)?;
            let pinyin: String = row.get(1)?;
            let language: String = row.get(2)?;
            let weight: u32 = row.get(3)?;
            if language == ENGLISH {
                english_statement.execute(params![text, pinyin, language, i64::from(weight)])?;
            } else {
                let table = classify(&text, weight);
                chinese_statements[table.index()].execute(params![
                    text,
                    pinyin,
                    language,
                    i64::from(weight)
                ])?;
            }
        }
    }
    {
        let mut total = 0i64;
        for table in ALL_TABLES {
            total += count(&transaction, table)?;
        }
        let mut insert = transaction.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
        for (key, value) in [
            ("format", FORMAT.to_owned()),
            ("name", metadata.name.clone()),
            ("license", metadata.license.clone()),
            ("attribution", metadata.attribution.clone()),
            ("source", metadata.source.clone()),
            ("entries", total.to_string()),
        ] {
            insert.execute(params![key, value])?;
        }
        for table in ALL_TABLES {
            insert.execute(params![
                format!("entries_{table}"),
                count(&transaction, table)?.to_string()
            ])?;
        }
    }
    transaction.execute_batch("DROP TABLE staging")?;
    transaction.commit()?;
    // 收拾页空间：分发的词库小一点，读起来也省事
    connection.execute_batch("VACUUM")?;
    Ok(())
}

/// 建 meta + 临时去重表 staging + 7 张分类表。
fn create_tables(connection: &Connection) -> Result<(), DictionaryError> {
    let mut schema = String::from(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;\n\
         CREATE TABLE staging (\n    text     TEXT    NOT NULL,\n    pinyin   TEXT    NOT NULL,\n    language TEXT    NOT NULL,\n    weight   INTEGER NOT NULL,\n    PRIMARY KEY (text, pinyin, language)\n) WITHOUT ROWID;\n",
    );
    for table in ALL_TABLES {
        schema.push_str("CREATE TABLE ");
        schema.push_str(table);
        schema.push_str(TABLE_COLUMNS);
        schema.push('\n');
    }
    connection.execute_batch(&schema)?;
    Ok(())
}

fn insert_sql(table: &str) -> String {
    format!("INSERT INTO {table} (text, pinyin, language, weight) VALUES (?1, ?2, ?3, ?4)")
}

fn count(connection: &Connection, table: &str) -> Result<i64, DictionaryError> {
    Ok(
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?,
    )
}

/// 读一组表，拼成一份中文词库。
fn read_tables(connection: &Connection, tables: &[&str]) -> Result<Dictionary, DictionaryError> {
    let mut entries: Vec<(String, String, u32)> = Vec::new();
    for table in tables {
        entries.extend(read_entries(connection, table)?);
    }
    Dictionary::from_entries(entries)
}

fn read_entries(
    connection: &Connection,
    table: &str,
) -> Result<Vec<(String, String, u32)>, DictionaryError> {
    let mut statement = connection.prepare(&format!("SELECT text, pinyin, weight FROM {table}"))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
        ))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 读 `words_english`，装成英文词表；编码取 `text`、原样写法取 `pinyin`。
fn read_english(connection: &Connection) -> Result<WordList, DictionaryError> {
    Ok(WordList::from_entries(read_entries(
        connection,
        ENGLISH_TABLE,
    )?))
}

/// 格式 2：单张 `words` 表，中文行按 [`classify`] 拆普通 / 稀有，英文行装词表。
fn read_split(
    connection: &Connection,
) -> Result<(Dictionary, Dictionary, WordList), DictionaryError> {
    let mut chinese: Vec<(String, String, u32)> = Vec::new();
    let mut rare: Vec<(String, String, u32)> = Vec::new();
    let mut english: Vec<(String, String, u32)> = Vec::new();
    {
        let mut statement =
            connection.prepare("SELECT text, pinyin, language, weight FROM words")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let text: String = row.get(0)?;
            let pinyin: String = row.get(1)?;
            let language: String = row.get(2)?;
            let weight: u32 = row.get(3)?;
            if language == ENGLISH {
                english.push((text, pinyin, weight));
            } else if classify(&text, weight).is_rare() {
                rare.push((text, pinyin, weight));
            } else {
                chinese.push((text, pinyin, weight));
            }
        }
    }
    Ok((
        Dictionary::from_entries(chinese)?,
        Dictionary::from_entries(rare)?,
        WordList::from_entries(english),
    ))
}

/// 中文行：按语言过滤后整段读回（格式 2）。
fn read_chinese(connection: &Connection) -> Result<Dictionary, DictionaryError> {
    let mut statement =
        connection.prepare("SELECT text, pinyin, weight FROM words WHERE language = ?1")?;
    let rows = statement.query_map(params![CHINESE], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
        ))
    })?;
    let entries = rows.collect::<Result<Vec<_>, _>>()?;
    Dictionary::from_entries(entries)
}

/// 旧存档：`keys` 表给拼音键，`words` 表按 `key_id` 归组。
fn read_legacy(connection: &Connection) -> Result<Dictionary, DictionaryError> {
    let keys: Vec<String> = {
        let mut statement = connection.prepare("SELECT key FROM keys ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut keys_arena = String::new();
    let mut offsets: Vec<(u32, u16)> = Vec::with_capacity(keys.len());
    for key in &keys {
        offsets.push((
            u32::try_from(keys_arena.len())
                .map_err(|_| DictionaryError::Corrupt("key arena too large"))?,
            u16::try_from(key.len()).map_err(|_| DictionaryError::Corrupt("pinyin too long"))?,
        ));
        keys_arena.push_str(key);
    }
    let mut texts = String::new();
    let mut rows = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT key_id, text, frequency FROM words ORDER BY key_id, rowid")?;
        let mut result = statement.query([])?;
        while let Some(row) = result.next()? {
            let key_id = row.get::<_, i64>(0)?;
            let text: String = row.get(1)?;
            let frequency: u32 = row.get(2)?;
            let (key_start, key_len) = offsets
                .get(usize::try_from(key_id).unwrap_or(usize::MAX))
                .copied()
                .ok_or(DictionaryError::Corrupt(
                    "word points outside the key table",
                ))?;
            let text_start = u32::try_from(texts.len())
                .map_err(|_| DictionaryError::Corrupt("text arena too large"))?;
            let text_len =
                u16::try_from(text.len()).map_err(|_| DictionaryError::Corrupt("word too long"))?;
            texts.push_str(&text);
            rows.push(Row {
                key_start,
                key_len,
                slot: Slot {
                    text_start,
                    frequency,
                    text_len,
                    reserved: 0,
                },
            });
        }
    }
    Ok(Dictionary::assemble(texts, keys_arena, rows))
}

/// `words` 表里有没有某一列（用列名认旧 schema）。
fn has_column(connection: &Connection, name: &str) -> Result<bool, DictionaryError> {
    let mut statement = connection.prepare("PRAGMA table_info(words)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let columns: Vec<String> = rows.collect::<Result<_, _>>()?;
    Ok(columns.iter().any(|column| column == name))
}

/// 库里有没有某张表（用表名认格式 2 / 3）。
fn has_table(connection: &Connection, name: &str) -> Result<bool, DictionaryError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn read_metadata(connection: &Connection) -> Result<Metadata, DictionaryError> {
    let mut metadata = Metadata::default();
    let mut statement = connection.prepare("SELECT key, value FROM meta")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        match key.as_str() {
            "format" => {
                if value != FORMAT && value != FORMAT_2 && value != LEGACY_FORMAT {
                    return Err(DictionaryError::Corrupt(
                        "unsupported dictionary format version",
                    ));
                }
            }
            "name" => metadata.name = value,
            "license" => metadata.license = value,
            "attribution" => metadata.attribution = value,
            "source" => metadata.source = value,
            "entries" => metadata.entries = value.parse().unwrap_or_default(),
            _ => {}
        }
    }
    Ok(metadata)
}
