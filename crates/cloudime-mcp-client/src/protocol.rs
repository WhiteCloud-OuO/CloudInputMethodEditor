//! Server 与 sidecar 的本地 IPC 消息。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一个代理请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// 请求号，由 Server 分配。
    pub id: u64,

    /// 请求内容。
    pub action: RequestAction,
}

/// 代理请求内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestAction {
    /// 列出配置里的 Server 名字。
    ListServers,

    /// 显式启动一个外部 MCP Server 的 session。
    StartServer { server: String },

    /// 关闭一个外部 MCP Server 的 session。
    StopServer { server: String },

    /// 关闭由 Lua 启动的 sidecar（Server 收到后结束子进程）。
    Shutdown,

    /// 列出某个 Server 的工具。
    ListTools { server: String },

    /// 调用一个工具。
    Call {
        server: String,
        tool: String,
        arguments: Value,
        timeout_ms: u64,
    },
}

/// 一个代理响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    /// 对应请求号。
    pub id: u64,

    /// 响应内容。
    pub body: ResponseBody,
}

/// 代理响应内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseBody {
    /// 配置里的 Server。
    Servers { servers: Vec<ServerSummary> },

    /// 某个 Server 已启动。
    Started {
        server: String,

        /// 这次启动是不是由 Server 拉起的 Lua 专属 sidecar。
        client_owned: bool,
    },

    /// 某个 Server 已停止。
    Stopped { server: String },

    /// sidecar 收到关闭请求。
    Shutdown,

    /// 某个 Server 的工具。
    Tools { tools: Vec<ToolSummary> },

    /// 工具调用结果。
    Call(CallOutcome),

    /// 请求失败。
    Error {
        error: String,

        /// 这个错误发生时，sidecar 是不是由这次 Lua 请求拉起的。
        #[serde(default)]
        client_owned: bool,
    },
}

/// 一个配置里的 Server。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSummary {
    /// 配置名。
    pub name: String,

    /// 允许调用的工具名。
    pub allowed_tools: Vec<String>,
}

/// 一个 MCP 工具。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSummary {
    /// 工具名。
    pub name: String,

    /// 工具说明。
    pub description: String,
}

/// 一次工具调用的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallOutcome {
    /// MCP content 数组，原样保留为 JSON。
    pub content: Vec<Value>,

    /// MCP structuredContent。
    pub structured: Option<Value>,

    /// MCP 是否把它标成错误。
    pub is_error: bool,

    /// 本地侧错误（超时、进程退出等）。
    pub error: Option<String>,
}
