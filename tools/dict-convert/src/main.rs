//! 把数据源转换成云朵输入法的 TSV 格式，或打包成 `.qj`。
//!
//! - `lexicon`：云朵基础词库，「输入法字词库_分类整理版」数据包（规范字 / 常用词 / THUOCL 领域词）+ Unihan 读音 + 多音字词标注 → `dict.tsv`
//! - `english`：`词\t编码` 英文词表（数据包的 `05_english`，ESDB / CSpell，MIT）→ `english.tsv`
//! - `bigram`：纯文本语料（如 `tools/corpus/parquet_to_text.py` 转出的中文维基 CC BY-SA 4.0、LCCC 对话 MIT）→ `lm-unigram.tsv` + `lm-bigram.tsv`
//! - `mine`：语料里分词落成连续单字的段 → `oov-candidates.tsv`（词库没收的高频词，标音后用 `lexicon --extra-words` 并入）
//! - `phrases`：bigram 表的相邻两词 + 语料的相邻三词 → `phrases.tsv`（我的 / 不知道 这类短语层，读音由成分词拼出，同样用 `lexicon --extra-words` 并入）
//! - `pack dict|lm|model`：TSV → 词库存档 / `.qj` 容器（`dict` 写 SQLite `.db`，`lm` 写 `lm.qj`），带名称 / 许可证 / 署名元数据；
//!   `model` 把本地整句模型的三件套目录打成一个 `model.qjm`
//! - `word-bank`：合并中文 / 英文 / 品牌 / 中英混杂词源，写出 `WordBank\Dict.db`（引擎按 `language` 分流加载）
//! - `phrase-db`：写出空的自定义短语库 `Phrases\Phrase.db`（`user` / `cloudime_default` 两张空表）
//! - `rehead`：把改名前的 `.qj`（魔数 `QINGJIAN`）就地改成当前魔数 `CLOUDIME`，只改头 8 字节，见 `rehead.rs`
//!
//! 输出默认写到仓库根目录 `data/generated/`（gitignore）。

mod args;
mod bigram;
mod english;
mod error;
mod lexicon;
mod oov_filter;
mod pack;
mod phrases;
mod rehead;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::args::{Args, Command};
use crate::error::ConvertError;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ConvertError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let args = Args::parse();
    std::fs::create_dir_all(&args.out_dir)?;
    match args.command {
        Command::Lexicon {
            pack,
            unihan,
            pinyin,
            frequency,
            emit_ambiguous,
            extra_words,
            domain_keep_min,
        } => lexicon::convert(
            &pack,
            &unihan,
            pinyin.as_deref(),
            frequency.as_deref(),
            emit_ambiguous.as_deref(),
            &extra_words,
            domain_keep_min,
            &args.out_dir,
        ),
        Command::English { inputs, frequency } => english::convert(
            &inputs,
            frequency.as_deref(),
            &args.out_dir.join("english.tsv"),
        ),
        Command::Bigram {
            corpus,
            dict,
            phrases,
            brand,
            min_count,
            max_bigrams,
        } => bigram::convert(
            &corpus,
            &dict,
            &phrases,
            &brand,
            min_count,
            max_bigrams,
            &args.out_dir,
        ),
        Command::Mine {
            corpus,
            dict,
            min_count,
            max_chars,
            frequency,
            min_pmi,
            candidates,
        } => bigram::mine(
            &bigram::MineOptions {
                corpus,
                dict,
                min_count,
                max_chars,
                frequency,
                min_pmi,
                candidates,
            },
            &args.out_dir,
        ),
        Command::PhraseDb { path, force } => {
            // 随包那份的 cloudime_default 表是手写进去的产品数据，默认不覆盖
            if path.exists() && !force {
                return Err(ConvertError::PhraseDbExists { path });
            }
            cloudime_platform::phrase::create_empty(&path)?;
            tracing::info!(out = %path.display(), "已写出空短语库");
            Ok(())
        }
        Command::Phrases {
            corpus,
            dialogue,
            dict,
            refresh,
            min_count,
            max_chars,
        } => phrases::mine(
            &phrases::PhraseOptions {
                dict,
                refresh,
                corpus,
                dialogue,
                min_count,
                max_chars,
            },
            &args.out_dir,
        ),
        Command::Rehead { kind, input } => rehead::rehead(kind, &input),
        Command::Pack {
            kind,
            input,
            name,
            license,
            attribution,
            source,
            data_version,
        } => pack::pack(
            kind,
            &input,
            cloudime_format::Metadata {
                name,
                license,
                attribution,
                source,
                version: data_version,
                ..cloudime_format::Metadata::default()
            },
            &args.out_dir,
        ),
        Command::WordBank {
            chinese,
            english,
            extra,
            name,
            license,
            attribution,
            source,
            data_version,
        } => pack::word_bank(
            &chinese,
            &english,
            &extra,
            cloudime_format::Metadata {
                name,
                license,
                attribution,
                source,
                version: data_version,
                ..cloudime_format::Metadata::default()
            },
            &args.out_dir,
        ),
    }
}
