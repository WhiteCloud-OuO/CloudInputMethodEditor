//! cwt —— 云朵词库转换工具：把第三方词库（`yaml` / `tsv` / `dat`）转成云朵的 `.db` 词库。
//!
//! ```text
//! cwt <输入文件> <输出.db> [--ignore_head] [--exclude_below NUM] [--exclude_lang chinese|english] [--remove_source 0|1]
//! ```
//!
//! 输入一律按 UTF-8 文本读：`.yaml` / `.yml` 按 Rime 词典（先跳过 `---` 开头的 YAML 头），
//! `.tsv` / `.dat` 及其它扩展名按制表符文本（`词<TAB>拼音或编码<TAB>权重`）。
//! 输出是单张 `words(text, pinyin, language, weight)` 表（外加 `meta`），与引擎里
//! `DictDb::from_path` 的 `format 2` 分支一致，可以直接丢进 `WordBank\`。
//!
//! 每个文件都是 UTF-8。

mod cli;
mod error;
mod source;
mod write;

use clap::Parser;

use crate::cli::Args;
use crate::error::Error;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let args = Args::parse();
    let entries = source::load(&args)?;
    let chinese = entries
        .iter()
        .filter(|entry| entry.language == source::Language::Chinese)
        .count();
    let name = args
        .input
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    write::write(&args.output, &entries, &name)?;
    tracing::info!(
        entries = entries.len(),
        chinese,
        english = entries.len() - chinese,
        out = %args.output.display(),
        "已写出词库"
    );
    if args.remove_source != 0 {
        std::fs::remove_file(&args.input).map_err(|source| Error::Delete {
            path: args.input.clone(),
            source,
        })?;
        tracing::info!(path = %args.input.display(), "已删除源文件");
    }
    Ok(())
}
