# 游戏兼容 P1/P2 实施计划（2026-11-10）

> 依据：[`docs/design/game-compat-tsf-uielement.md`](../design/game-compat-tsf-uielement.md)（已定稿）。
> 本文只拆执行顺序与验证点；设计与取舍以那份文档为准。
>
> 执行状态：未开始。P1 可立即做；P2 的“完整安装 / 系统目录”部分等 Certum 代码签名证书。

## 0. 目标与验收

P1（UI-less）：

- 声明 UI-less 的宿主自己画候选，服务端不弹候选窗；
- `host_drawn_candidates` 名单里的 IMM32 读取者，读到候选串后让出绘制权；
- 编码在抑制态强制行内；
- 本机 UI-less 测试宿主上全链路通过，Notepad / Chromium / QQ 无回归。

P2（安装方式 + 系统目录）：

- 签名构建的安装向导提供「推荐安装（缺省）/ 完整安装」；
- 只有完整安装写 `System32\IME\CloudIME` / `SysWOW64\IME\CloudIME` 并注册系统副本；
- 未签名构建只装 Program Files，不出现安装方式选择；
- 升级/卸载能正确处理两处副本与模式切换；
- CS2 在签名版 + 完整安装下通过（等证书）。

## 1. 基线与约束

Requirement Ready Check：`ready`。依据是已定稿的
[`docs/design/game-compat-tsf-uielement.md`](../design/game-compat-tsf-uielement.md)；
范围、验收、非目标、签名门槛都在那份文档里确认过。

- 设计文档：`docs/design/game-compat-tsf-uielement.md`；
- 协议：`PROTOCOL_VERSION` 23 → 24；DLL / Server 必须同一份源码编出、一起安装；
- TSF 接口在 `windows` 0.62.2 里齐全（`ITfTextInputProcessorEx` / `ITfThreadMgrEx` /
  `ITfUIElementMgr` / `ITfUIElementSink` / `ITfCandidateListUIElement` /
  `ITfCandidateListUIElementBehavior` / `TF_TMAE_UIELEMENTENABLEDONLY`）；
- 代码组织、注释、提交信息按 `docs/contributing.md`；改动前后跑
  `cargo fmt --all`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、
  `CLOUDIME_UIACCESS=0 cargo test --workspace --locked`；
- **签名门槛**：未签名构建不启用系统目录部署，也不出现「完整安装」；
- 不做：Dota 2 兼容名、`ITfIntegratableCandidateListUIElement`、`ITfReadingInformationUIElement`、
  跨进程隐藏 `CiceroUIWndFrame`。

### 1.1 变更必要性、影响面与压力检查

- **Change Necessity**：UI-less 必须新增 COM 接口与宿主状态，配置 / 文档替代不了；
  安装方式选择必须改 `cloudime.iss` 的条件编译与注册顺序；系统目录的 CS2 兼容必须靠
  代码签名，代码里没有绕过去的办法。最小代码边界就是上面 A1–E3 / P2-A1–B2。
- **Ripple Signal Triage**：协议（生产/消费两端：DLL ↔ Server）、候选状态与 preedit 模式、
  安装器与开发脚本、`launch.rs` 的 Server 定位、用户安装文档。规范所有者分别是
  `cloudime-platform::protocol`、`dispatch`、`cloudime.iss` / `reconfigure.ps1`、
  `service/launch.rs`；兼容边界是 v24 两端同源、未签名构建保持现有布局；
  不保留第二套候选数据通道（复用 `Frame`）。
- **Plan Pressure Test**：现有 `Frame` / `dispatch` / `cloudime.iss` 都是合适的所有者，
  不需要新架构层；验证点明确（测试宿主 + 安装场景）；任务按依赖排序可执行。

## 2. TDD Route

- mode：`auto`（仓库没有显式 TDD 配置，也没有用户要求 strict）；
- decision：P1 的协议 / 状态机 / 服务端抑制 / 动作 **strict**（先写失败测试，再实现）；
  P2 的安装器 / 脚本 / 文档 **light**（无纯函数单测，靠 `ISCC` 编译与安装场景验证）；
- authority：协议与候选状态是共享契约（`cloudime-platform` ↔ DLL ↔ Server），
  回归代价高；P2 主要是部署与脚本，行为由安装场景覆盖；
- verification：每个 strict 任务先跑对应 crate 的测试，再跑 workspace 检查。

## 3. 任务

### P1-A 协议与配置

#### A1 协议 v24

文件：

- `crates/cloudime-platform/src/protocol/client.rs`
- `crates/cloudime-platform/src/protocol/server.rs`
- `crates/cloudime-platform/src/protocol/mod.rs`
- `crates/cloudime-platform/src/protocol/codec.rs`（测试）

改动：

1. 新增 `CandidateAction { Select(usize), Finalize, Abort }`（`Serialize` / `Deserialize`）；
2. `ClientMessage::UiElementState { session, host_draws, ui_less, host_reads }`（异步）；
3. `ClientMessage::UiElementAction { session, action: CandidateAction }`（同步，回
   `ServerMessage::Update`）；
4. `InputSettings` 增加 `#[serde(default)] host_reads_opt_in: bool`；
5. `PROTOCOL_VERSION = 24`，在 `protocol/mod.rs` 顶部按现有格式补一段 v24 说明
   （为什么加、老 DLL 会怎样）。

先写测试：新消息 / 动作的 encode→decode 往返；`InputSettings::default()` 的
`host_reads_opt_in == false`。

验证：`cargo test -p cloudime-platform protocol`。

#### A2 配置项与 RouterConfig

文件：

- `crates/cloudime-platform/src/config/candidate.rs`
- `crates/cloudime-platform/src/config/mod.rs`（模板 / `template_parses_to_defaults`）
- `apps/windows/server/src/dispatch/config.rs`
- `docs/notes/crate-notes.md`（`[candidate]` 段一句话）

改动：

1. `CandidateConfig` 增加 `host_drawn_candidates: Vec<String>`，缺省空；
2. 配置模板加注释：IMM32 桥接读取候选串的宿主才需要，exe 文件名，大小写不敏感；
3. `RouterConfig` 增加同名字段、`From<&Config>` 映射、`Default`；
4. 新增 `RouterConfig::host_draws_candidates_for(program)`（trim + `eq_ignore_ascii_case`），
   与 `hides_candidate_for` 同风格。

先写测试：模板解析等于默认值（现有测试覆盖）；`host_draws_candidates_for` 大小写 /
空格 / 空名单。

验证：`cargo test -p cloudime-platform`、`cargo test -p cloudime-windows-server`。

### P1-B DLL 帧缓存与状态

#### B1 缓存最新 Frame

文件：

- `apps/windows/tsf/src/com/composition/shared.rs`
- `apps/windows/tsf/src/com/service/key_sink.rs`
- `apps/windows/tsf/src/com/service/document.rs`

改动：

1. `Shared` 增加 `last_frame: RefCell<Frame>`（`Frame: Clone`）；
2. `KeyResult` / `Update` / `Poll` 应答应用时写入；空帧也写（用于 `EndUIElement`）；
3. 提供 `last_frame()` 返回 clone（不在借用期间做 IPC）。

验证：`cargo test -p cloudime-windows-tsf`；编译器检查所有帧应用点都写了缓存。

#### B2 UIElement 状态

文件（新）：

- `apps/windows/tsf/src/com/uielement/mod.rs`
- `apps/windows/tsf/src/com/uielement/state.rs`

改动：

1. `UiElementState { ui_less, host_draws, host_reads, sent }`；
2. `set_*` 只在变化时返回 `true`；`reset()` 在 `Deactivate` 时清零并强制重报；
3. 发送走现有“不等回话”消息路径（`ClientMessage::UiElementState`）。

先写测试：三个标志各自只在翻转时返回变化；`reset` 后重报。

验证：`cargo test -p cloudime-windows-tsf`。

### P1-C UIElement COM

#### C1 `ITfTextInputProcessorEx`

文件：

- `apps/windows/tsf/src/com/service/mod.rs`（`#[implement(...)]` 列表）
- `apps/windows/tsf/src/com/service/processor.rs`

改动：

1. 增加 `ITfTextInputProcessorEx` 实现；`ActivateEx` 复用现有 `Activate` 的连接/注册逻辑，
   额外读 `dwFlags & TF_TMAE_UIELEMENTENABLEDONLY`；
2. 把 `ui_less` 写进 B2 的状态并在会话建立后上报；
3. `ITfTextInputProcessor` 保留（老宿主仍 QI 它）。

验证：`cargo clippy`、测试宿主日志出现 UI-less 激活与状态上报。

#### C2 元素对象与 getter

文件（新）：

- `apps/windows/tsf/src/com/uielement/element.rs`

改动：

1. `#[implement(ITfUIElement, ITfCandidateListUIElement, ITfCandidateListUIElementBehavior)]`；
2. 快照 = `Shared.last_frame()` 的候选页（items / highlight / page / page_count）；
3. `GetCount` / `GetPageIndex` / `GetCurrentPage` / `GetSelection` / `GetString` /
   `GetUpdatedFlags` / `GetDescription` / `IsShown` / `GetDocumentMgr` 按设计文档 §2.4；
4. `SetSelection` / `Finalize` / `Abort` 调新的 `Engine` 动作方法；
5. `GetString` 第一次调用置 `host_reads` 并上报；
6. getter 只读本地快照，不做 IPC。

先写测试：快照映射（空列表、`usize::MAX` 高亮、`display` 优先、页内下标边界）。

验证：`cargo test -p cloudime-windows-tsf`、`cargo clippy`。

#### C3 生命周期接线

文件：

- `apps/windows/tsf/src/com/uielement/mod.rs`
- `apps/windows/tsf/src/com/service/key_sink.rs` / `document.rs`
- `apps/windows/tsf/src/com/service/mod.rs`

改动：

1. 从 `ITfThreadMgr` QI `ITfThreadMgrEx` / `ITfUIElementMgr`（拿不到就降级为无 UIElement）；
2. 第一帧到达且（`ui_less` 或 `host_reads_opt_in`）→ 创建元素 + `BeginUIElement`；
3. `pbShow == FALSE` → `host_draws`，`UpdateUIElement`；
   `pbShow == TRUE` 且不在 opt-in → `EndUIElement`；
   `pbShow == TRUE` 且在 opt-in → 保留元素观察 `GetString`；
4. 帧变化 → 更新快照 + `UpdateUIElement`；空帧 / 组句结束 / 失焦 → `EndUIElement`；
5. `Deactivate` 清理元素、状态、会话。

验证：测试宿主完整跑通；Notepad 不创建元素。

### P1-D 服务端抑制与动作

#### D1 SessionInfo 与消息处理

文件：

- `apps/windows/server/src/dispatch/session/info.rs`
- `apps/windows/server/src/dispatch/message.rs`
- `apps/windows/server/src/dispatch/mod.rs`

改动：

1. `SessionInfo` 增加 `ui_element: UiElementFlags { ui_less, host_draws, host_reads }`；
2. `UiElementState` → 更新对应会话；状态实际变化且该会话是 `focused` 时，
   调一次 `reconcile_candidates(&self.self_drawn_frame())`；
3. `UiElementAction` → 校验 `focused == session`，处理动作，回 `Update { frame, commit }`；
4. `Router::ui_suppressed_by_host()` 按设计文档 §2.5 实现。

先写测试：三个条件各自命中；名单外 `host_reads` 不抑制；`ui_less` 单独命中。

验证：`cargo test -p cloudime-windows-server`。

#### D2 抑制落点

文件：

- `apps/windows/server/src/dispatch/candidates/mod.rs`
- `apps/windows/server/src/dispatch/composed/mod.rs`
- `apps/windows/server/src/dispatch/status_tip/mod.rs`

改动：

1. `reconcile_candidates` 开头：抑制时 `hide_candidate_window()` 并返回；
2. `effective_preedit_mode`：`exclusive_cached() || ui_suppressed_by_host()` → `Inline`；
3. 状态切换提示：抑制时不弹；工具栏沿用全屏规则，不改；
4. 状态翻转后立即重算候选窗。

先写测试：抑制态下 frame 的 `preedit_mode == Inline`；候选窗不显示；状态提示不弹。

验证：`cargo test -p cloudime-windows-server`。

#### D3 候选动作

文件：

- `apps/windows/server/src/dispatch/candidates/mod.rs`
- `apps/windows/server/src/dispatch/composed/mod.rs`

改动：

1. `Select(index)`：把页内下标 + 页起点落到 `highlight`（越界忽略）；
2. `Finalize`：提交当前高亮候选，`pending_commit` 带文本，清组句；
3. `Abort`：清组句、收窗；
4. 应答的 `Update` 走现有 DLL 应用路径（`apply_poll_commit`）。

先写测试：Select 改高亮；Finalize 回 commit；Abort 清空；非 focused 会话忽略。

验证：`cargo test -p cloudime-windows-server`。

### P1-E 测试宿主与回归

#### E1 单元测试补齐

- 协议往返（A1）、配置（A2）、状态（B2）、快照（C2）、抑制（D1/D2）、动作（D3）；
- 每个 strict 任务先写测试；实现后全跑。
- 验证：`CLOUDIME_UIACCESS=0 cargo test --workspace --locked`。

#### E2 本地 UI-less 测试宿主

新 crate：`apps/windows/tools/cloudime-uiless-host/`

- `Cargo.toml` 加进 workspace members；依赖 `windows`（TextServices / WindowsAndMessaging 等）；
- E2a：窗口 + `ITfThreadMgrEx::ActivateEx(TF_TMAE_UIELEMENTENABLEDONLY)` +
  `ITfUIElementSink`（`pbShow=FALSE`）+ 控制台打印 Begin/Update/End；
- E2b：最小 `ITextStoreACP` + `--auto`（输入 `nihao`、翻页、`SetSelection`、`Finalize`）；
- 不是安装包的一部分，不注册 TSF。

验证：`cargo run -p cloudime-uiless-host -- --auto`（先 `push-local` 安装当前构建）；
日志/退出码表示全链路成功。

#### E3 回归

- Notepad / Chromium / QQ：候选窗、组句、上屏、翻页不变；
- 日志确认这三类不创建 UIElement、不发 `UiElementState`（未命中 opt-in）。
- 验证：手工 + `docs/notes/crate-notes.md` 记录。

### P2-A 安装方式（`cloudime.iss`）

#### A1 任务页与条件编译

文件：`apps/windows/installer/cloudime.iss`

改动：

1. `SystemDeploy` 预处理开关（由 `build.ps1` 传入）：
   - 未定义：只编译推荐安装路径，不出现任务页；
   - 定义：`[Tasks]` 两个 `Flags: exclusive` 项 `recommended`（缺省）/ `full`（`unchecked`）；
2. `[Files]`：`{app}` 两份 DLL 始终安装；系统目录两份只在 `Tasks: full`；
3. `[Run]`：`{app}` 的 `regsvr32` 用 `Tasks: not full`；系统副本的 `regsvr32` 用 `Tasks: full`
   （x86 用 `{syswow64}\regsvr32.exe`）；
4. `[Registry]`：`InstallDir` 始终写；`SystemDeploy` 按任务写 1 / 0；
5. `[UninstallRun]` / `[UninstallDelete]`：系统副本与 `{app}` 副本都覆盖；
6. `[Messages]`：`TasksPageCaption` / `TasksPageDescription` / 任务描述说明两种安装方式；
7. `[Code]`：读旧 `SystemDeploy`，处理完整→推荐（反注册/删系统副本）与推荐→完整（补装/
   注册）；系统目录注册失败退回 `{app}` 并写安装日志；
8. `[InstallDelete]`/`RetireLoadedDll`：系统目录与 `{app}` 两处旧 DLL 都改名让路。

验证：

- `ISCC` 分别在不传 / 传 `/DSystemDeploy=1` 下编译通过；
- 签名的测试包：默认推荐；选完整后系统目录有文件、`InprocServer32` 指向系统副本；
- 推荐→完整→推荐的升级切换；卸载后两处无残留。

#### A2 开发脚本

文件：

- `apps/windows/scripts/push-local.ps1`
- `apps/windows/scripts/reconfigure.ps1`

改动：

1. `push-local` 增加 `-SystemDir` 并透传给 `reconfigure`（提权重入也要带）；
2. `reconfigure`：
   - 默认只覆盖 `{app}`、注册 `{app}`（现状）；
   - `-SystemDir`：复制两份 DLL 到系统目录、用系统 `regsvr32` 注册系统副本、写
     `InstallDir` / `SystemDeploy`；
   - `Resolve-InstallDir` 先读 `InstallDir`，再 `InprocServer32` 同目录，再自启快捷方式；
3. 未签名构建默认不带 `-SystemDir`，文档写清。

验证：本机 `push-local`（不带 / 带 `-SystemDir`）各跑一次；Server 能启动、输入法可用；
`InprocServer32` 指向正确。

#### A3 文档

- `apps/windows/installer/README.md`：安装方式、布局、升级/卸载、签名门槛；
- `docs/notes/release.md`：签名版才开 `-SystemDeploy`，CI 变量与签名步骤绑定；
- `docs/user/getting-started/install.md`：推荐 / 完整安装的选择与区别；
- `docs/user/help/uninstall.md`：系统目录副本的卸载；
- `docs/notes/crate-notes.md`：安装器与 `launch.rs` 的 InstallDir 记录。

验证：与实现一致，无“未签名也建议完整安装”的说法。

### P2-B 签名 / CS2（等证书）

#### B1 构建签名

文件：

- `apps/windows/installer/build.ps1`
- `apps/windows/installer/sign-local.ps1`

改动：

1. `build.ps1` 增加签名步骤（Certum 证书指纹来自 CI secret / 本机参数）；
2. 只有签名成功才传 `-SystemDeploy=1` 给 `ISCC`；
3. 签 x64/x86 DLL、Server、设置程序、安装包。

验证：`signtool verify /pa` 全部通过；未配置证书时构建自动退回未签名 / 推荐安装。

#### B2 CI 发版

文件：

- `.github/workflows/release.yml`
- `docs/notes/release.md`

改动：

1. 证书就绪后去掉 `CLOUDIME_UIACCESS=0`，加签名步骤；
2. release 构建传 `-SystemDeploy=1`；
3. `build-info` / `SHA256SUMS` 覆盖系统 DLL；
4. 发版前向 Microsoft Defender / 主流杀软提交样本。

验证：CI 产物签名正确；安装包两种模式都可装。

#### B3 CS2

- 完整安装 + 签名版：`trustedlaunch.cfg` 无拒绝；游戏内聊天出候选。
- 未签名 / 推荐安装：不入 System32，不承诺 CS2。
- 阻塞：Certum 证书。

## 4. 验证矩阵

| 阶段 | 命令 / 场景 | 通过标准 |
|---|---|---|
| P1 单元 | `CLOUDIME_UIACCESS=0 cargo test --workspace --locked` | 全绿 |
| P1 静态 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | 无警告 |
| P1 测试宿主 | `cargo run -p cloudime-uiless-host -- --auto` | Begin/Update/End、候选页、SetSelection/Finalize 全通，服务端不弹窗 |
| P1 回归 | Notepad / Chromium / QQ 打字 | 无行为变化，无新 UIElement |
| P2 安装编译 | `ISCC`（不带 / 带 `-SystemDeploy`） | 两种包都能编出 |
| P2 推荐安装 | 安装向导默认项 | `System32\IME\CloudIME` 无文件，`InprocServer32` 指向 `{app}` |
| P2 完整安装 | 选完整 + 签名包 | 两处文件齐全，`InprocServer32` 指向系统副本，CS2 可加载 |
| P2 切换/卸载 | 完整→推荐、卸载 | 系统目录与 `{app}` 无残留，`SystemDeploy` 正确 |

## 5. 兼容与退场

- 老 DLL / 新 Server：协议 v24，两端同源；安装器与 `push-local` 一起换；
- `ITfTextInputProcessor` 保留；UIElement 只在 UI-less / opt-in 时创建，普通宿主零变化；
- 未签名构建保持现有 Program Files 布局，不引入系统目录；
- 退场触发：若 `ITfCandidateListUIElement` 在某个宿主上造成崩溃/卡死，先按 pid 关掉
  UIElement（opt-in 名单反向），再评估是否只保留声明 UI-less 的宿主。

## 6. 风险

- UIElement getter 在宿主 `UpdateUIElement` 调用栈里重入：只读本地快照，禁止 IPC；
- `host_reads` 是推断，默认不据此收窗；名单外误收的代价是“一个候选窗都没有”；
- 系统目录升级的文件锁：两处都改名让路，重启后删；
- 未签名 / 自签证书的杀软与 CS2 兼容：按签名门槛执行，不做未签名系统目录发布；
- 测试宿主需要最小 `ITextStoreACP`，是最容易低估工作量的一块，先做 E2a 再接 E2b。

## 7. 执行方式

- 依赖关系：A1 → A2 → B1/B2 → C1/C2 → C3 → D1/D2/D3 → E；P2-A 可在 P1 后半段并行；
- 采用 **inline** 执行（同一会话按任务顺序做，P2-A 可与 P1-E 并行评估）；
- 需要用户确认的外部边界：P2-B 的 Certum 证书与 CS2 真机验证；
- 每完成一个任务：跑该任务的验证命令，`git diff` 自审，再进入下一个任务。
