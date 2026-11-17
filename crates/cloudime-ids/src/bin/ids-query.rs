//! 命令行冒烟：`ids-query <IDS> [--data <ids.qj>]`。

use std::path::PathBuf;

use cloudime_ids::{IdsDatabase, IdsError, IdsMatch, ParseError, parse, source_label};

const USAGE: &str = "用法：ids-query <IDS> [--data <ids.qj>]\n例：ids-query \"⿰木木\"\n     ids-query LR木木 --data data/generated/ids.qj";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(arguments: Vec<String>) -> Result<(), CliError> {
    let mut query = None;
    let mut data = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--data" => {
                data = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
            }
            other if query.is_none() => query = Some(other.to_owned()),
            other => return Err(CliError::Unexpected(other.to_owned())),
        }
    }

    let query = query.ok_or(CliError::Usage)?;
    let path = data.unwrap_or_else(|| PathBuf::from("data/generated/ids.qj"));
    if !path.is_file() {
        return Err(CliError::Missing(path));
    }

    let database = IdsDatabase::open(&path)?;
    let node = parse(&query)?;
    if node.contains_wildcard() {
        let found = database.lookup_wildcard(&query, 200)?;
        println!(
            "命中 {} 条（最多显示 200 条，本次 {} 条）",
            found.total,
            found.matches.len()
        );
        for item in &found.matches {
            print_match(item);
        }
    } else {
        let matches = database.lookup_exact(&query)?;
        if matches.is_empty() {
            println!("没有找到：Unicode IDS 未收录这个结构");
        }
        for item in &matches {
            print_match(item);
        }
    }
    Ok(())
}

fn print_match(item: &IdsMatch) {
    let sources = source_label(item.sources);
    if sources.is_empty() {
        println!(
            "{}  U+{:04X}  {}",
            item.character, item.character as u32, item.ids
        );
    } else {
        println!(
            "{}  U+{:04X}  {}  {}",
            item.character, item.character as u32, item.ids, sources
        );
    }
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,

    #[error("多余的参数：{0}")]
    Unexpected(String),

    #[error("找不到 IDS 数据：{0}（先运行 ids-build 生成）")]
    Missing(PathBuf),

    #[error(transparent)]
    Ids(#[from] IdsError),

    #[error(transparent)]
    Parse(#[from] ParseError),
}
