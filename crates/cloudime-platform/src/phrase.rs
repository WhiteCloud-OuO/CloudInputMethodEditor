//! 用户短语库：数据目录下的 SQLite 文件（缺省 `Phrase.db`）。
//!
//! 表：`phrases`（输入码 + 原样上屏的文本 + 固定候选位置）+ `meta`（存档版本）。config.toml 只记文件位置；
//! 「设置 → 短语」页读写这个文件，Server 启动与热加载时读进引擎。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cloudime_core::CustomPhrase;
use cloudime_core::custom_phrase::{self, MAX_POSITION};
use rusqlite::{Connection, OpenFlags, params};

use crate::PhraseConfig;
use crate::config::DEFAULT_PHRASE_FILE;

/// SQLite 文件头。
const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// 存档结构；版本写在 `meta` 里。
const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE phrases (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, position INTEGER NOT NULL);
";

/// 存档格式版本。
const FORMAT: &str = "2";

/// 短语库的读写错误。
#[derive(Debug, thiserror::Error)]
pub enum PhraseError {
    /// SQLite 报的错（坏文件、表结构不对等）。
    #[error("短语库读写失败：{0}")]
    Database(#[from] rusqlite::Error),

    /// 文件系统层面的错。
    #[error("短语库文件读写失败：{0}")]
    Io(#[from] std::io::Error),

    /// 短语内容不合法（见 [`custom_phrase::validate_phrases`]）。
    #[error("{0}")]
    Invalid(String),
}

/// 短语库文件的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhraseStore {
    /// 短语库文件的完整路径。
    pub path: PathBuf,
}

impl PhraseStore {
    /// 按配置与数据目录定位。`file` 空着用缺省文件名；相对路径按数据目录解析。
    pub fn locate(data_dir: &Path, config: &PhraseConfig) -> Self {
        let configured = config.file.trim();
        let file = if configured.is_empty() {
            DEFAULT_PHRASE_FILE
        } else {
            configured
        };
        let path = Path::new(file);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            data_dir.join(path)
        };
        Self { path }
    }

    /// 文件在不在。
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    /// 读全部短语（按写入顺序）。文件不在返回空表。
    ///
    /// 旧版把位置这一列叫 `weight`（1–999999，越大越靠前）：按同一输入码内的名次换算成 0–9 的位置，
    /// 下次保存后就是新结构。
    pub fn load(&self) -> Result<Vec<CustomPhrase>, PhraseError> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let columns = table_columns(&connection, "phrases")?;
        if columns.iter().any(|column| column == "position") {
            let mut statement =
                connection.prepare("SELECT code, text, position FROM phrases ORDER BY id")?;
            let rows = statement.query_map([], |row| {
                Ok(CustomPhrase {
                    code: row.get(0)?,
                    text: row.get(1)?,
                    position: row.get(2)?,
                })
            })?;
            return Ok(rows.collect::<Result<Vec<_>, _>>()?);
        }
        if columns.iter().any(|column| column == "weight") {
            let mut statement =
                connection.prepare("SELECT code, text, weight FROM phrases ORDER BY id")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            })?;
            let raw: Vec<(String, String, u32)> = rows.collect::<Result<Vec<_>, _>>()?;
            return Ok(legacy_positions(raw));
        }
        Ok(Vec::new())
    }

    /// 覆盖写全部短语（先校验，再写临时文件改名）。目录不存在时建出来。
    pub fn save(&self, phrases: &[CustomPhrase]) -> Result<(), PhraseError> {
        custom_phrase::validate_phrases(phrases).map_err(PhraseError::Invalid)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = temporary_path(&self.path);
        let result = (|| -> Result<(), PhraseError> {
            {
                let mut connection = Connection::open(&temporary)?;
                connection.execute_batch(SCHEMA)?;
                let transaction = connection.transaction()?;
                {
                    let mut statement = transaction.prepare(
                        "INSERT INTO phrases (code, text, position) VALUES (?1, ?2, ?3)",
                    )?;
                    for phrase in phrases {
                        statement.execute(params![phrase.code, phrase.text, phrase.position])?;
                    }
                }
                {
                    let mut statement =
                        transaction.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
                    statement.execute(params!["format", FORMAT])?;
                    statement.execute(params!["count", phrases.len().to_string()])?;
                }
                transaction.commit()?;
            }
            std::fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

/// 这个文件是不是 SQLite 短语库（看文件头，不看扩展名）。
pub fn is_database(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 16];
    file.read_exact(&mut head).is_ok() && &head == MAGIC
}

/// 表的列名（`PRAGMA table_info`），用来认旧版把位置列叫 `weight` 的文件。
fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, PhraseError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 旧版短语库（`weight` 列）换算成位置：同一输入码内权重大的排前，名次就是位置，超过上限的停在
/// [`MAX_POSITION`]。同权重按写入顺序，保证结果稳定。
fn legacy_positions(raw: Vec<(String, String, u32)>) -> Vec<CustomPhrase> {
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| raw[b].2.cmp(&raw[a].2).then_with(|| a.cmp(&b)));
    let mut next: HashMap<String, u32> = HashMap::new();
    let mut positions = vec![0u32; raw.len()];
    for index in order {
        let rank = next.entry(raw[index].0.clone()).or_default();
        positions[index] = (*rank).min(MAX_POSITION);
        *rank += 1;
    }
    raw.into_iter()
        .zip(positions)
        .map(|((code, text, _), position)| CustomPhrase {
            code,
            text,
            position,
        })
        .collect()
}

/// 与目标同目录的临时文件：改名不能跨文件系统，带进程号避免两个进程互相覆盖。
fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> (PathBuf, PhraseStore) {
        let dir =
            std::env::temp_dir().join(format!("cloudime-phrase-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = PhraseStore::locate(&dir, &PhraseConfig::default());
        (dir, store)
    }

    fn phrase(code: &str, text: &str, position: u32) -> CustomPhrase {
        CustomPhrase {
            code: code.into(),
            text: text.into(),
            position,
        }
    }

    #[test]
    fn file_path_follows_the_config() {
        let dir = Path::new("C:/Users/me/AppData/Roaming/CloudIME");
        let default = PhraseStore::locate(dir, &PhraseConfig::default());
        assert_eq!(default.path, dir.join("Phrase.db"));

        let custom = PhraseStore::locate(
            dir,
            &PhraseConfig {
                file: "sub/phrases.db".to_owned(),
            },
        );
        assert_eq!(custom.path, dir.join("sub/phrases.db"));
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let (dir, store) = scratch("missing");
        assert!(!store.exists());
        assert!(store.load().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips_in_order() {
        let (dir, store) = scratch("roundtrip");
        let phrases = vec![
            phrase("ee", "；", 1),
            phrase("omw", "On my way!", 9),
            phrase("xh", "😀", 0),
        ];
        store.save(&phrases).unwrap();
        assert!(store.exists());
        assert_eq!(store.load().unwrap(), phrases);
        // 覆盖写：旧内容不再存在
        store.save(&[phrase("aa", "，", 3)]).unwrap();
        assert_eq!(store.load().unwrap(), [phrase("aa", "，", 3)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_weight_column_maps_to_positions() {
        let (dir, store) = scratch("legacy");
        let connection = Connection::open(&store.path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE phrases (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, weight INTEGER NOT NULL);
                 INSERT INTO phrases (code, text, weight) VALUES ('ee', '；', 5), ('ee', '：', 9), ('ee', '。', 1), ('aa', '，', 7);",
            )
            .unwrap();
        drop(connection);
        // 同一输入码内权重大的排前，名次就是位置；新码各从 0 起
        assert_eq!(
            store.load().unwrap(),
            [
                phrase("ee", "；", 1),
                phrase("ee", "：", 0),
                phrase("ee", "。", 2),
                phrase("aa", "，", 0),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_phrases_are_refused_before_writing() {
        let (dir, store) = scratch("invalid");
        let error = store.save(&[phrase("AA", "大写", 1)]).unwrap_err();
        assert!(matches!(error, PhraseError::Invalid(_)));
        assert!(!store.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
