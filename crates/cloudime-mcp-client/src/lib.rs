//! MCP Client 的 sidecar 共用协议、配置与本地命名管道。

mod config;
mod ipc;
mod protocol;

pub use config::{McpConfig, ServerConfig, load_config};
pub use ipc::{IPC_PIPE, read_message, write_message};
pub use protocol::{
    CallOutcome, McpRequest, McpResponse, RequestAction, ResponseBody, ServerSummary, ToolSummary,
};
