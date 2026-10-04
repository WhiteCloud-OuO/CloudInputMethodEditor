//! 用户自造词库：SQLite 文件（缺省 `UserWordBank.db`）。
//!
//! 自动造出的用户词连权重一起存这里；`FrequencyLearner` 把它和 `user-words.tsv` 合成一张用户词库给引擎查。
//! 位置由壳决定（Server 按 `[word_bank] user_file` 解析，缺省在安装目录的 `WordBank\` 下；CLI 仍与词频文件同目录）。
//! 表 `words`（词 + 语言 + 全拼 + 初始权重 + 重选次数）：当前权重 = 初始权重 × 增长比例^重选次数（撤销时退回一次）。
//! 语言分两种：`中文` 按拼音音节查（进用户词库）、`英文` 整串大小写不敏感地匹配（进个人英文词表）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::error::LearningError;

/// 缺省文件名（learner 缺省与词频文件同目录时才用）。
pub const DEFAULT_FILE: &str = "UserWordBank.db";

/// 自造词每多选一次涨的权重比例。
pub const USER_WORD_GROWTH: f64 = 1.2;

/// 自造词权重上限，防止无限增长。
pub const MAX_USER_WEIGHT: f64 = 1.0e9;

/// 存档结构。
const SCHEMA: &str = "CREATE TABLE words (text TEXT PRIMARY KEY, language TEXT NOT NULL, pinyin TEXT NOT NULL, base REAL NOT NULL, repeats INTEGER NOT NULL);";

/// 一条自造词的语言：决定它按拼音音节查还是整串（大小写不敏感）匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordLanguage {
    /// 拼音串（自动造词 / 整句收录）：进用户词库，按拼音音节查。
    Chinese,

    /// Ctrl + 回车记下的原样字母：进个人英文词表，整串大小写不敏感地匹配。
    English,
}

impl WordLanguage {
    /// 存档里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "中文",
            Self::English => "英文",
        }
    }

    /// 从存档里的写法读回来；认不出的按中文处理。
    fn parse(value: &str) -> Self {
        match value {
            "英文" => Self::English,
            _ => Self::Chinese,
        }
    }
}

/// 一条自造词。
#[derive(Debug, Clone, PartialEq)]
pub struct BankWord {
    /// 语言。
    pub language: WordLanguage,

    /// 全拼（空格分隔）；英文词为空。
    pub pinyin: String,

    /// 初始权重（录入时各字词库词频的最大值 / 英文固定 1000）。
    pub base: f64,

    /// 录入后又被选过几次；每次权重 ×[`USER_WORD_GROWTH`]，撤销退回一次。
    pub repeats: u32,
}

impl BankWord {
    /// 新录一条；`base` 是初始权重。
    pub fn new(language: WordLanguage, pinyin: String, base: f64) -> Self {
        Self {
            language,
            pinyin,
            base: base.clamp(1.0, MAX_USER_WEIGHT),
            repeats: 0,
        }
    }

    /// 当前权重：初始权重 × 增长比例^重选次数，封顶。
    pub fn weight(&self) -> f64 {
        let exponent = i32::try_from(self.repeats).unwrap_or(i32::MAX);
        (self.base * USER_WORD_GROWTH.powi(exponent)).min(MAX_USER_WEIGHT)
    }

    /// 又多选了一次。
    pub fn grow(&mut self) {
        self.repeats = self.repeats.saturating_add(1);
    }

    /// 撤销一次重选，权重退回一档。
    pub fn shrink(&mut self) {
        self.repeats = self.repeats.saturating_sub(1);
    }
}

/// 读一份自造词库；文件不在返回空表。旧版（没有 `language` 列）一律当中文。
pub fn load(path: &Path) -> Result<BTreeMap<String, BankWord>, LearningError> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let connection = Connection::open(path)?;
    let legacy = !has_language(&connection)?;
    let query = if legacy {
        "SELECT text, pinyin, base, repeats FROM words"
    } else {
        "SELECT text, language, pinyin, base, repeats FROM words"
    };
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map([], |row| {
        let (language, pinyin, base, repeats) = if legacy {
            (
                WordLanguage::Chinese,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?.max(0) as u32,
            )
        } else {
            (
                WordLanguage::parse(&row.get::<_, String>(1)?),
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?.max(0) as u32,
            )
        };
        Ok((
            row.get::<_, String>(0)?,
            BankWord {
                language,
                pinyin,
                base,
                repeats,
            },
        ))
    })?;
    let mut words = BTreeMap::new();
    for row in rows {
        let (text, word) = row?;
        // 坏行跳过：空词没有意义；中文词必须有全拼
        if text.is_empty()
            || (word.language == WordLanguage::Chinese && word.pinyin.trim().is_empty())
        {
            continue;
        }
        words.insert(text, word);
    }
    Ok(words)
}

/// 覆盖写一份自造词库（先建临时文件再改名）。目录不存在时建出来。
pub fn save(path: &Path, words: &BTreeMap<String, BankWord>) -> Result<(), LearningError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = temporary_path(path);
    let result = (|| -> Result<(), LearningError> {
        {
            let mut connection = Connection::open(&temporary)?;
            connection.execute_batch(SCHEMA)?;
            let transaction = connection.transaction()?;
            {
                let mut statement = transaction.prepare(
                    "INSERT INTO words (text, language, pinyin, base, repeats) VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for (text, word) in words {
                    statement.execute(params![
                        text,
                        word.language.as_str(),
                        word.pinyin,
                        word.base,
                        i64::from(word.repeats)
                    ])?;
                }
            }
            transaction.commit()?;
        }
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// 表里有没有 `language` 列（旧版没有）。
fn has_language(connection: &Connection) -> Result<bool, LearningError> {
    let mut statement = connection.prepare("PRAGMA table_info(words)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let names: Vec<String> = rows.collect::<Result<_, _>>()?;
    Ok(names.iter().any(|name| name == "language"))
}

/// 与目标同目录的临时文件：改名不能跨文件系统，带进程号避免两个进程互相覆盖。
fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}
