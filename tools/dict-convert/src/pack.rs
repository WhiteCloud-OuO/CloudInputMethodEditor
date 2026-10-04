//! TSV → 词库存档（SQLite `.db`）：解析成内存结构后落盘，加上元数据；`model` 是三件套目录 → `.qjm`。
//! `word_bank` 把中文 / 英文 / 品牌 / 混杂词源合并成一份 `Dict.db`。

use std::path::{Path, PathBuf};
use std::time::Instant;

use cloudime_dictionary::{DictDb, Dictionary, WORD_BANK_FILE, WordList};
use cloudime_format::Metadata;
use cloudime_lm::BigramModel;

use crate::args::DataKind;
use crate::error::ConvertError;

/// 打包一种数据。`inputs` 为空时从 `out_dir` 里找缺省的 TSV。
pub fn pack(
    kind: DataKind,
    inputs: &[PathBuf],
    metadata: Metadata,
    out_dir: &Path,
) -> Result<(), ConvertError> {
    let metadata = Metadata {
        generator: format!("cloudime-dict-convert {}", env!("CARGO_PKG_VERSION")),
        ..metadata
    };
    if metadata.name.is_empty() {
        return Err(ConvertError::MissingName { kind: kind.name() });
    }
    let started = Instant::now();
    match kind {
        DataKind::Dict => {
            let input = inputs
                .first()
                .cloned()
                .unwrap_or_else(|| out_dir.join("dict.tsv"));
            // 输出名跟着输入文件走：`dict.tsv` → `dict.db`，`law.qj` → `law.db`
            let stem = input
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("dict");
            let dictionary = Dictionary::from_path(&input)?;
            let out = out_dir.join(format!("{stem}.db"));
            dictionary.write_db(&out, &metadata)?;
            report(&out, dictionary.len(), started);
        }
        DataKind::Lm => {
            let (unigram, bigram) = match inputs {
                [unigram, bigram, ..] => (unigram.clone(), bigram.clone()),
                _ => (
                    out_dir.join("lm-unigram.tsv"),
                    out_dir.join("lm-bigram.tsv"),
                ),
            };
            let model = BigramModel::from_paths(&unigram, &bigram)?;
            let out = out_dir.join("lm.qj");
            model.write_qj(&out, &metadata)?;
            report(&out, model.bigram_count(), started);
        }
        DataKind::Model => {
            let input = inputs
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("data/local_models"));
            let out = out_dir.join("model.qjm");
            let parameters = cloudime_neural::qjm::pack(&input, &out, &metadata)?;
            report(
                &out,
                usize::try_from(parameters).unwrap_or(usize::MAX),
                started,
            );
        }
    }
    Ok(())
}

/// 把中文 / 英文 / 品牌 / 中英混杂词源合并成一份 `WordBank\Dict.db`。
///
/// 中文与英文各拼一份 TSV 解析：中文源是 `词\t拼音\t词频`，品牌 / 混杂源是 `词\t次数\t拼音`（列序不同，转一下）；
/// 英文源是 `词\t编码\t词频`。同一 `(词, 拼音, 语言)` 先去重（权重大的胜出、相同则先到先得），
/// 再按 `dictionary/db.rs` 的判据归进 7 张表。
pub fn word_bank(
    chinese: &[PathBuf],
    english: &[PathBuf],
    extra: &[PathBuf],
    metadata: Metadata,
    out_dir: &Path,
) -> Result<(), ConvertError> {
    let metadata = Metadata {
        generator: format!("cloudime-dict-convert {}", env!("CARGO_PKG_VERSION")),
        ..metadata
    };
    if metadata.name.is_empty() {
        return Err(ConvertError::MissingName { kind: "word-bank" });
    }
    let started = Instant::now();
    let chinese_sources = if chinese.is_empty() {
        vec![PathBuf::from("data/generated/dict.tsv")]
    } else {
        chinese.to_vec()
    };
    let extra_sources = if extra.is_empty() {
        default_extras()
    } else {
        extra.to_vec()
    };
    let english_sources = if english.is_empty() {
        [PathBuf::from("data/generated/english.tsv")]
            .into_iter()
            .chain([PathBuf::from("assets/lexicon/english.tsv")])
            .filter(|path| path.is_file())
            .take(1)
            .collect::<Vec<_>>()
    } else {
        english.to_vec()
    };

    let mut tsv = String::new();
    for path in &chinese_sources {
        append_chinese(&mut tsv, path)?;
    }
    for path in &extra_sources {
        append_extra(&mut tsv, path)?;
    }
    let dictionary = Dictionary::parse(&tsv)?;

    let mut english_text = String::new();
    for path in &english_sources {
        let text = std::fs::read_to_string(path)?;
        english_text.push_str(&text);
        if !text.ends_with('\n') {
            english_text.push('\n');
        }
    }
    let english = if english_text.is_empty() {
        WordList::default()
    } else {
        WordList::parse(&english_text)?
    };

    let db = DictDb {
        chinese: dictionary,
        rare: Dictionary::default(),
        english,
        metadata: None,
    };
    let out = out_dir.join(WORD_BANK_FILE);
    db.write(&out, &metadata)?;
    report(&out, db.len(), started);
    Ok(())
}

/// 缺省带的品牌词与中英混杂词源（存在才带）。
fn default_extras() -> Vec<PathBuf> {
    ["assets/lexicon/brand.tsv", "assets/lexicon/mixed_words.tsv"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect()
}

/// 中文源：TSV 直接拼；`.db` / `.qj` 先读出来再把词目拼成 `词\t拼音\t词频`。
fn append_chinese(tsv: &mut String, path: &Path) -> Result<(), ConvertError> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("db") | Some("qj") => {
            let dictionary = Dictionary::from_path(path)?;
            for entry in dictionary.entries() {
                append_row(tsv, entry.text, entry.pinyin, entry.frequency);
            }
        }
        _ => {
            let text = std::fs::read_to_string(path)?;
            tsv.push_str(&text);
            if !text.ends_with('\n') {
                tsv.push('\n');
            }
        }
    }
    Ok(())
}

/// 品牌 / 混杂词源：`词\t次数\t拼音` → 云朵词库的 `词\t拼音\t词频`。
fn append_extra(tsv: &mut String, path: &Path) -> Result<(), ConvertError> {
    for (index, raw) in std::fs::read_to_string(path)?.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(word), Some(count), Some(pinyin)) = (fields.next(), fields.next(), fields.next())
        else {
            tracing::warn!(file = %path.display(), line = index + 1, "品牌 / 混杂词行格式不对，跳过");
            continue;
        };
        let frequency: u32 = count.trim().parse().unwrap_or(1);
        append_row(tsv, word.trim(), pinyin.trim(), frequency);
    }
    Ok(())
}

fn append_row(tsv: &mut String, text: &str, pinyin: &str, frequency: u32) {
    tsv.push_str(text);
    tsv.push('\t');
    tsv.push_str(pinyin);
    tsv.push('\t');
    tsv.push_str(&frequency.to_string());
    tsv.push('\n');
}

fn report(out: &Path, entries: usize, started: Instant) {
    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        out = %out.display(),
        entries,
        size_mb = size / 1_000_000,
        elapsed_ms = started.elapsed().as_millis(),
        "已打包"
    );
}
