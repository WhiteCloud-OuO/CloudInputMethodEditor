//! 写出云朵的 `.db` 词库：单张 `words(text, pinyin, language, weight)` 表 + `meta`。
//!
//! 这就是引擎 `DictDb::from_path` 的 `format 2` 分支（单张表带 `language`，中文行按判据拆普通 / 稀有），
//! 可以直接丢进 `WordBank\`。

use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::Error;
use crate::source::Entry;

/// 存档格式版本：单张 `words` 表。
const FORMAT: &str = "2";

/// 写出词库（覆盖已有文件）。
pub(crate) fn write(path: &Path, entries: &[Entry], name: &str) -> Result<(), Error> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|source| Error::Delete {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let mut connection = Connection::open(path).map_err(sqlite(path))?;
    connection
        .execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
             CREATE TABLE words (
                 text     TEXT    NOT NULL,
                 pinyin   TEXT    NOT NULL,
                 language TEXT    NOT NULL,
                 weight   INTEGER NOT NULL,
                 PRIMARY KEY (text, pinyin, language)
             ) WITHOUT ROWID;",
        )
        .map_err(sqlite(path))?;
    let transaction = connection.transaction().map_err(sqlite(path))?;
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO words (text, pinyin, language, weight) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (text, pinyin, language) DO UPDATE SET weight = max(weight, excluded.weight)",
            )
            .map_err(sqlite(path))?;
        for entry in entries {
            insert
                .execute(params![
                    entry.text,
                    entry.pinyin,
                    entry.language.as_str(),
                    i64::from(entry.weight)
                ])
                .map_err(sqlite(path))?;
        }
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")
            .map_err(sqlite(path))?;
        for (key, value) in [
            ("format", FORMAT.to_owned()),
            ("name", name.to_owned()),
            ("entries", entries.len().to_string()),
        ] {
            insert.execute(params![key, value]).map_err(sqlite(path))?;
        }
    }
    transaction.commit().map_err(sqlite(path))?;
    // 收拾页空间，分发的词库小一点。
    connection.execute_batch("VACUUM").map_err(sqlite(path))?;
    Ok(())
}

/// 把 rusqlite 的错误带上路径。
fn sqlite(path: &Path) -> impl Fn(rusqlite::Error) -> Error + '_ {
    move |source| Error::Sqlite {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_dictionary::DictDb;

    use crate::source::Language;

    fn entry(text: &str, pinyin: &str, weight: u32, language: Language) -> Entry {
        Entry {
            text: text.to_owned(),
            pinyin: pinyin.to_owned(),
            weight,
            language,
        }
    }

    #[test]
    fn the_engine_reads_back_what_we_write() {
        let path = std::env::temp_dir().join(format!("cwt-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let entries = [
            entry("开发", "kai fa", 9000, Language::Chinese),
            entry("龘", "da", 1, Language::Chinese),
            entry("github", "github", 700, Language::English),
        ];
        write(&path, &entries, "测试词库").unwrap();

        let loaded = DictDb::from_path(&path).unwrap();
        assert_eq!(loaded.chinese.lookup(&["kai", "fa"], false)[0].text, "开发");
        // 权重 1 → 稀有组
        assert_eq!(loaded.rare.lookup(&["da"], false)[0].text, "龘");
        assert_eq!(loaded.english.get("github"), Some("github"));
        assert_eq!(loaded.metadata.as_ref().unwrap().name, "测试词库");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rewriting_overwrites_the_old_file() {
        let path = std::env::temp_dir().join(format!("cwt-rewrite-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write(
            &path,
            &[entry("开发", "kai fa", 9000, Language::Chinese)],
            "一",
        )
        .unwrap();
        write(
            &path,
            &[entry("词语", "ci yu", 500, Language::Chinese)],
            "二",
        )
        .unwrap();
        let loaded = DictDb::from_path(&path).unwrap();
        assert_eq!(loaded.chinese.len(), 1);
        assert_eq!(loaded.chinese.lookup(&["ci", "yu"], false)[0].text, "词语");
        assert_eq!(loaded.metadata.as_ref().unwrap().name, "二");
        let _ = std::fs::remove_file(&path);
    }
}
