//! `cloudime-mcp-client`：连接外部 MCP Server 的 sidecar，经命名管道接受 `cloudime-server` 的代理请求。
//!
//! 进程由 Server 在脚本显式 `cloudime.mcp.start` 时拉起；管道断开（Server 退出或崩溃）就退出。
//! stdout 不承载协议，日志全部走 stderr。

mod sessions;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use cloudime_mcp_client::{
    IPC_PIPE, McpRequest, McpResponse, RequestAction, ResponseBody, load_config, write_message,
};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::{mpsc, oneshot};
use tracing_subscriber::EnvFilter;

/// MCP Client sidecar。
#[derive(Debug, Parser)]
#[command(version, about = "云朵输入法的 MCP Client sidecar")]
struct Args {
    /// Server 与 sidecar 之间的命名管道。
    #[arg(long, default_value = IPC_PIPE)]
    pipe: String,

    /// `mcp.toml` 路径。
    #[arg(long)]
    config: PathBuf,
}

/// 一条要写给 Server 的响应；`ack` 用来确认「已经写出去」再退出。
struct Outgoing {
    response: McpResponse,
    ack: Option<oneshot::Sender<()>>,
}

#[tokio::main]
async fn main() {
    init_logging();
    let args = Args::parse();
    if let Err(error) = run(args).await {
        eprintln!("cloudime-mcp-client: {error}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config(&args.config)?;
    let sessions = std::sync::Arc::new(sessions::Sessions::new(config));

    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&args.pipe)?;
    server.connect().await?;
    let (mut reader, mut writer) = tokio::io::split(server);

    let (responses_tx, mut responses_rx) = mpsc::unbounded_channel::<Outgoing>();
    let writer_task = tokio::spawn(async move {
        while let Some(outgoing) = responses_rx.recv().await {
            let mut frame = Vec::new();
            if write_message(&mut frame, &outgoing.response).is_err() {
                break;
            }
            if writer.write_all(&frame).await.is_err() || writer.flush().await.is_err() {
                break;
            }
            if let Some(ack) = outgoing.ack {
                let _ = ack.send(());
            }
        }
    });

    let mut handlers = tokio::task::JoinSet::new();
    loop {
        let request: McpRequest = match read_frame(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "读取 MCP 代理请求失败");
                break;
            }
        };
        if matches!(&request.action, RequestAction::Shutdown) {
            // 先停掉在跑的调用，再关外部 MCP Server 的 session（连带子进程），确认响应写出去再退出。
            handlers.abort_all();
            while handlers.join_next().await.is_some() {}
            sessions.shutdown_all().await;
            let (ack, written) = oneshot::channel();
            let _ = responses_tx.send(Outgoing {
                response: McpResponse {
                    id: request.id,
                    body: ResponseBody::Shutdown,
                },
                ack: Some(ack),
            });
            let _ = written.await;
            break;
        }
        let sessions = sessions.clone();
        let responses = responses_tx.clone();
        handlers.spawn(async move {
            let body = sessions.handle(request.action).await;
            let _ = responses.send(Outgoing {
                response: McpResponse {
                    id: request.id,
                    body,
                },
                ack: None,
            });
        });
    }

    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    drop(responses_tx);
    let _ = writer_task.await;
    drop(sessions);
    // 给 rmcp 的子进程清理任务一点时间，别把外部 MCP Server 留成孤儿。
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(())
}

async fn read_frame<T: DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> io::Result<Option<T>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > 8 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MCP IPC frame too large",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}
