# cloudime-mcp-server：只读 stdio MCP Server

2026-10-28 定。目标是把云朵输入法的**静态词库查询**能力暴露给外部 AI 客户端，
不进入 TSF DLL，不接触当前输入会话和用户学习数据。

> 2026-10-28：已实现 `apps/mcp` 并通过 stdio 集成测试；安装包构建脚本已加入 `cloudime-mcp-server.exe`。

## 目标与非目标

目标：

- 提供独立控制台程序 `cloudime-mcp-server.exe`，通过 stdio 提供 MCP `tools`；
- 查询随包主词库和 `WordBank\` 下用户导入的第三方词库；
- 只读：不写词库、不学习、不改输入状态；
- 服务器日志全部写 stderr，stdout 只承载 MCP JSON-RPC。

非目标：

- 不提供按键、上屏、候选切换、脚本执行、网络请求；
- 不读取 `UserWordBank.db`、`%APPDATA%` 学习数据、输入日志或当前输入框；
- 不做 Streamable HTTP；需要时另开设计。

## 数据与进程边界

```text
安装目录\WordBank\Dict.db             主词库（普通中文 + 英文，稀有组默认不读）
安装目录\WordBank\<第三方>.db/.tsv    通过 WordBank::load_except 加载
安装目录\WordBank\UserWordBank.db    明确跳过
```

- 主词库：`DictDb::from_path_without_rare`；`--include-rare` 时才用 `from_path`；
- 第三方词库：`WordBank::load_except(Some(&main))`，坏文件只告警跳过；
- `--no-extras` 只读主词库；
- `--dict <path>` 覆盖主词库；`--word-bank <dir>` 覆盖第三方词库目录；
- 查询使用 `Engine::new` + `with_english` + `set_extra_dictionaries`，不注入 Learner、LM 或神经模型；
- MCP 查询和 `cloudime-server` 完全分离，进程退出不影响输入法。

## MCP 契约

工具名固定为：

| 工具 | 参数 | 结果 |
|---|---|---|
| `cloudime_segment_pinyin` | `pinyin`, `use_jian_pin` | 切分列表、首选切分、`tail` |
| `cloudime_lookup_pinyin` | `pinyin`, `limit`, `use_jian_pin` | 切分列表、候选、候选权重 |
| `cloudime_dictionary_info` | 无 | 主词库与第三方词库的名称、条数、许可证、是否包含稀有组 |

候选结果包含 `text`、`display`、`kind`、`syllables`、`weight`。
输入长度上限 200 字符，候选上限 100，切分列表最多返回 16 个。

错误处理：

- 空拼音、非法拼音：MCP tool error，不崩溃；
- 词库缺失：stderr 写路径原因并以非零状态退出；
- 第三方词库损坏：启动时告警跳过，其他词库照常工作；
- 查询线程与 MCP 运行时隔离，单次查询超时返回 error。

## 技术路线

- 新 crate：`apps/mcp`，二进制名 `cloudime-mcp-server`；
- `rmcp 3.8.0`（官方 Rust SDK）：`server` + `macros` + `transport-io`；
- `schemars` 生成工具参数 / 结果 schema；
- `tokio` 作为 MCP 运行时，`Engine` 放在 `Arc<Mutex<_>>` 中，查询走 `spawn_blocking`；
- `tracing` 初始化到 stderr，禁止 `println!`。

## 文件

```text
apps/mcp/Cargo.toml
apps/mcp/src/main.rs        # 参数、装载、stdio 服务
apps/mcp/src/data/          # 主词库/第三方词库装载与 info
apps/mcp/src/tools/         # MCP 参数、返回 DTO 与查询实现
apps/mcp/src/server.rs      # rmcp ServerHandler / tool router
apps/mcp/tests/stdio.rs     # 真实 stdio 集成测试
```

## 兼容与发布边界

- 不修改 `PROTOCOL_VERSION` 或 TSF DLL；
- `cargo-build.ps1` 增加 `cloudime-mcp-server` 构建与产物检查；
- installer 把 `cloudime-mcp-server.exe` 放到 `{app}`，不注册服务、不开机启动；
- 文档给出 Claude Desktop / Cursor 的 stdio 配置示例；
- 不修改用户配置文件；MCP 的路径选项全部是命令行参数。

## TDD Route

- Mode: auto
- Decision: strict
- Authority: 新公共 MCP tool 契约 + 新发布二进制
- Test posture: 先写工具结果 / 数据边界测试，再写 stdio 集成测试；实现后跑全 workspace
- Verification: `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace --locked`，以及 `cloudime-mcp-server --dict assets/sample/dict.tsv` 的 stdio 测试

## 任务

1. **crate 与依赖**：加入 workspace 成员、`rmcp`/`schemars`/`tokio`/`tracing` 依赖，建立二进制骨架。
2. **数据层**：实现主词库、第三方词库、稀有组开关、`--dict`/`--word-bank`/`--no-extras`，并测试
   `UserWordBank.db` 不会被加载。
3. **工具层**：实现拼音切分、候选查询、词库信息 DTO；测试 `kaifa`、第三方词库命中、非法输入。
4. **MCP 服务**：接入 rmcp stdio、初始化、`tools/list`、`tools/call`、stderr 日志和错误返回。
5. **发布与文档**：构建脚本、installer、用户文档、crate notes。
6. **验证**：stdio 集成测试、完整 workspace 测试、CI 等价的 clippy/fmt。

## 验收

- AI 客户端能启动 `cloudime-mcp-server` 并列出三个工具；
- `lookup_pinyin("kaifa")` 返回“开发”；
- 第三方 `.tsv`/`.db` 中的词能查到，`UserWordBank.db` 中的词查不到；
- stdout 没有日志污染；
- 缺词库时 stderr 有明确原因且退出非零；
- 不启动 Server 也能独立工作。
