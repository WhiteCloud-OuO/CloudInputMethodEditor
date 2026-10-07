//! 词条的学习状态：用户目录的 `translate.db`（SQLite）里一张表，键是「词典文件名 + 词条」。
//!
//! 词典文件本身只读（`.qj` 是 mmap），所以「上屏过几次、学会没有」记在这里：
//! 释义 Tip 每上屏一次 `input_times` 加 1，到 `need_times` 就把 `learned` 置上，
//! 设置页里那个 `candidate_translate_tip_color` 的颜色就跟着它变。
//! 「重置学习内容」= 把这个词典的词条全删掉（缺省就是 0 次、没学会）。

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::TranslateError;

/// 一个词条的学习状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    /// 释义上屏过几次。
    pub input_times: u32,

    /// 学会了没有（`input_times` 到过 `need_times` 就是）。
    pub learned: bool,
}

/// 学习状态库。
pub struct Learning {
    connection: Connection,

    /// 内存里的一份，免得每帧查库；`load` 时从库里重读。
    entries: HashMap<(String, String), Entry>,
}

impl Learning {
    /// 打开（不存在就建出表）。
    pub fn open(path: &Path) -> Result<Self, TranslateError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                 dictionary  TEXT    NOT NULL,
                 word        TEXT    NOT NULL,
                 input_times INTEGER NOT NULL DEFAULT 0,
                 learned     INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (dictionary, word)
             );",
        )?;
        let mut learning = Self {
            connection,
            entries: HashMap::new(),
        };
        learning.load()?;
        Ok(learning)
    }

    /// 从库里全读一遍（外部改过库时用）。
    pub fn load(&mut self) -> Result<(), TranslateError> {
        let mut statement = self
            .connection
            .prepare("SELECT dictionary, word, input_times, learned FROM entries")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                Entry {
                    input_times: row.get(2)?,
                    learned: row.get(3)?,
                },
            ))
        })?;
        self.entries.clear();
        for row in rows {
            let (dictionary, word, entry) = row?;
            self.entries.insert((dictionary, word), entry);
        }
        Ok(())
    }

    /// 一个词条的状态；没记过就是缺省（0 次、没学会）。
    pub fn entry(&self, dictionary: &str, word: &str) -> Entry {
        self.entries
            .get(&(dictionary.to_owned(), word.to_owned()))
            .copied()
            .unwrap_or_default()
    }

    /// 这个词典里「学过的条数 / 记过的条数」（设置页显示用）。
    pub fn counts(&self, dictionary: &str) -> (usize, usize) {
        let mut learned = 0;
        let mut total = 0;
        for ((name, _), entry) in &self.entries {
            if name != dictionary {
                continue;
            }
            total += 1;
            learned += usize::from(entry.learned);
        }
        (learned, total)
    }

    /// 释义上屏一次：`input_times` 加 1；到了 `need_times` 就把 `learned` 置上。返回新状态。
    pub fn record_commit(
        &mut self,
        dictionary: &str,
        word: &str,
        need_times: u32,
    ) -> Result<Entry, TranslateError> {
        let current = self.entry(dictionary, word);
        let input_times = current.input_times.saturating_add(1);
        let entry = Entry {
            input_times,
            learned: current.learned || input_times >= need_times.max(1),
        };
        self.connection.execute(
            "INSERT INTO entries (dictionary, word, input_times, learned) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(dictionary, word) DO UPDATE SET input_times = ?3, learned = ?4",
            params![dictionary, word, entry.input_times, entry.learned],
        )?;
        self.entries
            .insert((dictionary.to_owned(), word.to_owned()), entry);
        Ok(entry)
    }

    /// 「重置学习内容」：把这个词典记过的词条全删掉（等于都回到 0 次、没学会）。返回删了几条。
    pub fn reset(&mut self, dictionary: &str) -> Result<usize, TranslateError> {
        let removed = self
            .connection
            .execute("DELETE FROM entries WHERE dictionary = ?1", [dictionary])?;
        self.entries
            .retain(|(name, _), _| name.as_str() != dictionary);
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temporary() -> (Learning, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("cloudime-translate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("learning-{}.db", unique_suffix()));
        let learning = Learning::open(&path).unwrap();
        (learning, path)
    }

    /// 每个测试用不同的文件名，免得并行跑时互相踩（目录名里已经带了进程号）。
    fn unique_suffix() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn counts_commits_until_learned() {
        let (mut learning, path) = open_temporary();
        assert_eq!(learning.entry("en.qj", "悲伤的"), Entry::default());
        for times in 1..=2 {
            let entry = learning.record_commit("en.qj", "悲伤的", 3).unwrap();
            assert_eq!(entry.input_times, times);
            assert!(!entry.learned);
        }
        let entry = learning.record_commit("en.qj", "悲伤的", 3).unwrap();
        assert_eq!((entry.input_times, entry.learned), (3, true));
        // 另一个词典的词条各记各的
        assert_eq!(learning.entry("ja.qj", "悲伤的").input_times, 0);
        assert_eq!(learning.counts("en.qj"), (1, 1));
        // 重开一遍也在（已经落盘）
        let reopened = Learning::open(&path).unwrap();
        assert!(reopened.entry("en.qj", "悲伤的").learned);
        // 连接还开着时 Windows 删不掉这个文件，先放掉
        drop(reopened);
        drop(learning);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reset_forgets_only_that_dictionary() {
        let (mut learning, path) = open_temporary();
        learning.record_commit("en.qj", "sad", 3).unwrap();
        learning.record_commit("ja.qj", "sad", 3).unwrap();
        assert_eq!(learning.reset("en.qj").unwrap(), 1);
        assert_eq!(learning.entry("en.qj", "sad"), Entry::default());
        assert_eq!(learning.entry("ja.qj", "sad").input_times, 1);
        drop(learning);
        std::fs::remove_file(&path).unwrap();
    }
}
