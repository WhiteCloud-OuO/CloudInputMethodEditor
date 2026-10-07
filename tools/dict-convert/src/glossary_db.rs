//! 释义表 `.qj` → 翻译 Tip 用的 `.db`：总表 `words` + 副表 `contents`
//! （结构见 `cloudime-translate` 的 `glossary/db.rs`）。
//!
//! 中文词与译词从释义表搬过来（一个中文词几条译词就写几条记录，顺序照原样）；拼音与首字母从词库查，
//! 查不到的留空——那两列是给以后按首字母 / 拼音检索用的，翻译 Tip 自己只按中文词查。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cloudime_dictionary::Dictionary;
use cloudime_translate::Glossary;
use rusqlite::{Connection, params};

use crate::error::ConvertError;

/// 转换。`input` 可给多个释义表，各写一份同名的 `.db`；`out` 给目录就写到那里（缺省与输入同目录）。
/// `word_bank` 是查拼音 / 首字母的词库（云朵 TSV / `.db` / `.qj` 都认），不给就只搬中文词与译词。
pub fn convert(
    input: &[PathBuf],
    word_bank: Option<&Path>,
    out: Option<&Path>,
) -> Result<(), ConvertError> {
    let pinyin = match word_bank {
        Some(path) => read_pinyin(path)?,
        None => HashMap::new(),
    };
    for source in input {
        let glossary = Glossary::open(source)?;
        let target = output_path(source, out);
        let written = write(&glossary, &pinyin, &target)?;
        // 写完自查：用读端重新打开，词条数与译词数对得上才算好
        let reopened = Glossary::open(&target)?;
        let entries = reopened.entries();
        let (words, senses) = (
            entries.len(),
            entries
                .iter()
                .map(|(_, senses)| senses.len())
                .sum::<usize>(),
        );
        if words != written.words || senses != written.senses {
            return Err(ConvertError::Verify(format!(
                "{}：写进去 {}/{}，读回来 {words}/{senses}",
                target.display(),
                written.words,
                written.senses
            )));
        }
        tracing::info!(
            out = %target.display(),
            words = written.words,
            senses = written.senses,
            indexed = written.indexed,
            filled = written.filled,
            "已写出本地词典 .db"
        );
    }
    Ok(())
}

/// 输出路径：`out` 是目录就放进去、否则当完整路径；缺省与输入同目录，主名不变、扩展名换成 `.db`。
fn output_path(source: &Path, out: Option<&Path>) -> PathBuf {
    let name = source.file_stem().map_or_else(
        || "dictionary.db".to_owned(),
        |stem| format!("{}.db", stem.to_string_lossy()),
    );
    match out {
        Some(dir) if dir.is_dir() => dir.join(name),
        Some(path) => path.to_path_buf(),
        None => source.with_file_name(name),
    }
}

/// 词库里的「词 → 拼音」。同一个词取先出现的那条（词库按权重排过）。
fn read_pinyin(path: &Path) -> Result<HashMap<String, String>, ConvertError> {
    let dictionary = Dictionary::from_path(path)?;
    let mut pinyin: HashMap<String, String> = HashMap::new();
    for entry in dictionary.entries() {
        pinyin
            .entry(entry.text.to_owned())
            .or_insert_with(|| entry.pinyin.to_owned());
    }
    tracing::info!(path = %path.display(), words = pinyin.len(), "词库已读入（查拼音用）");
    Ok(pinyin)
}

/// 写一份 `.db` 的数。
struct Written {
    /// 中文词条数。
    words: usize,

    /// 译词记录数。
    senses: usize,

    /// 副表登记数。
    indexed: usize,

    /// 其中查到了拼音（副表那两列有值）的条数。
    filled: usize,
}

/// 写一份 `.db`：先建两张表、灌进去，最后再建索引（比边插边维护快）。
fn write(
    glossary: &Glossary,
    pinyin: &HashMap<String, String>,
    target: &Path,
) -> Result<Written, ConvertError> {
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    let mut connection = Connection::open(target)?;
    connection.execute_batch(
        "PRAGMA journal_mode = OFF;
         CREATE TABLE words (
             id          INTEGER PRIMARY KEY,
             word        TEXT    NOT NULL,
             translation TEXT    NOT NULL,
             pos         TEXT,
             reading     TEXT
         );
         CREATE TABLE contents (
             id       INTEGER PRIMARY KEY,
             initials TEXT,
             pinyin   TEXT,
             word     TEXT NOT NULL,
             reading  TEXT
         );",
    )?;
    let mut written = Written {
        words: 0,
        senses: 0,
        indexed: 0,
        filled: 0,
    };
    let transaction = connection.transaction()?;
    {
        let mut insert_word = transaction.prepare(
            "INSERT INTO words (id, word, translation, pos, reading) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut insert_index = transaction.prepare(
            "INSERT INTO contents (id, initials, pinyin, word, reading) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (word, senses) in glossary.entries() {
            if senses.is_empty() {
                continue;
            }
            written.words += 1;
            for sense in &senses {
                written.senses += 1;
                insert_word.execute(params![
                    written.senses as i64,
                    word,
                    sense.text,
                    sense.pos,
                    sense.reading
                ])?;
            }
            written.indexed += 1;
            let reading = pinyin.get(&word);
            written.filled += usize::from(reading.is_some());
            // 副表这一列是**中文词**的读音：与拼音同值（副表自带一份，只查它也能拿到读音）
            insert_index.execute(params![
                written.indexed as i64,
                reading.and_then(|reading| first_letter(reading)),
                reading.map(String::as_str),
                word,
                reading.map(String::as_str)
            ])?;
        }
    }
    transaction.commit()?;
    connection.execute_batch(
        "CREATE INDEX words_by_word ON words (word);
         CREATE INDEX contents_by_word ON contents (word);",
    )?;
    Ok(written)
}

/// 拼音的首字母（`yun duo` → `y`）：取第一个拉丁字母、转小写。
fn first_letter(pinyin: &str) -> Option<String> {
    pinyin
        .chars()
        .find(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase().to_string())
}
