# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 仓库现状

Windows 输入法（只做 Windows，上游的 macOS / Linux 壳已删除）。Core 平台无关，壳只做接入：`apps/windows/{server,tsf,settings,installer}`。
本仓库从青简 fork、版本号独立，从 `0.0.1` 起，唯一来源是根目录 `version.txt`（当前就是 `0.0.1`，`CHANGELOG.md` 顶上就是它那一节）。
阶段与已完成项见 `docs/plan/roadmap.md`，待办见 `docs/plan/todo.md`。

## 目录地图

一行一个，只说它是什么、入口在哪；实现要点（数据文件、常数、生成命令）在 `docs/notes/crate-notes.md`，改了实现要同步那里。

- `crates/cloudime-core`：引擎。`Engine` 是对外唯一门面，`Learner` 等 trait 在 `engine` 模块；拼音解析、纠错、候选、排序、整句、英文模式都在这里。
- `crates/cloudime-dictionary`：词库（SQLite 存档 `.db`；TSV 与老 `.qj` 也认），按音节位置二分查询。
- `crates/cloudime-learning`：用户侧落盘：词频 / 用户词 / 个人 n-gram / 敲错表（`FrequencyLearner`）、输入日志（`InputLog`）、输入统计（`UsageStats`）、词汇记录（`VocabularyBook`）。
- `crates/cloudime-lm`：整句转换的 bigram 语言模型 `BigramModel`。
- `crates/cloudime-neural`：字级 Transformer 本地推理 `CharScorer`（candle），给整句前几条路径重打分。
- `crates/cloudime-format`：`.qj` 数据容器（mmap 读、零拷贝视图、写入器、哈希索引）。
- `crates/cloudime-platform`：平台层共用：`Config`（TOML 配置）、`extra_dictionaries`、Windows Server ↔ DLL 的 `protocol` 类型。
- `crates/cloudime-render`：自绘渲染器（spike 中，分支 renderer-spike）：候选窗一帧 + 主题 → 位图，壳只贴图。见 `docs/design/rendering.md`。
- `crates/cloudime-update`：检查更新：读官网的 `releases.json`、验 ed25519 签名、按平台与渠道挑新版本，只提示不安装；发版侧的签名工具是 `tools/release-sign`。见 `docs/design/update.md`。
- `apps/cli`：Core 的验证工具：查询、逐键计时、输入日志回放、整句评测、常数扫描。排序 / 整句 / 纠错的改动先跑它再合。
- `apps/windows`：`server`（Server 进程：Engine + IPC + 自绘候选窗与状态条）+ `tsf`（TSF DLL）+ `settings`（WinUI 3）+ `installer`（Inno）。DLL 不能带 Engine 的依赖树，所以是两个 package。
- `tools/dict-convert`、`tools/corpus`：产品数据生成（词库 / 语言模型 / 英文词表），词库合并输出到 `WordBank/Dict.db`、语言模型与英文 TSV 中间产物输出到 `data/generated/`（都 gitignore）。
- `assets/`：随包数据源与样例，各目录有 README 写来源与许可。雾凇拼音（GPL）已彻底移除，不要再引入。

`docs/` 分四类（索引在 `docs/README.md`）：`design/` 设计与决定、`plan/` 路线与待办、`notes/` 工程记录（性能、复盘、踩坑、crate 实现要点）、
`user/` 用户文档（官网构建时拉取渲染，约定见 `docs/user/README.md`，措辞面向用户、不出现实现词）。
根目录 `tutorial.md` 是**随安装包发布的单文件使用手册**（打包时装到 `{app}\tutorial.md`），用户可见的行为改了要同步它，见 `docs/contributing.md`「文档同步」。

## 常用命令

```bash
cargo build                                   # 整个 workspace
cargo test                                    # 全部测试；-p <crate> 单个，加测试名过滤
cargo clippy --all-targets -- -D warnings
cargo fmt --all
cargo run -p cloudime-cli -- <拼音>...          # Core 的主要验证方式；--replay / --eval-text 见 crate-notes
```

开发机就是 Windows，直接 `cargo build` / `cargo test`；跑测试二进制前要 `CLOUDIME_UIACCESS=0`（原因见 `AGENTS.md`）。
装到本机验证走 `apps/windows/scripts/test-local.ps1`；`apps\windows\scripts\reconfigure.ps1` 现在会把 `target\release` 与 `target\i686-pc-windows-msvc\release` 里的 Server / 两份 TSF DLL / 设置程序 / 状态栏图标写进安装目录（先停掉在跑的 Server / 设置程序，被占用的文件先改名让开），再重注册并刷新 ctfmon / Explorer / Server，免得为生效去注销或重启电脑。
只改了代码、已经编好 release 时直接跑 `reconfigure.ps1`；没编就用 `apps\windows\scripts\push-local.ps1`（先带 `CLOUDIME_UIACCESS=0` 编译再调它）。
**刚装完正式安装包时请加 `-SkipCopy`**：否则会把包里的 (签过名 / 带 uiAccess 的) 产物覆盖成本地编的；改动随包数据或版本号也仍请走安装包。
打包与安装流程见 `apps/windows/README.md`、`apps/windows/installer/README.md`；
产物已编好、只想打一次包时用仓库根的 `build-installer.ps1`（只从 `target\release` 提取文件并调 Inno Setup，不编译）；
三个 Windows 壳的版本号来自根目录 `version.txt`，`.\cargo-build.ps1` 负责同步它（`-OnlyVersion` 只同步、不编译）并编出 release 产物（含 32 位 TSF DLL）。

## 约定

架构约束、代码组织、版本号、提交信息、文档同步、提交前检查与发版都在 `docs/contributing.md`，随本文件一起载入：

@docs/contributing.md

交流用中文。
