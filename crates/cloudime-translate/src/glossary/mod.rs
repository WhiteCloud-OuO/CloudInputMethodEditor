//! 一个本地词典文件：按扩展名分 `.qj`（mmap 零拷贝）与 `.db`（SQLite）。

mod db;
mod qj;

use std::path::{Path, PathBuf};

use crate::error::TranslateError;
use crate::model::Sense;

/// 打开着的词典。
pub struct Glossary {
    /// 文件路径（学习状态按文件名归属）。
    path: PathBuf,

    /// 词条与释义。
    entries: Entries,
}

enum Entries {
    /// `.qj`：映射进来的 arena、词条表、释义表与哈希索引。
    Qj(qj::Qj),

    /// `.db`：只读连着的 SQLite。
    Db(db::Db),
}

impl Glossary {
    /// 按扩展名打开：`.qj` 走容器映射，`.db` 走 SQLite，别的报 [`TranslateError::Unsupported`]。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, TranslateError> {
        let path = path.into();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase);
        let entries = match extension.as_deref() {
            Some("qj") => Entries::Qj(qj::Qj::open(&path)?),
            Some("db") => Entries::Db(db::Db::open(&path)?),
            _ => return Err(TranslateError::Unsupported(path.display().to_string())),
        };
        tracing::debug!(path = %path.display(), entries = entries.len(), "本地词典已打开");
        Ok(Self { path, entries })
    }

    /// 词典文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 词典文件名（`glossary-en.qj`）：学习状态按它归属。
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// 条数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 查一个词条的释义；词典里没有返回 `None`。
    pub fn lookup(&self, word: &str) -> Option<Vec<Sense>> {
        self.entries.lookup(word)
    }

    /// 全部词条（中文词 + 它的释义），按词典里的顺序。转换工具与统计用。
    pub fn entries(&self) -> Vec<(String, Vec<Sense>)> {
        self.entries.entries()
    }
}

impl Entries {
    fn len(&self) -> usize {
        match self {
            Self::Qj(qj) => qj.len(),
            Self::Db(db) => db.len(),
        }
    }

    fn lookup(&self, word: &str) -> Option<Vec<Sense>> {
        match self {
            Self::Qj(qj) => qj.lookup(word),
            Self::Db(db) => db.lookup(word),
        }
    }

    fn entries(&self) -> Vec<(String, Vec<Sense>)> {
        match self {
            Self::Qj(qj) => qj.entries(),
            Self::Db(db) => db.entries(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::Glossary;

    /// 一次基准里命中 / 未命中各查多少次。
    const LOOKUPS: usize = 20_000;

    /// 仓库根 `LocalDictionary\<name>`；不存在就跳过。可用环境变量指到别处（比如转出来的 `.db`）。
    fn candidate(name: &str, env: &str) -> Option<PathBuf> {
        if let Ok(path) = std::env::var(env) {
            let path = PathBuf::from(path);
            return path.is_file().then_some(path);
        }
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../LocalDictionary")
            .join(name);
        path.is_file().then_some(path)
    }

    /// 查 `words` 里的每一个词，返回总耗时与查到的次数。
    fn time(glossary: &Glossary, words: &[&str]) -> (Duration, usize) {
        let started = Instant::now();
        let mut hits = 0;
        for word in words {
            hits += usize::from(glossary.lookup(word).is_some());
        }
        (started.elapsed(), hits)
    }

    /// 带 `reading` 列的 `.db`（转出来的日语词典）真的带上了读音；文件不在就跳过。
    #[test]
    fn the_converted_dictionary_keeps_readings() {
        let Some(path) = candidate("glossary-ja.db", "CLOUDIME_TRANSLATE_DB_JA") else {
            eprintln!("跳过： LocalDictionary/glossary-ja.db 不在");
            return;
        };
        let glossary = Glossary::open(&path).unwrap();
        let with_reading = glossary
            .entries()
            .into_iter()
            .flat_map(|(_, senses)| senses)
            .filter(|sense| sense.reading.is_some())
            .count();
        assert!(with_reading > 10_000, "带读音的译词只有 {with_reading} 条");
        println!("{path:?}: 带读音的译词 {with_reading} 条");
    }

    /// `.qj` 与 `.db` 两种本地词典的查词开销对比（手工跑，release）：
    ///
    /// ```text
    /// cargo test -p cloudime-translate --release -- --ignored --nocapture lookup_latency
    /// $env:CLOUDIME_TRANSLATE_DB = '<转出来的 glossary-en.db>'
    /// ```
    #[test]
    #[ignore = "手工跑的基准：.qj 与 .db 的查词开销"]
    fn lookup_latency() {
        for (label, path) in [
            ("qj", candidate("glossary-en.qj", "CLOUDIME_TRANSLATE_QJ")),
            ("db", candidate("glossary-en.db", "CLOUDIME_TRANSLATE_DB")),
        ] {
            let Some(path) = path else {
                println!("{label}: 跳过（{label} 词典文件不在）");
                continue;
            };
            let started = Instant::now();
            let glossary = Glossary::open(&path).unwrap();
            let open = started.elapsed();
            let words: Vec<String> = glossary
                .entries()
                .into_iter()
                .map(|(word, _)| word)
                .collect();
            // 均匀铺开取命中样本，别都挤在文件开头（缓存不真实）
            let step = (words.len() / LOOKUPS).max(1);
            let hits: Vec<&str> = words
                .iter()
                .step_by(step)
                .take(LOOKUPS)
                .map(String::as_str)
                .collect();
            let misses: Vec<String> = (0..LOOKUPS).map(|i| format!("没这个词{i}")).collect();
            let miss_refs: Vec<&str> = misses.iter().map(String::as_str).collect();
            // 预热：让 mmap / sqlite 的页与语句缓存先就位
            time(&glossary, &hits[..LOOKUPS.min(1_000)]);
            time(&glossary, &miss_refs[..LOOKUPS.min(1_000)]);
            let (hit, found) = time(&glossary, &hits);
            let (miss, extra) = time(&glossary, &miss_refs);
            let micros = |elapsed: Duration| elapsed.as_secs_f64() * 1e6 / LOOKUPS as f64;
            println!(
                "{label}: 打开 {:.1} ms · 词条 {} · 命中 {:.2} µs/次（查到 {found}）· 未命中 {:.2} µs/次（误中 {extra}）",
                open.as_secs_f64() * 1e3,
                words.len(),
                micros(hit),
                micros(miss),
            );
        }
    }
}
