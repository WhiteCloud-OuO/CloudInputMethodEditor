# cloudime-mcp-client：独立 sidecar + Lua 异步调用

2026-10-28 定，同日实现。MCP Client 不嵌进 `cloudime-server` 的输入工人线程，新增独立 sidecar；
Lua 仍使用 `cloudime.mcp.*`，由 Server 通过本地命名管道代理。

> 实现状态：`crates/cloudime-mcp-client`、`apps/mcp-client`、`apps/windows/server/src/mcp`、
> `crates/cloudime-script/src/mcp.rs` 已落地；单测覆盖协议、配置、JSON 转换、Lua 回调与工具白名单。

## 目标与非目标

目标：

- 新增 `cloudime-mcp-client.exe`，独立管理外部 MCP Server 的 stdio 子进程；
- Lua 可以列出配置的 Server、显式启动 / 停止、列出工具、异步调用工具；
- 支持 `mcp.toml` 的 server 命令、参数、环境变量、超时和工具白名单；
- sidecar 崩溃、超时或外部工具失败只让当前调用失败，不影响输入。

## 硬规则（2026-10-28 追加）

1. MCP Client sidecar 与外部 MCP Server **都不随输入法启动**，必须由用户手动启动，或由 Lua
   `cloudime.mcp.start` 显式启动；`tools` / `call` 不会隐式拉起进程。
2. 由 Lua 启动的进程必须在脚本这一轮任务链结束前关闭：从 `start` 到最后一条 MCP 回调结束
   （没有挂起的 MCP 请求）时，运行时自动 `stop` 该脚本启动的 Server；sidecar 是 Lua 拉起的，
   同时 `Shutdown`。用户手动启动的 sidecar 不受影响。

非目标：

- 不做 Streamable HTTP；
- 不做 MCP sampling / roots / elicitation / resources / prompts；
- 不提供生成模型或内置写作功能；
- 不允许脚本动态新增 Server；
- 不在输入工人线程上等待 MCP。

## 配置

`%APPDATA%\CloudIME\mcp.toml`：

```toml
[[servers]]
name = "search"
command = "C:\\Tools\\mcp-search.exe"
args = []
timeout_ms = 10000
allowed_tools = ["search", "fetch"]

[servers.env]
API_KEY = "..."
```

`allowed_tools` 为空或不写时不允许任何工具；脚本只能访问配置里的 Server 和工具。
第一版修改配置后重启 Server，不做热加载。

## IPC

`\\.\pipe\cloudime-mcp-client`，长度前缀 JSON；消息类型：

```text
McpRequest  { id, action }
McpResponse { id, body }
```

- `ListServers`：只读配置名字；
- `StartServer` / `StopServer`：显式建立 / 关闭一个外部 MCP Server 的 session；
- `ListTools`：返回已启动 session 的工具；
- `Call`：带 server、tool、arguments、timeout；
- `Shutdown`：关闭所有 session 并退出 sidecar。
- 管道使用系统默认 ACL（当前用户可访问）；sidecar 的日志走 stderr，不混入协议。

## 进程与生命周期

- sidecar **不随输入法启动**；`cloudime.mcp.start` 第一次显式启动时，Server 才以
  `--pipe <name> --config <path>` 拉起它；
- 外部 MCP Server 也只在 `StartServer` 时建立 session，`tools` / `call` 不会隐式启动；
- 脚本任务链结束：自动 `StopServer` 该脚本启动的 Server；sidecar 是 Lua 拉起的且无其他
  MCP scope 时发 `Shutdown`；
- sidecar 收到 `Shutdown` 先关所有外部 session（连同子进程），响应写回后退出；
  Server 最多等 2 秒，超时才强杀；
- 用户手动启动的 sidecar 不会被脚本关掉；Server 退出或管道断开时 sidecar 退出；
- 一个 Server 一个 session，调用串行；不同 Server 可并行。

## Lua API

```lua
cloudime.mcp.servers()                         -- 配置里的名字，不启动进程
cloudime.mcp.start("search", 5000, cb)         -- 显式启动；链尾自动停
cloudime.mcp.tools("search", 5000, cb)         -- 异步；server 必须已 start
cloudime.mcp.call("search", "search", args, 5000, cb)
cloudime.mcp.stop("search", 5000, cb)          -- 提前停
```

`start` / `stop` 回调：

```lua
{ server = "search", started = true, client_owned = true }  -- client_owned：sidecar 是否本次由 Lua 拉起
{ server = "search", stopped = true }
```

`call` 回调结果：

```lua
{
    server = "search",
    tool = "search",
    content = { { type = "text", text = "..." } },
    structured = { ... },
    is_error = false,
    error = nil,
}
```

异步回调沿用 `poll_requests` 的动作表（`online` / `notice` / `display` 等），
不自动上屏，不阻塞输入。

## 文件

```text
crates/cloudime-mcp-client/        # mcp.toml、命名管道帧、IPC 协议
apps/mcp-client/                   # cloudime-mcp-client.exe（rmcp client + 管道服务端）
apps/windows/server/src/mcp/       # 启动 sidecar、代理结果
crates/cloudime-script/src/mcp.rs  # JSON / Lua 转换与结果表
```

## TDD Route

- Mode: auto
- Decision: strict
- Authority: 新 Lua 公共 API + 新 sidecar 协议 + 新发布二进制
- Verification: 协议单测、mock MCP Server、Lua 回调、sidecar 崩溃/超时测试，
  以及 `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace --locked`。

## 验收

- `mcp.toml` 缺失时 `cloudime.mcp` 不可用，输入法正常启动；
- 不调用 `start` 时 sidecar 和外部 Server 都不启动，`tools` / `call` 回 `error`；
- Lua 显式 `start` 后能列出工具并异步调用 mock 工具；
- 脚本任务链结束后，Lua 拉起的 sidecar 和外部 Server 都已退出；
- 未知 Server / 未 start / 未知工具 / 超时 / sidecar 崩溃都回 `error`，不阻塞输入；
- 正常输入不经过 sidecar；
- 现有 `cloudime-mcp-server` Server 保持不变。
