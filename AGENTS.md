# AGENTS.md

仓库级说明的唯一来源是 [CLAUDE.md](CLAUDE.md)（仓库现状、目录地图、常用命令），动手前先完整读它。
- CLAUDE.md 末尾那行 `@docs/contributing.md` 是 Claude Code 的导入语法，OpenCode 不会展开，**要自己打开 [docs/contributing.md](docs/contributing.md) 读完**：架构约束、代码组织、命名与注释、版本号、提交信息、文档同步、提交前检查、外部 PR 规则。
- 动手改某个 crate 之前读 [docs/notes/crate-notes.md](docs/notes/crate-notes.md) 的对应章节，改完把数据文件、常数、生成命令的变动同步回去。

本文件只是速查，不另立规矩；与上面三份冲突时以它们为准，仓库级改动仍只改 CLAUDE.md。

## 命令

```bash
cargo test -p cloudime-core                 # 单个 crate；再加一个字符串参数过滤到单个测试
$env:CLOUDIME_UIACCESS=0; cargo test --workspace --locked   # 手跑全量测试必设，否则 Server 测试二进制 os error 740（PowerShell 写法；bash 用 CLOUDIME_UIACCESS=0 cargo test …）
cargo clippy --workspace --all-targets -- -D warnings   # 与 .githooks/pre-commit、CI 同款，警告即错误
cargo fmt --all
cargo run --release -p cloudime-cli -- kaifa  # 验证 Core 的主要方式；性能 / 回放 / 评测一律 --release（debug 慢十倍，数字没意义）
```

- 提交前顺序就是 `.githooks/pre-commit` 跑的：装饰性分隔注释检查 → fmt --check → clippy（钩子不跑测试）；全 workspace 测试在 `.githooks/pre-push`（带 `--locked`）。钩子首次要 `git config core.hooksPath .githooks` 启用一次。
- 排序 / 整句 / 纠错的改动合之前先跑 `apps/cli`：`--replay <input-log.jsonl>` 与 `--eval-text`（尺子与冻结句集见 crate-notes「apps/cli」）。
- CLI 的数据与配置文件都按 cwd 解析，**必须在仓库根目录跑**；数据按 `data/generated/*.qj` → 那里的 TSV → `assets/lexicon/` → `assets/sample/` 找，全新 clone 只有样例数据。CLI 缺省配置是根目录 `./config.toml`，与产品读的 `%APPDATA%\CloudIME\config.toml` 不是同一份。

## 编译范围（照抄 CI，别自己发挥）

- 只有 Windows 一个 job：`cargo fmt --check` → `cargo clippy --workspace --all-targets -- -D warnings` → `cargo test --workspace`，再加 `cargo check -p cloudime-windows-tsf --target i686-pc-windows-msvc`（cargo 全 `--locked`）。
- 32 位 target `rust-toolchain.toml` 没声明，新机器先 `rustup target add i686-pc-windows-msvc`，否则最后一步直接失败。
- Windows 跑测试二进制要 `CLOUDIME_UIACCESS=0`：Server 的 build.rs 嵌 uiAccess manifest，未签名的 exe 起不来（os error 740）；pre-push 与 CI 都已带上，**手动 `cargo test` 不会自动带**（见上面命令）。

## 容易踩的坑（文档里都有，漏读代价高）

- 三个 Windows 壳的版本号唯一来源是根 `version.txt`，`.\cargo-build.ps1` 把它写进三个 `Cargo.toml` 再编 release，`-OnlyVersion` 只同步不编译。
- 真机联调用 `pwsh -File apps\windows\scripts\test-local.ps1`，别在仓库里直接跑（`bundled_root()` 按 exe 位置找数据，会退回样例词库，看起来像没装好）。
- `.ps1` 要存成**带 BOM 的 UTF-8**：PowerShell 5.1 对无 BOM 的脚本按系统 ANSI 解析，中文乱码并报「字符串缺少终止符」。编辑时别丢 BOM。
- Server 与 TSF DLL 必须同一份源码编出：`cloudime_core::Candidate` 是线上格式，Core 加一个 `CandidateKind` 变体，老 DLL 就解不出整条帧、把按键原样放行（表现为「突然只出英文」）。加枚举变体要同时把 `protocol::PROTOCOL_VERSION` +1 并重装 DLL，详见 `crates/cloudime-platform/src/protocol/mod.rs` 模块注释。

## 会拦你的东西

- `.githooks/commit-msg`：第一行必须 `类型(范围): 中文说明`（Conventional Commits，范围英文小写，如 `fix(core): 修自绘输入框吞数字`）。
- `.githooks/pre-commit`：装饰性分隔注释（`// ====`、`// ────`）直接拒绝，用空行分组。
- 代码组织硬规矩见 contributing.md「代码组织」，最容易踩的三条：子模块用 `foo/mod.rs` 而不是 `foo.rs` + `foo/`；同词干的兄弟文件收进目录、不用文件名前缀分组；不用 `use super::*` / `use foo::*`（测试模块里的 `use super::*` 除外）。
- 版本号：`crates/*` 与 `apps/cli` 用 `version.workspace = true`，**`apps/windows` 三个壳写死自己的版本**（来自 `version.txt`，改完跑 `.\cargo-build.ps1 -OnlyVersion` 同步）；发版之间带 `-dev`。
- 依赖版本只在根 `[workspace.dependencies]` 定，子 crate 写 `xxx.workspace = true`；错误用 `thiserror`（不用 anyhow），日志用 `tracing`。

## 改了行为要同步的文档

- 用户能感知（按键、菜单、设置、配置文件、数据文件）→ 同一提交改 `docs/user/` 对应页，按键改动同时改 `keys.md`。
- crate 实现要点（数据文件、常数、生成命令）→ `docs/notes/crate-notes.md`。
- CHANGELOG 手写，由维护者发版时统一改，PR 不动它。

交流用中文；标识符英文，注释与文档中文。
