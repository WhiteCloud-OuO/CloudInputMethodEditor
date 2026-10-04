use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use cloudime_core::storage::{read_text_lossy, write_atomic_str};
use cloudime_core::{VocabularySummary, VocabularyTracker};
use jiff::civil::Date;

use crate::error::LearningError;

/// 「最近」算几天（含今天）。
const RECENT_DAYS: i64 = 7;

/// 词汇记录的落盘：一个词一行 `词\t看到轮次\t上屏次数\t首见\t末见`，只写在本机数据目录里。
///
/// 文件坏了按行跳过，读不了就只在内存里记（记一次警告），输入不受影响。
#[derive(Debug, Default)]
pub struct VocabularyBook {
    /// 词 → 记录，按键排好。
    entries: BTreeMap<String, Entry>,

    /// 自上次保存后有没有新记录。
    dirty: bool,

    /// [`VocabularyTracker::flush`] 时写回的路径；`None` 只在内存里记。
    path: Option<PathBuf>,
}

/// 一个词的记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// 上屏时在候选窗口里的轮次。
    seen: u32,

    /// 上屏过带它的候选几次。
    committed: u32,

    /// 第一次记录的日期。
    first: Date,

    /// 最近一次记录的日期。
    last: Date,
}

impl Entry {
    fn new(date: Date) -> Self {
        Self {
            seen: 0,
            committed: 0,
            first: date,
            last: date,
        }
    }
}

impl VocabularyBook {
    /// 从文件加载（不存在就从零开始，flush 时建）；读不了就退回只在内存里记。
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        match read_text_lossy(&path) {
            Ok(text) => Self {
                entries: text.as_deref().map(parse).unwrap_or_default(),
                dirty: false,
                path: Some(path),
            },
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "词汇记录读不了，本次只在内存里记");
                Self::default()
            }
        }
    }

    /// 记录的不同词数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry(&mut self, word: &str, date: Date) -> &mut Entry {
        self.dirty = true;
        let entry = self
            .entries
            .entry(word.to_owned())
            .or_insert_with(|| Entry::new(date));
        entry.last = date;
        entry
    }

    /// 记一次看到（[`VocabularyTracker::record_exposure`] 记在今天）。
    pub fn record_exposure_on(&mut self, word: &str, date: Date) {
        self.entry(word, date).seen += 1;
    }

    /// 记一次上屏（[`VocabularyTracker::record_commit`] 记在今天）。
    pub fn record_commit_on(&mut self, word: &str, date: Date) {
        self.entry(word, date).committed += 1;
    }

    /// 以 `today` 为准的汇总（[`VocabularyTracker::summary`] 以本机今天为准）。
    pub fn summary_on(&self, today: Date) -> VocabularySummary {
        let recent_from = today.saturating_sub(jiff::Span::new().days(RECENT_DAYS - 1));
        let mut summary = VocabularySummary::default();
        for entry in self.entries.values() {
            summary.seen += 1;
            summary.committed += u64::from(entry.committed > 0);
            summary.new_this_week += u64::from(entry.first >= recent_from && entry.first <= today);
        }
        summary
    }

    /// 写回文件（没有新记录就什么都不做）。
    pub fn save(&mut self) -> Result<(), LearningError> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        if !self.dirty {
            return Ok(());
        }
        self.save_to(&path)?;
        self.dirty = false;
        Ok(())
    }

    fn save_to(&self, path: &Path) -> Result<(), LearningError> {
        let mut text = String::from("# 词\t看到轮次\t上屏次数\t首见\t末见\n");
        for (word, entry) in &self.entries {
            let _ = writeln!(
                text,
                "{word}\t{}\t{}\t{}\t{}",
                entry.seen, entry.committed, entry.first, entry.last
            );
        }
        write_atomic_str(path, &text)?;
        Ok(())
    }
}

/// 按行解析；格式不对的行跳过（记警告），重复的键取后一条。
fn parse(text: &str) -> BTreeMap<String, Entry> {
    let mut entries = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Some((key, entry)) => {
                entries.insert(key, entry);
            }
            None => tracing::warn!(line = number + 1, "词汇记录有坏行，跳过"),
        }
    }
    entries
}

fn parse_line(line: &str) -> Option<(String, Entry)> {
    let mut fields = line.split('\t');
    let word = fields.next()?.to_owned();
    if word.is_empty() {
        return None;
    }
    let mut number = || fields.next()?.parse::<u32>().ok();
    let seen = number()?;
    let committed = number()?;
    let first: Date = fields.next()?.parse().ok()?;
    let last: Date = fields.next()?.parse().ok()?;
    Some((
        word,
        Entry {
            seen,
            committed,
            first,
            last,
        },
    ))
}

fn today() -> Date {
    jiff::Zoned::now().date()
}

impl VocabularyTracker for VocabularyBook {
    fn record_exposure(&mut self, word: &str) {
        self.record_exposure_on(word, today());
    }

    fn record_commit(&mut self, word: &str) {
        self.record_commit_on(word, today());
    }

    fn flush(&mut self) {
        if let Err(error) = self.save() {
            tracing::warn!(%error, "词汇记录保存失败");
        }
    }

    fn summary(&self) -> VocabularySummary {
        self.summary_on(today())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        text.parse().unwrap()
    }

    #[test]
    fn counts_exposures_and_summarizes() {
        let mut book = VocabularyBook::default();
        book.record_exposure_on("开发", date("2026-08-01"));
        book.record_exposure_on("开发", date("2026-08-01"));
        book.record_exposure_on("开放", date("2026-09-05"));
        book.record_commit_on("开放", date("2026-09-05"));
        book.record_commit_on("先", date("2026-09-06"));
        assert_eq!(book.len(), 3);
        let summary = book.summary_on(date("2026-09-06"));
        assert_eq!(
            summary,
            VocabularySummary {
                seen: 3,
                committed: 2,
                new_this_week: 2,
            }
        );
    }

    #[test]
    fn round_trips_through_the_file_and_skips_bad_lines() {
        let dir = std::env::temp_dir().join(format!("cloudime-vocab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("user-vocab.tsv");
        std::fs::write(
            &path,
            "# 头\n开发\t2\t1\t2026-09-01\t2026-09-05\n坏行\n\t1\t0\t2026-09-01\t2026-09-01\n",
        )
        .unwrap();
        let mut book = VocabularyBook::open(&path);
        assert_eq!(book.len(), 1);
        book.save().unwrap();
        book.record_exposure_on("开发", date("2026-09-06"));
        book.record_commit_on("世界", date("2026-09-06"));
        book.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# 词\t看到轮次\t上屏次数\t首见\t末见\n\
             世界\t0\t1\t2026-09-06\t2026-09-06\n\
             开发\t3\t1\t2026-09-01\t2026-09-06\n"
        );
        let reloaded = VocabularyBook::open(&path);
        assert_eq!(reloaded.summary_on(date("2026-09-06")).seen, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
