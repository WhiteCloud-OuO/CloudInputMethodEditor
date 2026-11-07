//! 外部 MCP Server 的会话管理：懒启动、初始化、`tools/list` 与 `tools/call`。

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use cloudime_mcp_client::{
    CallOutcome, McpConfig, RequestAction, ResponseBody, ServerConfig, ServerSummary, ToolSummary,
};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tokio::sync::Mutex;

/// 单次工具结果的序列化上限；超过就当错误返回，避免把脚本或 Server 撑爆。
const MAX_RESULT_BYTES: usize = 256 * 1024;

type Session = Arc<Mutex<RunningService<RoleClient, ()>>>;

/// 配置里的所有 MCP Server；每个 Server 一个懒启动的 session。
pub struct Sessions {
    config: McpConfig,
    sessions: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// 用已校验的 `mcp.toml` 建会话表；不会在这里启动任何子进程。
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// 处理一条代理请求。
    pub async fn handle(&self, action: RequestAction) -> ResponseBody {
        match action {
            RequestAction::ListServers => ResponseBody::Servers {
                servers: self
                    .config
                    .servers
                    .iter()
                    .map(|server| ServerSummary {
                        name: server.name.clone(),
                        allowed_tools: server.allowed_tools.clone(),
                    })
                    .collect(),
            },
            RequestAction::StartServer { server } => self.start(&server).await,
            RequestAction::StopServer { server } => self.stop(&server).await,
            RequestAction::Shutdown => ResponseBody::Shutdown,
            RequestAction::ListTools { server } => self.list_tools(&server).await,
            RequestAction::Call {
                server,
                tool,
                arguments,
                timeout_ms,
            } => self.call(&server, &tool, arguments, timeout_ms).await,
        }
    }

    /// 关掉所有外部 Server 的 session，并给 rmcp 的子进程清理留一点时间。
    pub async fn shutdown_all(&self) {
        let sessions: Vec<Session> = self
            .sessions
            .lock()
            .await
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in sessions {
            let mut guard = session.lock().await;
            let _ = tokio::time::timeout(Duration::from_secs(1), guard.close()).await;
            drop(guard);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    async fn start(&self, name: &str) -> ResponseBody {
        let Some(server) = self.config.server(name).cloned() else {
            return not_configured(name);
        };
        if self.sessions.lock().await.contains_key(name) {
            return ResponseBody::Started {
                server: name.to_owned(),
                client_owned: false,
            };
        }
        match connect(&server).await {
            Ok(session) => {
                self.sessions
                    .lock()
                    .await
                    .insert(name.to_owned(), Arc::new(Mutex::new(session)));
                ResponseBody::Started {
                    server: name.to_owned(),
                    client_owned: false,
                }
            }
            Err(error) => failure(name, error),
        }
    }

    async fn stop(&self, name: &str) -> ResponseBody {
        self.sessions.lock().await.remove(name);
        ResponseBody::Stopped {
            server: name.to_owned(),
        }
    }

    async fn list_tools(&self, name: &str) -> ResponseBody {
        let Some(config) = self.config.server(name).cloned() else {
            return not_configured(name);
        };
        let session = match self.existing(name).await {
            Ok(session) => session,
            Err(error) => return failure(name, error),
        };
        let timeout = Duration::from_millis(config.timeout_ms.max(1));
        let result = {
            let guard = session.lock().await;
            tokio::time::timeout(timeout, guard.list_tools(None)).await
        };
        match result {
            Ok(Ok(tools)) => ResponseBody::Tools {
                tools: tools
                    .tools
                    .iter()
                    .map(|tool| ToolSummary {
                        name: tool.name.to_string(),
                        description: tool
                            .description
                            .as_ref()
                            .map(|description| description.to_string())
                            .unwrap_or_default(),
                    })
                    .collect(),
            },
            Ok(Err(error)) => {
                self.invalidate(name, &session).await;
                failure(name, format!("tools/list 失败：{error}"))
            }
            Err(_) => failure(name, "tools/list 超时".to_owned()),
        }
    }

    async fn call(
        &self,
        name: &str,
        tool: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> ResponseBody {
        let Some(server) = self.config.server(name).cloned() else {
            return not_configured(name);
        };
        if !server.allows(tool) {
            return failure(
                name,
                format!("工具 `{tool}` 不在 {name} 的 allowed_tools 里"),
            );
        }
        let arguments = match arguments {
            Value::Null => serde_json::Map::new(),
            Value::Object(arguments) => arguments,
            _ => return failure(name, "arguments 必须是 JSON 对象".to_owned()),
        };
        let session = match self.existing(name).await {
            Ok(session) => session,
            Err(error) => return failure(name, error),
        };
        let timeout = Duration::from_millis(timeout_ms.min(server.timeout_ms.max(1)).max(1));
        let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
        let result = {
            let guard = session.lock().await;
            tokio::time::timeout(timeout, guard.call_tool(params)).await
        };
        match result {
            Ok(Ok(result)) => match outcome(result) {
                Ok(outcome) => ResponseBody::Call(outcome),
                Err(error) => {
                    self.invalidate(name, &session).await;
                    failure(name, error)
                }
            },
            Ok(Err(error)) => {
                self.invalidate(name, &session).await;
                failure(name, format!("tools/call 失败：{error}"))
            }
            Err(_) => failure(name, "tools/call 超时".to_owned()),
        }
    }

    async fn existing(&self, name: &str) -> Result<Session, String> {
        self.sessions
            .lock()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| format!("MCP Server `{name}` 未启动，请先调用 cloudime.mcp.start"))
    }

    async fn invalidate(&self, name: &str, session: &Session) {
        let mut sessions = self.sessions.lock().await;
        if sessions
            .get(name)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            sessions.remove(name);
        }
    }
}

async fn connect(server: &ServerConfig) -> Result<RunningService<RoleClient, ()>, String> {
    let mut command = tokio::process::Command::new(&server.command);
    command.args(&server.args);
    for (key, value) in &server.env {
        command.env(key, value);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let (transport, stderr) = rmcp::transport::TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("启动 MCP Server `{}` 失败：{error}", server.name))?;
    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(mcp_server_stderr = %line, "外部 MCP Server 的 stderr");
            }
        });
    }
    let timeout = Duration::from_millis(server.timeout_ms.max(1));
    tokio::time::timeout(timeout, ().serve(transport))
        .await
        .map_err(|_| format!("MCP Server `{}` 初始化超时", server.name))?
        .map_err(|error| format!("MCP Server `{}` 初始化失败：{error}", server.name))
}

fn outcome(result: rmcp::model::CallToolResult) -> Result<CallOutcome, String> {
    let content = result
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<Value>, _>>()
        .map_err(|error| format!("工具结果无法序列化：{error}"))?;
    let outcome = CallOutcome {
        content,
        structured: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
        error: None,
    };
    let size = serde_json::to_vec(&outcome)
        .map_err(|error| format!("工具结果无法序列化：{error}"))?
        .len();
    if size > MAX_RESULT_BYTES {
        return Err(format!(
            "工具结果 {size} 字节，超过 {MAX_RESULT_BYTES} 字节上限"
        ));
    }
    Ok(outcome)
}

fn not_configured(name: &str) -> ResponseBody {
    failure(name, format!("没有配置名为 `{name}` 的 MCP Server"))
}

fn failure(name: &str, message: String) -> ResponseBody {
    tracing::warn!(server = name, %message, "MCP 代理请求失败");
    ResponseBody::Error {
        error: message,
        client_owned: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerConfig as RmcpServerConfig, Tool,
    };
    use rmcp::service::RequestContext;
    use rmcp::{RoleServer, ServerHandler, ServiceExt};

    #[derive(Clone)]
    struct Mock;

    impl ServerHandler for Mock {
        fn get_info(&self) -> RmcpServerConfig {
            let mut info =
                RmcpServerConfig::new(ServerCapabilities::builder().enable_tools().build());
            info.server_info = Implementation::new("mock", "0");
            info
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
            std::future::ready(Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "echo",
                "echo the arguments",
                serde_json::Map::new(),
            )])))
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<CallToolResponse, rmcp::ErrorData>> + Send + '_ {
            let arguments = request
                .arguments
                .map(|arguments| serde_json::Value::Object(arguments).to_string())
                .unwrap_or_default();
            std::future::ready(Ok(CallToolResult::success(vec![ContentBlock::text(
                format!("echo:{arguments}"),
            )])
            .into()))
        }
    }

    fn config() -> McpConfig {
        McpConfig {
            servers: vec![ServerConfig {
                name: "mock".to_owned(),
                command: "unused".to_owned(),
                args: Vec::new(),
                env: Default::default(),
                timeout_ms: 5_000,
                allowed_tools: vec!["echo".to_owned()],
            }],
        }
    }

    async fn sessions() -> (Sessions, tokio::task::JoinHandle<()>) {
        let (server_side, client_side) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let service = Mock.serve(server_side).await.expect("启动 mock MCP Server");
            let _ = service.waiting().await;
        });
        let client = ().serve(client_side).await.expect("初始化 mock MCP Client");
        let sessions = Sessions {
            config: config(),
            sessions: Mutex::new(HashMap::from([(
                "mock".to_owned(),
                Arc::new(Mutex::new(client)),
            )])),
        };
        (sessions, server)
    }

    #[tokio::test]
    async fn lists_and_calls_a_tool() {
        let (sessions, server) = sessions().await;

        match sessions.handle(RequestAction::ListServers).await {
            ResponseBody::Servers { servers } => {
                assert_eq!(servers.len(), 1);
                assert_eq!(servers[0].name, "mock");
                assert_eq!(servers[0].allowed_tools, ["echo"]);
            }
            body => panic!("应为 Servers：{body:?}"),
        }

        match sessions
            .handle(RequestAction::ListTools {
                server: "mock".to_owned(),
            })
            .await
        {
            ResponseBody::Tools { tools } => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "echo");
                assert_eq!(tools[0].description, "echo the arguments");
            }
            body => panic!("应为 Tools：{body:?}"),
        }

        match sessions
            .handle(RequestAction::Call {
                server: "mock".to_owned(),
                tool: "echo".to_owned(),
                arguments: serde_json::json!({ "text": "你好" }),
                timeout_ms: 2_000,
            })
            .await
        {
            ResponseBody::Call(outcome) => {
                assert!(!outcome.is_error);
                assert_eq!(outcome.content.len(), 1);
                let text = outcome.content[0]["text"].as_str().unwrap_or_default();
                assert!(text.contains("你好"), "{text}");
            }
            body => panic!("应为 Call：{body:?}"),
        }

        server.abort();
    }

    #[tokio::test]
    async fn rejects_unknown_servers_and_tools() {
        let (sessions, server) = sessions().await;

        match sessions
            .handle(RequestAction::ListTools {
                server: "missing".to_owned(),
            })
            .await
        {
            ResponseBody::Error { error, .. } => assert!(error.contains("missing"), "{error}"),
            body => panic!("应为 Error：{body:?}"),
        }

        match sessions
            .handle(RequestAction::Call {
                server: "mock".to_owned(),
                tool: "other".to_owned(),
                arguments: serde_json::Value::Null,
                timeout_ms: 2_000,
            })
            .await
        {
            ResponseBody::Error { error, .. } => {
                assert!(error.contains("allowed_tools"), "{error}")
            }
            body => panic!("应为 Error：{body:?}"),
        }

        server.abort();
    }

    #[tokio::test]
    async fn requires_an_explicit_start() {
        let sessions = Sessions::new(config());

        match sessions
            .handle(RequestAction::Call {
                server: "mock".to_owned(),
                tool: "echo".to_owned(),
                arguments: serde_json::Value::Null,
                timeout_ms: 2_000,
            })
            .await
        {
            ResponseBody::Error { error, .. } => assert!(error.contains("未启动"), "{error}"),
            body => panic!("应为 Error：{body:?}"),
        }

        match sessions
            .handle(RequestAction::ListTools {
                server: "mock".to_owned(),
            })
            .await
        {
            ResponseBody::Error { error, .. } => assert!(error.contains("未启动"), "{error}"),
            body => panic!("应为 Error：{body:?}"),
        }

        assert!(matches!(
            sessions
                .handle(RequestAction::StopServer {
                    server: "mock".to_owned(),
                })
                .await,
            ResponseBody::Stopped { .. }
        ));
        assert!(matches!(
            sessions.handle(RequestAction::Shutdown).await,
            ResponseBody::Shutdown
        ));
    }
}
