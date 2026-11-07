//! `cloudime-mcp-server`：通过 stdio 给外部 AI 客户端提供只读词库查询。
//!
//! stdout 只允许 MCP JSON-RPC；所有日志和启动错误走 stderr。

mod data;
mod server;
mod tools;

use std::error::Error;
use std::path::PathBuf;

use clap::Parser;
use data::LoadOptions;
use rmcp::ServiceExt;
use server::CloudimeServer;
use tracing_subscriber::EnvFilter;

/// 云朵输入法只读 MCP Server。
#[derive(Debug, Parser)]
#[command(version, about = "云朵输入法的只读 stdio MCP Server")]
struct Args {
    /// 覆盖主词库路径；没同时给 --word-bank 时不加载第三方词库。
    #[arg(long)]
    dict: Option<PathBuf>,

    /// 覆盖 WordBank 目录，目录下的第三方词库会一起加载。
    #[arg(long)]
    word_bank: Option<PathBuf>,

    /// 加载主词库的生僻字 / 生僻词组。
    #[arg(long)]
    include_rare: bool,

    /// 只查主词库，不加载第三方词库。
    #[arg(long)]
    no_extras: bool,
}

#[tokio::main]
async fn main() {
    init_logging();
    let args = Args::parse();
    if let Err(error) = run(args).await {
        eprintln!("cloudime-mcp-server: {error}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), Box<dyn Error>> {
    let loaded = data::load(&LoadOptions {
        dict: args.dict,
        word_bank: args.word_bank,
        include_rare: args.include_rare,
        no_extras: args.no_extras,
    })?;
    let server = CloudimeServer::new(loaded.engine, loaded.info);
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}
