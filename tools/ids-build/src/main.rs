//! 生成 IDS 组字数据：`IDS.TXT` → `ids.qj`。
//!
//! 输入是 BabelStone 的 IDS.TXT（Unicode 16.0，97,680 字），每行
//! `U+XXXX <TAB> 字 <TAB> ^IDS$(来源)…`；同一个字可以有多条来源 IDS。
//! 输出按「规范化 IDS」聚合：一个 IDS 对应多个字、一个字对应多个 IDS 都保留；
//! `components.tsv` 里的难输入部件另外写成 `COMP` 分节。

mod args;
mod classify;
mod components;
mod derive;
mod read;
mod report;
mod stroke;
mod write;

use std::collections::HashSet;
use std::process::ExitCode;

use clap::Parser;

use crate::args::Args;
use crate::components::ComponentRow;
use crate::stroke::Unihan;

fn main() -> ExitCode {
    let args = Args::parse();
    let result = if args.derive {
        run_derive(&args)
    } else {
        run_build(&args)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run_build(args: &Args) -> Result<(), BuildError> {
    let rows = read_rows(args)?;
    let typeable = load_typeable(args)?;
    let resolved = load_components(args, &rows, &typeable)?;
    let metadata = cloudime_format::Metadata {
        name: "BabelStone IDS (Unicode 16.0)".to_owned(),
        license: "Public Domain".to_owned(),
        attribution: "Andrew West (BabelStone); based on data from Kawabata Taichi, \
                      John Knightley and IRG submissions"
            .to_owned(),
        source: "https://babelstone.co.uk/CJK/IDS.TXT".to_owned(),
        version: "2025-06-27".to_owned(),
        entries: 0,
        generator: format!("ids-build {}", env!("CARGO_PKG_VERSION")),
    };
    let stats = write::write(
        &args.out,
        &rows,
        &resolved.components,
        &resolved.component_text,
        metadata,
    )?;
    println!(
        "写出 {}：{} 个 IDS 键，{} 个字，{} 条记录，{} 个部件",
        args.out.display(),
        stats.keys,
        stats.characters,
        stats.entries,
        stats.components
    );
    if let Some(path) = &args.report {
        report::write(path, &resolved.report)?;
        let low_confidence = resolved
            .report
            .iter()
            .filter(|row| {
                row.auto_primary
                    .is_some_and(|primary| row.roles[primary as usize] < 300)
            })
            .count();
        let unknown_stroke = resolved.report.iter().filter(|row| row.stroke == 0).count();
        println!(
            "报告 {}：{} 行，低置信 {}，笔画未知 {}",
            path.display(),
            resolved.report.len(),
            low_confidence,
            unknown_stroke
        );
    }
    Ok(())
}

fn run_derive(args: &Args) -> Result<(), BuildError> {
    let rows = read_rows(args)?;
    let typeable = load_typeable(args)?;
    let component_rows = load_component_rows(args)?;
    let existing: HashSet<char> = component_rows.iter().map(|row| row.character).collect();
    let unihan = load_unihan(args)?;
    let classifications = classify::classify(&rows);
    let candidates = derive::derive(
        &rows,
        &existing,
        &typeable,
        &unihan,
        &classifications,
        args.derive_min,
        args.derive_include_typeable,
    );
    derive::write(&args.derive_out, &candidates)?;
    println!(
        "候选 {}：{} 条（出现字数 ≥ {}，已收录 {} 个部件）",
        args.derive_out.display(),
        candidates.len(),
        args.derive_min,
        existing.len()
    );
    Ok(())
}

fn read_rows(args: &Args) -> Result<read::Rows, BuildError> {
    let source = std::fs::read_to_string(&args.ids).map_err(|source| BuildError::Read {
        path: args.ids.clone(),
        source,
    })?;
    read::parse(&source)
}

/// 读 `components.tsv`，按 TSV → Unihan → IDS 递归补笔画，用位置统计补分类。
fn load_components(
    args: &Args,
    ids: &read::Rows,
    typeable: &HashSet<char>,
) -> Result<components::Resolved, BuildError> {
    let rows = load_component_rows(args)?;
    if rows.is_empty() {
        eprintln!(
            "提示：{} 不在或没有启用的行，跳过部件表（只生成 IDS 查询）",
            args.components.display()
        );
        return Ok(components::Resolved::default());
    }
    let unihan = load_unihan(args)?;
    let classifications = classify::classify(ids);
    let glyphs = load_glyphs(args)?;
    Ok(components::resolve(
        &rows,
        ids,
        &unihan,
        &classifications,
        typeable,
        &glyphs,
    ))
}

fn load_glyphs(args: &Args) -> Result<std::collections::HashMap<char, String>, BuildError> {
    if !args.glyphs.is_file() {
        return Ok(std::collections::HashMap::new());
    }
    let source = std::fs::read_to_string(&args.glyphs).map_err(|source| BuildError::Read {
        path: args.glyphs.clone(),
        source,
    })?;
    let mut glyphs = std::collections::HashMap::new();
    for (index, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((character, path)) = line.split_once('\t') else {
            return Err(BuildError::BadLine {
                line: index + 1,
                reason: "字形行要 `字符 <TAB> path`".to_owned(),
            });
        };
        let mut characters = character.chars();
        let Some(character) = characters.next() else {
            return Err(BuildError::BadLine {
                line: index + 1,
                reason: "字形行缺字符".to_owned(),
            });
        };
        if characters.next().is_some() {
            return Err(BuildError::BadLine {
                line: index + 1,
                reason: "字形行第一列不是单个字符".to_owned(),
            });
        }
        glyphs.insert(character, path.trim().to_owned());
    }
    Ok(glyphs)
}

fn load_component_rows(args: &Args) -> Result<Vec<ComponentRow>, BuildError> {
    if !args.components.is_file() {
        return Ok(Vec::new());
    }
    let source = std::fs::read_to_string(&args.components).map_err(|source| BuildError::Read {
        path: args.components.clone(),
        source,
    })?;
    components::parse(&source)
}

fn load_unihan(args: &Args) -> Result<Unihan, BuildError> {
    match &args.unihan {
        Some(path) => Unihan::load(path),
        None => Ok(Unihan::default()),
    }
}

fn load_typeable(args: &Args) -> Result<HashSet<char>, BuildError> {
    match &args.dict {
        Some(path) => derive::typeable_from_dict(path),
        None => Ok(HashSet::new()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("读取 {path} 失败：{source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("创建输出目录 {path} 失败：{source}")]
    CreateDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("写 {path} 失败：{source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("第 {line} 行格式不对：{reason}")]
    BadLine { line: usize, reason: String },

    #[error("第 {line} 行的 IDS 解析失败：{source}")]
    Parse {
        line: usize,
        source: cloudime_ids::ParseError,
    },

    #[error(transparent)]
    Format(#[from] cloudime_format::FormatError),

    #[error("IDS 数据里没有任何条目")]
    Empty,

    #[error("数据太大，偏移或长度放不进 u32 / u16")]
    TooLarge,
}
