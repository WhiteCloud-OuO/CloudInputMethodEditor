---
title: MCP：查询词库与调用外部工具
order: 4
description: 用只读 MCP Server 让外部 AI 客户端查询云朵输入法的词库，也支持脚本通过 MCP Client 调用外部工具。
---

## 用途

安装目录里的 `cloudime-mcp-server.exe` 是一个只读的 Model Context Protocol Server。
AI 客户端可以通过它查询：

- 拼音切分和首选切分；
- 按拼音排序的候选词；
- 当前加载的主词库和导入词库的名称、条数与许可证。

它不会读取当前输入内容、用户自造词、学习记录或输入日志，也不会修改词库。

## 配置

在支持 MCP 的 AI 客户端里添加一个 stdio Server，命令填写安装目录中的程序：

```json
{
  "mcpServers": {
    "cloudime": {
      "command": "C:\\Program Files\\CloudIME\\cloudime-mcp-server.exe"
    }
  }
}
```

不同客户端的配置文件位置不同，具体入口以客户端说明为准。
程序启动后会在对话中提供三个工具：

- `cloudime_segment_pinyin`：切分拼音；
- `cloudime_lookup_pinyin`：查询候选；
- `cloudime_dictionary_info`：查看已加载词库。

## 词库范围

默认加载：

- 随包 `WordBank\Dict.db` 的普通中文和英文；
- `WordBank\` 下用户导入的第三方词库；
- 不加载生僻字 / 生僻词组，除非启动时加 `--include-rare`；
- 不加载 `UserWordBank.db`，也不读取个人学习数据。

调试或只使用指定词库时，可以给程序加参数：

```text
--dict <词库路径>          覆盖主词库
--word-bank <目录>         指定第三方词库目录
--include-rare             加载主词库的生僻组
--no-extras                只查主词库
```

MCP Server 只使用本机词库，不发起网络请求。

## 让脚本调用外部 MCP Server（反向）

安装目录里的 `cloudime-mcp-client.exe` 是 MCP Client sidecar。它**不随输入法启动**：
脚本必须先调 `cloudime.mcp.start`，或者由用户手动运行
`cloudime-mcp-client.exe --config %APPDATA%\CloudIME\mcp.toml`（默认管道名可省略）。
Lua 脚本可以用 `cloudime.mcp.*` 调用外部 MCP Server 提供的工具。

先在 `%APPDATA%\CloudIME\mcp.toml` 里配置（没有这个文件时客户端不启用）：

```toml
[[servers]]
name = "search"
command = "C:\\Tools\\mcp-search.exe"
args = []
timeout_ms = 10000
allowed_tools = ["search"]

[servers.env]
API_KEY = "..."
```

脚本里：

```lua
cloudime.mcp.servers()
cloudime.mcp.start("search", 5000, function(result) … end)
cloudime.mcp.tools("search", 5000, function(result) … end)
cloudime.mcp.call("search", "search", { query = "云朵" }, 5000, function(result) … end)
cloudime.mcp.stop("search", 5000, function(result) … end)
```

- 只允许调用 `mcp.toml` 里的 Server 和 `allowed_tools` 中的工具；
- **必须显式 `start`**：不 `start` 时 sidecar 和外部 MCP Server 都不会启动，`tools` / `call` 会报错；
- **脚本拉起的进程在任务链结束时自动关闭**：从 `start` 到最后一条 MCP 回调结束，
  运行时会停掉该脚本启动的 Server，并关掉它拉起的 sidecar；
- 用户自己手动启动的 sidecar 不会被脚本关闭；
- 调用是异步的，有超时和结果大小限制，失败只让当前回调拿到 `error`；
- 正常打字不经过它，外部 Server 崩溃或卡住不会影响输入；
- 第一版只支持工具调用，不支持 resources / prompts / sampling，也不支持 HTTP 传输；
- 修改 `mcp.toml` 后重启 Server 生效。

完整 API 见安装目录 `lua.md` 的「MCP」一节。
