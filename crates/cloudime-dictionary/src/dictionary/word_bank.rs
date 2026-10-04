//! `WordBank\Dict.db`：中文与英文合一份的词库存档。
//!
//! 引擎按语言与稀有度分流：中文普通组装配成 [`DictDb::chinese`]、中文稀有组装配成
//! [`DictDb::rare`]（都可整体开关）、英文行装配成 [`DictDb::english`]。
//! 表结构与分类判据见 `db.rs` 与 `docs/notes/crate-notes.md`。

use std::path::Path;

use cloudime_format::Metadata;

use super::Dictionary;
use super::db;
use crate::error::DictionaryError;
use crate::word_list::WordList;

/// 随包词库的文件名（在 `WordBank\` 目录下）。
pub const WORD_BANK_FILE: &str = "Dict.db";

/// 一份 `Dict.db` 读出来的内容。
#[derive(Debug, Default)]
pub struct DictDb {
    /// 中文普通组（单字 / 地名 / 多字词 / 常见词）：按拼音查。
    pub chinese: Dictionary,

    /// 中文稀有组（生僻字 / 生僻词）：按拼音查，可整体跳过。
    pub rare: Dictionary,

    /// 英文行：整串大小写不敏感地查。
    pub english: WordList,

    /// 存档里的来历（名称、许可证、署名）；读进来的总会有。
    pub metadata: Option<Metadata>,
}

impl DictDb {
    /// 打开 `Dict.db`：按语言与稀有度分流成普通中文 / 稀有中文 / 英文。
    /// 非 SQLite（TSV / `.qj`，样例词库与开发数据）当单份中文词库读，没有稀有组与英文。
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DictionaryError> {
        let path = path.as_ref();
        if !db::is_db(path) {
            let chinese = Dictionary::from_path(path)?;
            let metadata = chinese.metadata().cloned();
            return Ok(Self {
                chinese,
                rare: Dictionary::default(),
                english: WordList::default(),
                metadata,
            });
        }
        let (chinese, rare, english, metadata) = db::open_word_bank(path)?;
        Ok(Self {
            chinese,
            rare,
            english,
            metadata: Some(metadata),
        })
    }

    /// 写成 `Dict.db`：7 张分类表一起落盘，`(text, pinyin, language)` 重复的权重大的胜出。
    pub fn write(&self, path: &Path, metadata: &Metadata) -> Result<(), DictionaryError> {
        db::write_word_bank(&self.chinese, &self.rare, &self.english, path, metadata)
    }

    /// 中英词条总数（含稀有组）。
    pub fn len(&self) -> usize {
        self.chinese.len() + self.rare.len() + self.english.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chinese.is_empty() && self.rare.is_empty() && self.english.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cloudime_format::Metadata;
    use rusqlite::{Connection, params};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cloudime-dict-db-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn db(chinese: &str, english: &str) -> DictDb {
        DictDb {
            chinese: Dictionary::parse(chinese).unwrap(),
            rare: Dictionary::default(),
            english: WordList::parse(english).unwrap(),
            metadata: None,
        }
    }

    /// 某张表里的 `(词, 权重)`，按词排序。
    fn rows(path: &Path, table: &str) -> Vec<(String, u32)> {
        let connection = Connection::open(path).unwrap();
        let mut statement = connection
            .prepare(&format!("SELECT text, weight FROM {table} ORDER BY text"))
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn meta(path: &Path, key: &str) -> String {
        let connection = Connection::open(path).unwrap();
        connection
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn classify_boundaries_pick_the_first_matching_table() {
        let path = temp("classify");
        // 权重 1：不管字数、不管末字，一律生僻字；单字权重够大归汉字；4 字地名归地名不进多字词；
        // 1 < 权重 < 100 归生僻词；权重够大且 4 字以上归多字词。
        let value = db(
            "某市\tmou shi\t1\n龘\tda\t1\n好\thao\t500\n\
             石家庄市\tshi jia zhuang shi\t500\n词汇量\tci hui liang\t30\n\
             词语测试\tci yu ce shi\t500\n常见\tchang jian\t500\n",
            "github\tgithub\t700\n",
        );
        value.write(&path, &Metadata::default()).unwrap();

        assert_eq!(
            rows(&path, "words_rare_char"),
            [("某市".to_owned(), 1), ("龘".to_owned(), 1)]
        );
        assert_eq!(rows(&path, "words_hanzi"), [("好".to_owned(), 500)]);
        assert_eq!(rows(&path, "words_rare_word"), [("词汇量".to_owned(), 30)]);
        assert_eq!(rows(&path, "words_place"), [("石家庄市".to_owned(), 500)]);
        assert!(
            !rows(&path, "words_long")
                .iter()
                .any(|(text, _)| text == "石家庄市"),
            "4 字地名不该同时进多字词"
        );
        assert_eq!(rows(&path, "words_long"), [("词语测试".to_owned(), 500)]);
        assert_eq!(rows(&path, "words_common"), [("常见".to_owned(), 500)]);
        assert_eq!(rows(&path, "words_english"), [("github".to_owned(), 700)]);

        assert_eq!(meta(&path, "entries"), "8");
        assert_eq!(meta(&path, "entries_words_rare_char"), "2");
        assert_eq!(meta(&path, "entries_words_place"), "1");
        assert_eq!(meta(&path, "entries_words_english"), "1");
        assert_eq!(meta(&path, "format"), "3");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_ordinary_rare_and_english() {
        let path = temp("roundtrip");
        let value = db(
            "开发\tkai fa\t9000\n龘\tda\t1\n生僻词\tcuo ci\t30\n",
            "GitHub\tgithub\t700\nhello\thello\t1000\n",
        );
        let metadata = Metadata {
            name: "测试词库".to_owned(),
            license: "MIT".to_owned(),
            ..Metadata::default()
        };
        value.write(&path, &metadata).unwrap();

        let loaded = DictDb::from_path(&path).unwrap();
        assert_eq!(loaded.chinese.lookup(&["kai", "fa"], false)[0].text, "开发");
        assert_eq!(loaded.rare.lookup(&["da"], false)[0].text, "龘");
        assert_eq!(loaded.rare.lookup(&["cuo", "ci"], false)[0].text, "生僻词");
        assert!(loaded.chinese.lookup(&["da"], false).is_empty());
        assert_eq!(loaded.english.get("github"), Some("GitHub"));
        assert_eq!(loaded.english.frequency("hello"), Some(1000));
        assert_eq!(loaded.metadata.as_ref().unwrap().name, "测试词库");
        assert_eq!(loaded.metadata.as_ref().unwrap().entries, 5);

        // 只走中文的旧入口把普通组与稀有组合并读回来
        let dictionary = Dictionary::from_path(&path).unwrap();
        assert_eq!(dictionary.lookup(&["cuo", "ci"], false)[0].text, "生僻词");
        assert_eq!(dictionary.len(), 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deduplicates_by_text_pinyin_language_keeping_the_heavier_weight() {
        let path = temp("dedupe");
        let value = DictDb {
            chinese: Dictionary::from_entries(vec![
                ("重复".to_owned(), "chong fu".to_owned(), 100),
                ("重复".to_owned(), "chong fu".to_owned(), 500),
            ])
            .unwrap(),
            rare: Dictionary::default(),
            english: WordList::default(),
            metadata: None,
        };
        value.write(&path, &Metadata::default()).unwrap();
        let loaded = DictDb::from_path(&path).unwrap();
        assert_eq!(loaded.chinese.len(), 1);
        assert_eq!(
            loaded.chinese.lookup(&["chong", "fu"], false)[0].frequency,
            500
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deduplicates_before_classifying_so_the_weight_decides_the_table() {
        let path = temp("dedupe-boundary");
        // 同一条 (词, 拼音) 权重 1 与 500：先按权重去重，再按 500 归常见词，不能一个进稀有组一个进普通组
        let value = DictDb {
            chinese: Dictionary::from_entries(vec![
                ("罕见".to_owned(), "han jian".to_owned(), 1),
                ("罕见".to_owned(), "han jian".to_owned(), 500),
            ])
            .unwrap(),
            rare: Dictionary::default(),
            english: WordList::default(),
            metadata: None,
        };
        value.write(&path, &Metadata::default()).unwrap();
        let loaded = DictDb::from_path(&path).unwrap();
        assert!(loaded.rare.is_empty());
        assert_eq!(loaded.chinese.len(), 1);
        assert_eq!(
            loaded.chinese.lookup(&["han", "jian"], false)[0].frequency,
            500
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reads_format2_split_into_ordinary_and_rare() {
        let path = temp("format2");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
                 CREATE TABLE words (
                     text TEXT NOT NULL, pinyin TEXT NOT NULL, language TEXT NOT NULL, weight INTEGER NOT NULL,
                     PRIMARY KEY (text, pinyin, language)
                 ) WITHOUT ROWID;",
            )
            .unwrap();
        connection
            .execute("INSERT INTO meta (key, value) VALUES ('format', '2')", [])
            .unwrap();
        for (text, pinyin, language, weight) in [
            ("开发", "kai fa", "中文", 9000),
            ("龘", "da", "中文", 1),
            ("生僻词", "cuo ci", "中文", 30),
            ("github", "github", "英文", 700),
        ] {
            connection
                .execute(
                    "INSERT INTO words (text, pinyin, language, weight) VALUES (?1, ?2, ?3, ?4)",
                    params![text, pinyin, language, weight],
                )
                .unwrap();
        }
        drop(connection);

        let loaded = DictDb::from_path(&path).unwrap();
        assert_eq!(loaded.chinese.lookup(&["kai", "fa"], false)[0].text, "开发");
        assert_eq!(loaded.rare.lookup(&["da"], false)[0].text, "龘");
        assert_eq!(loaded.rare.lookup(&["cuo", "ci"], false)[0].text, "生僻词");
        assert_eq!(loaded.english.get("github"), Some("github"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reads_legacy_schema_as_chinese_only() {
        let path = temp("legacy");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
                 CREATE TABLE keys (id INTEGER PRIMARY KEY, key TEXT NOT NULL);
                 CREATE TABLE words (key_id INTEGER NOT NULL, text TEXT NOT NULL, frequency INTEGER NOT NULL);
                 CREATE INDEX idx_words_key ON words (key_id);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                params!["format", "1"],
            )
            .unwrap();
        connection
            .execute("INSERT INTO keys (id, key) VALUES (0, 'kai fa')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO words (key_id, text, frequency) VALUES (0, '开发', 9000)",
                [],
            )
            .unwrap();
        drop(connection);

        let loaded = DictDb::from_path(&path).unwrap();
        assert!(loaded.english.is_empty());
        assert!(loaded.rare.is_empty());
        assert_eq!(loaded.chinese.lookup(&["kai", "fa"], false)[0].text, "开发");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_unknown_format_version() {
        let path = temp("badformat");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
                 CREATE TABLE words (text TEXT NOT NULL, pinyin TEXT NOT NULL, language TEXT NOT NULL, weight INTEGER NOT NULL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                params!["format", "99"],
            )
            .unwrap();
        drop(connection);
        assert!(DictDb::from_path(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
