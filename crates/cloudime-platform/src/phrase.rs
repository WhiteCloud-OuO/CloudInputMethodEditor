//! 短语库：安装目录下 `Phrases\Phrase.db` 的 SQLite 文件。
//!
//! 两张结构相同的表：`user`（用户自己的短语，「设置 → 短语」页读写）与 `cloudime_default`（软件自带短语，
//! 随安装包带一份空库、内容以后填）。字段 `id / code / text / title / position`（`title` 可空，位置 1–9）。
//! Server 启动与热加载时经 [`PhraseStore::load`] 读进引擎；设置页用 [`PhraseStore::save_user`] 只覆盖 `user`。
//! 旧版数据目录（`%APPDATA%\CloudIME\Phrase.db`）的单表 `phrases` 由迁移读一次（[`read_legacy`]）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cloudime_core::CustomPhrase;
use cloudime_core::custom_phrase::{self, MAX_POSITION, MIN_POSITION};
use rusqlite::{Connection, OpenFlags, params};

/// SQLite 文件头。
const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// 短语库目录名（安装目录下）。
pub const PHRASES_DIR: &str = "Phrases";

/// 短语库文件名。
pub const PHRASE_FILE: &str = "Phrase.db";

/// 随包的内置短语同步源（相对安装根）：安装包每次升级都覆盖它，Server 启动时用
/// [`PhraseStore::sync_defaults`] 把它的 `cloudime_default` 同步进 `Phrases\Phrase.db`（`user` 不动）。
pub const DEFAULT_SOURCE_FILE: &str = "data/phrase-default.db";

/// 用户短语表。
const USER_TABLE: &str = "user";

/// 软件自带短语表。
const DEFAULT_TABLE: &str = "cloudime_default";

/// 旧版单表的表名（迁移时读老文件用）。
const LEGACY_TABLE: &str = "phrases";

/// 两张结构相同的表：`title` 可空，`position` 1–9。
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS user (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, title TEXT, position INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS cloudime_default (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, title TEXT, position INTEGER NOT NULL);
";

/// 连接上的忙碌等待，避免与 Server 的并发读撞成 `SQLITE_BUSY`。
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// 固定定位：安装根（随包根）下的 `Phrases\Phrase.db`。
    pub fn locate(root: &Path) -> Self {
        Self {
            path: root.join(PHRASES_DIR).join(PHRASE_FILE),
        }
    }

    /// 文件在不在。
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    /// 读短语：先 `user`（按 `id`），`use_default` 为真时再读 `cloudime_default`，
    /// 丢掉 `code` 已出现在 `user` 里的那些（同一输入码用户的那份完整顶掉内置的）。
    /// 文件或表不存在按空。
    pub fn load(&self, use_default: bool) -> Result<Vec<CustomPhrase>, PhraseError> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let mut phrases = if table_exists(&connection, USER_TABLE)? {
            read_table(&connection, USER_TABLE)?
        } else {
            Vec::new()
        };
        if use_default && table_exists(&connection, DEFAULT_TABLE)? {
            let user_codes: std::collections::HashSet<String> =
                phrases.iter().map(|phrase| phrase.code.clone()).collect();
            for phrase in read_table(&connection, DEFAULT_TABLE)? {
                if !user_codes.contains(&phrase.code) {
                    phrases.push(phrase);
                }
            }
        }
        custom_phrase::normalize_phrases(&mut phrases);
        Ok(phrases)
    }

    /// 只覆盖 `user` 表（`cloudime_default` 原样保留）。设置页保存用户短语走这里。
    pub fn save_user(&self, phrases: &[CustomPhrase]) -> Result<(), PhraseError> {
        let mut phrases = phrases.to_vec();
        custom_phrase::normalize_phrases(&mut phrases);
        custom_phrase::validate_phrases(&phrases).map_err(PhraseError::Invalid)?;
        self.save_table(USER_TABLE, &phrases)
    }

    /// 只覆盖 `cloudime_default` 表（`user` 原样保留）。
    pub fn save_default(&self, phrases: &[CustomPhrase]) -> Result<(), PhraseError> {
        let mut phrases = phrases.to_vec();
        custom_phrase::normalize_phrases(&mut phrases);
        custom_phrase::validate_phrases(&phrases).map_err(PhraseError::Invalid)?;
        self.save_table(DEFAULT_TABLE, &phrases)
    }

    /// 按随包的同步源（`{安装根}\data\phrase-default.db` 的 `cloudime_default` 表）刷新内置短语：
    /// 与当前不同才整表替换（`user` 不动）并返回 `true`；相同、或源文件不在，返回 `false`。
    ///
    /// 安装包每次升级都覆盖源文件、却用 `onlyifdoesntexist` 保住 `Phrases\Phrase.db`（里面有用户短语），
    /// 内置短语的更新就靠这里在 Server 启动时补上。
    pub fn sync_defaults(&self, source: &Path) -> Result<bool, PhraseError> {
        if !source.is_file() {
            return Ok(false);
        }
        let mut incoming = read_file_table(source, DEFAULT_TABLE)?;
        custom_phrase::normalize_phrases(&mut incoming);
        if self.read_default()? == incoming {
            return Ok(false);
        }
        self.save_default(&incoming)?;
        Ok(true)
    }

    /// 当前 `cloudime_default` 表的内容（没有文件 / 表就是空）。
    fn read_default(&self) -> Result<Vec<CustomPhrase>, PhraseError> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let mut phrases = read_file_table(&self.path, DEFAULT_TABLE)?;
        custom_phrase::normalize_phrases(&mut phrases);
        Ok(phrases)
    }

    /// 只覆盖 `table` 那一张表：先把现有文件拷一份到同目录临时文件（没有就新建全套 schema），
    /// 在临时文件里整表重写它，再原子改名回原路径；另一张表原样保留。目录不存在时建出来。
    /// 并发读靠连接的 busy timeout 与改名前的完整拷贝兜住。
    fn save_table(&self, table: &str, phrases: &[CustomPhrase]) -> Result<(), PhraseError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = temporary_path(&self.path);
        let result = (|| -> Result<(), PhraseError> {
            // 从现有文件起步，保住另一张表；文件不在就从头建。
            if self.path.is_file() {
                std::fs::copy(&self.path, &temporary)?;
            }
            {
                let mut connection = Connection::open(&temporary)?;
                connection.busy_timeout(BUSY_TIMEOUT)?;
                connection.execute_batch(SCHEMA)?;
                let transaction = connection.transaction()?;
                transaction.execute(&format!("DELETE FROM {table}"), [])?;
                {
                    let mut statement = transaction.prepare(&format!(
                        "INSERT INTO {table} (code, text, title, position) VALUES (?1, ?2, ?3, ?4)"
                    ))?;
                    for phrase in phrases {
                        statement.execute(params![
                            phrase.code,
                            phrase.text,
                            phrase.title,
                            phrase.position
                        ])?;
                    }
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

/// 写出两张空表（`user` / `cloudime_default`）的短语库，给生成工具与安装包用：先写临时文件再改名，
/// 已存在的文件被整份替换成空库。
pub fn create_empty(path: &Path) -> Result<(), PhraseError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = temporary_path(path);
    let result = (|| -> Result<(), PhraseError> {
        let _ = std::fs::remove_file(&temporary);
        {
            let connection = Connection::open(&temporary)?;
            connection.busy_timeout(BUSY_TIMEOUT)?;
            connection.execute_batch(SCHEMA)?;
        }
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// 读旧版数据目录里的 `Phrase.db`（单表 `phrases`）：位置列可能叫 `position`（旧 0 基）或 `weight`（1–999999），
/// 都换算成 1–9 的位置。给迁移用，`title` 一律 `None`。
pub fn read_legacy(path: &Path) -> Result<Vec<CustomPhrase>, PhraseError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    if !table_exists(&connection, LEGACY_TABLE)? {
        return Ok(Vec::new());
    }
    let columns = table_columns(&connection, LEGACY_TABLE)?;
    if columns.iter().any(|column| column == "position") {
        let mut statement =
            connection.prepare("SELECT code, text, position FROM phrases ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let raw: Vec<(String, String, i64)> = rows.collect::<Result<Vec<_>, _>>()?;
        return Ok(raw
            .into_iter()
            .map(|(code, text, position)| CustomPhrase {
                code,
                text,
                title: None,
                position: shift_position(position),
            })
            .collect());
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
        return Ok(legacy_weights(raw));
    }
    Ok(Vec::new())
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

/// 打开一个短语库文件读它的一张表（文件不在 / 表不在按空）。
fn read_file_table(path: &Path, table: &str) -> Result<Vec<CustomPhrase>, PhraseError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    if !table_exists(&connection, table)? {
        return Ok(Vec::new());
    }
    read_table(&connection, table)
}

/// 读一张表（两张结构相同）。
fn read_table(connection: &Connection, table: &str) -> Result<Vec<CustomPhrase>, PhraseError> {
    let mut statement = connection.prepare(&format!(
        "SELECT code, text, title, position FROM {table} ORDER BY id"
    ))?;
    let rows = statement.query_map([], |row| {
        Ok(CustomPhrase {
            code: row.get(0)?,
            text: row.get(1)?,
            title: row.get(2)?,
            position: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 表在不在。
fn table_exists(connection: &Connection, table: &str) -> Result<bool, PhraseError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// 表的列名（`PRAGMA table_info`），用来认旧版把位置列叫 `weight` 的文件。
fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, PhraseError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 旧版 `position` 列（0 基）换算成 1 基，夹到范围里。
fn shift_position(position: i64) -> u32 {
    (position + 1).clamp(i64::from(MIN_POSITION), i64::from(MAX_POSITION)) as u32
}

/// 旧版 `weight` 列（1–999999，越大越靠前）：同一输入码内权重大的排前，名次换算成 1 基的位置，
/// 超过上限的停在 [`MAX_POSITION`]。同权重按写入顺序，保证结果稳定。
fn legacy_weights(raw: Vec<(String, String, u32)>) -> Vec<CustomPhrase> {
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| raw[b].2.cmp(&raw[a].2).then_with(|| a.cmp(&b)));
    let mut next: HashMap<String, u32> = HashMap::new();
    let mut positions = vec![MIN_POSITION; raw.len()];
    for index in order {
        let rank = next.entry(raw[index].0.clone()).or_default();
        positions[index] = (*rank + MIN_POSITION).min(MAX_POSITION);
        *rank += 1;
    }
    raw.into_iter()
        .zip(positions)
        .map(|((code, text, _), position)| CustomPhrase {
            code,
            text,
            title: None,
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

    fn scratch(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cloudime-phrase-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn phrase(code: &str, text: &str, position: u32) -> CustomPhrase {
        CustomPhrase {
            code: code.into(),
            text: text.into(),
            title: None,
            position,
        }
    }

    fn titled(code: &str, text: &str, title: Option<&str>, position: u32) -> CustomPhrase {
        CustomPhrase {
            code: code.into(),
            text: text.into(),
            title: title.map(str::to_owned),
            position,
        }
    }

    #[test]
    fn locate_uses_the_install_directory() {
        let store = PhraseStore::locate(Path::new("C:/CloudIME"));
        assert_eq!(store.path, Path::new("C:/CloudIME/Phrases/Phrase.db"));
    }

    /// 内置短语同步：源里换了内容就整表替换（`user` 不动），一样 / 源不在就不动。
    #[test]
    fn default_phrases_sync_from_the_bundled_source() {
        let dir = scratch("sync-defaults");
        let store = PhraseStore::locate(&dir);
        store.save_user(&[phrase("wo", "我", 1)]).unwrap();
        store.save_default(&[phrase("zuo", "←", 5)]).unwrap();
        // 同步源：内置内容变了（多一条、带 title）
        let source = dir.join(DEFAULT_SOURCE_FILE);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        create_empty(&source).unwrap();
        PhraseStore {
            path: source.clone(),
        }
        .save_default(&[phrase("zuo", "←", 5), titled("you", "→", Some("右箭头"), 5)])
        .unwrap();

        assert!(store.sync_defaults(&source).unwrap());
        assert_eq!(
            store.load(true).unwrap(),
            [
                phrase("wo", "我", 1),
                phrase("zuo", "←", 5),
                titled("you", "→", Some("右箭头"), 5),
            ]
        );
        // 再同步一次：内容一样，不动
        assert!(!store.sync_defaults(&source).unwrap());
        // 源不在也不动
        assert!(!store.sync_defaults(&dir.join("nope.db")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = scratch("missing");
        let store = PhraseStore::locate(&dir);
        assert!(!store.exists());
        assert!(store.load(true).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips_in_order() {
        let dir = scratch("roundtrip");
        let store = PhraseStore::locate(&dir);
        let phrases = vec![
            titled("ee", "；", Some("分号"), 1),
            phrase("omw", "On my way!", 9),
            phrase("xh", "😀", 2),
        ];
        store.save_user(&phrases).unwrap();
        assert!(store.exists());
        assert_eq!(store.load(false).unwrap(), phrases);
        // 覆盖写：旧内容不再存在
        store.save_user(&[phrase("aa", "，", 3)]).unwrap();
        assert_eq!(store.load(false).unwrap(), [phrase("aa", "，", 3)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_phrases_are_opt_in_and_user_code_replaces_them() {
        let dir = scratch("default");
        let store = PhraseStore::locate(&dir);
        create_empty(&store.path).unwrap();
        {
            let connection = Connection::open(&store.path).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO cloudime_default (code, text, title, position) VALUES
                     ('dz', '地址', NULL, 2),
                     ('yx', '邮箱', NULL, 1);
                     INSERT INTO user (code, text, title, position) VALUES
                     ('yx', '我的邮箱', NULL, 1);",
                )
                .unwrap();
        }
        // 关掉自带短语：只有用户短语
        assert_eq!(store.load(false).unwrap(), [phrase("yx", "我的邮箱", 1)]);
        // 开着：自带的 `dz` 补进来，`yx` 被用户那份顶掉
        let loaded = store.load(true).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&phrase("dz", "地址", 2)));
        assert!(loaded.contains(&phrase("yx", "我的邮箱", 1)));
        assert!(!loaded.contains(&phrase("yx", "邮箱", 1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_user_leaves_cloudime_default_alone() {
        let dir = scratch("keep-default");
        let store = PhraseStore::locate(&dir);
        create_empty(&store.path).unwrap();
        {
            let connection = Connection::open(&store.path).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO cloudime_default (code, text, title, position) VALUES ('dz', '地址', NULL, 2);",
                )
                .unwrap();
        }
        store.save_user(&[phrase("aa", "，", 1)]).unwrap();
        let loaded = store.load(true).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&phrase("dz", "地址", 2)));
        assert!(loaded.contains(&phrase("aa", "，", 1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_weight_column_maps_to_one_based_positions() {
        let dir = scratch("legacy-weight");
        let path = dir.join("Phrase.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE phrases (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, weight INTEGER NOT NULL);
                 INSERT INTO phrases (code, text, weight) VALUES ('ee', '；', 5), ('ee', '：', 9), ('ee', '。', 1), ('aa', '，', 7);",
            )
            .unwrap();
        drop(connection);
        // 同一输入码内权重大的排前，名次换算成 1 基；新码从 1 起
        assert_eq!(
            read_legacy(&path).unwrap(),
            [
                phrase("ee", "；", 2),
                phrase("ee", "：", 1),
                phrase("ee", "。", 3),
                phrase("aa", "，", 1),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_position_column_shifts_to_one_base_and_clamps() {
        let dir = scratch("legacy-position");
        let path = dir.join("Phrase.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE phrases (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, position INTEGER NOT NULL);
                 INSERT INTO phrases (code, text, position) VALUES ('aa', '甲', 0), ('bb', '乙', 8), ('cc', '丙', 9);",
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            read_legacy(&path).unwrap(),
            [
                phrase("aa", "甲", 1),
                phrase("bb", "乙", 9),
                phrase("cc", "丙", 9),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_phrases_are_refused_before_writing() {
        let dir = scratch("invalid");
        let store = PhraseStore::locate(&dir);
        let error = store.save_user(&[phrase("AA", "大写", 1)]).unwrap_err();
        assert!(matches!(error, PhraseError::Invalid(_)));
        assert!(!store.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
