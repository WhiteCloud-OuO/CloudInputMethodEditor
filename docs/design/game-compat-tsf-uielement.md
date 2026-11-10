# 游戏兼容：TSF UI-less 与系统目录部署（2026-11-10 设计）

> 状态：设计已确认，**尚未实施**。P0（全屏独占收窗 + 强制行内编码）已落地
> （`3b5e0d6` / `39b0570` / `02758b1`），本文覆盖 P1（TSF UI-less）与 P2（系统目录 + 签名）。
>
> 相关：`docs/design/architecture.md`、`docs/notes/crate-notes.md`、
> `apps/windows/installer/README.md`、外部参考 WindInput 的
> `docs/design/game-compat-tsf-uielement.md`（结论可参考，协议/缓存模型不照搬）。

## 1. 问题与目标

两类游戏宿主，两种失败：

| 宿主 | 现状 | 后果 |
|---|---|---|
| 声明 UI-less 的宿主（SDL2、部分 UE / ImeSharp 类） | 宿主用 `TF_TMAE_UIELEMENTENABLEDONLY` 激活线程、`BeginUIElement` 回 `pbShow=FALSE`，我们仍然弹自己的候选窗 | 游戏自己画一份、我们再盖一份；候选窗还可能把独占全屏踢出 |
| 走 IMM32 桥接的传统宿主（Civ VI 这类） | 游戏不认 UIElement，msctf 在进程里建 `CiceroUIWndFrame`；候选窗由服务端自绘 | 多一个系统 IME 小窗；候选位置依赖游戏报的假光标矩形（P0 已做兜底） |

目标：

- **P1**：对声明 UI-less 的宿主，宿主自己画候选，我们不弹窗、不盖窗；对命中 opt-in 名单的
  IMM32 读取者，也把绘制权让出去。编码在两种情况下都强制行内。
- **P2**：把 TSF DLL 部署到系统目录并保持代码签名，满足 CS2 的 Trusted Mode；安装/升级/卸载、
  开发部署与 cs2 验证都可重复。

非目标：

- Dota 2 的兼容名（按注册表 Profile Description 全等匹配，改了就全局冒用别家名字）。**不做**；
  Dota 2 保持无候选窗，文档写清。
- 非 UI-less 独占全屏游戏的内联候选（P3 备选，默认不做）。
- 跨进程隐藏 `CiceroUIWndFrame`（反作弊敏感，不做）。

## 2. P1：TSF UI-less

### 2.1 范围

- `ITfTextInputProcessorEx::ActivateEx`：识别 `TF_TMAE_UIELEMENTENABLEDONLY`；
- `ITfUIElement` / `ITfCandidateListUIElement` / `ITfCandidateListUIElementBehavior`；
- 宿主状态（声明接管 / 读取候选串）上报服务端，服务端据此收窗；
- `host_drawn_candidates` 的 per-app opt-in（IMM32 读取者默认不收窗，名单里的才收）；
- 本机 UI-less 测试宿主。

不做：`ITfIntegratableCandidateListUIElement`（Win8 搜索框集成）、
`ITfReadingInformationUIElement`（Dota 2 实测救不了退格）、多页候选（只交当页）。

### 2.2 数据流：复用现有 Frame

DLL 已经在 `KeyResult` / `Update` / `Poll` 的应答里拿到完整 `Frame`（候选、`highlight`、`page`、
`page_count`、`layout`、`columns`）。P1 **不再新增 Query/Page 协议**，只在 DLL 里缓存最新一帧：

- `Shared` 增加 `last_frame: RefCell<Frame>`；
- `key_sink` / `document` 应用帧时同步写缓存；
- UIElement 的 getter 只读这份缓存，**不做任何 IPC**。

好处：热路径零新增往返；服务端帧构造只有一处；DLL/服务端不会出现两份候选快照。

### 2.3 状态模型

DLL 侧（`apps/windows/tsf/src/com/uielement/state.rs`）：

| 标志 | 来源 | 语义 |
|---|---|---|
| `ui_less_thread` | `ActivateEx` 的 `dwFlags` | 宿主已声明走 UI-less，激活时就抑制 |
| `host_draws` | `BeginUIElement` 回 `pbShow=FALSE` / 元素 `Show(FALSE)` | 宿主真的要画；sticky 到 `Deactivate` |
| `host_reads` | `GetString` 第一次被调用 | 宿主读走了候选串（推断，不等于会画） |

只在标志变化时发 `ClientMessage::UiElementState`；`Deactivate` / 断线时清零并重报。

服务端侧（`SessionInfo` 增加 `ui_element`）：

```text
ui_suppressed_by_host() =
    ui_less_thread
 || host_draws
 || (host_reads && exe 命中 [candidate] host_drawn_candidates)
```

`ui_less_thread` 是宿主**声明**，直接信任；`host_reads` 是**推断**，必须名单 opt-in
（参考 WindInput 的负对照：Notepad / Illustrator / EverEdit 都会读但都不画）。

### 2.4 UIElement 生命周期

`apps/windows/tsf/src/com/uielement/`：

- `element.rs`：`#[implement(ITfUIElement, ITfCandidateListUIElement, ITfCandidateListUIElementBehavior)]`；
- 组句第一帧到达后、且（`ui_less_thread` 或 `host_reads_opt_in`）时创建元素并 `BeginUIElement`：
  - `pbShow=FALSE` → `host_draws=true`，立即 `UpdateUIElement` 交快照；
  - `pbShow=TRUE` 且不在 opt-in 名单 → 普通宿主，立即 `EndUIElement`，不改变现有行为；
  - `pbShow=TRUE` 但在 opt-in 名单 → 元素留到组句结束，用来观察 `GetString` 是否被读
    （读了才说明宿主可能自己画，服务端再决定收不收窗；不读就完全无影响）；
- 帧变化：宿主画（`host_draws` 或 `ui_less_thread`）→ 更新快照 + `UpdateUIElement`；
- 空帧 / 组句结束 / 失焦 → `EndUIElement`，元素作废；
- `GetString` 第一次调用 → `host_reads=true` 并上报（无需等回话）。

快照形状：**只带当页**，与本地候选窗一致：

| getter | 值 |
|---|---|
| `GetCount` | `frame.candidates.items.len()` |
| `GetPageIndex` | `[0]`，页数 1 |
| `GetCurrentPage` | `0` |
| `GetSelection` | `frame.highlight`（`usize::MAX` 回 `S_FALSE`） |
| `GetString(i)` | `display` 优先，回退 `text` |
| `GetUpdatedFlags` | 每次更新置全部 `TF_CLUIE_*` 位 |
| `GetDocumentMgr` | `E_NOTIMPL` |
| `GetDescription` | `云朵输入法候选` |
| `IsShown` | 元素存在期间为 `TRUE` |

行为接口（宿主调用）：

- `SetSelection(n)` → `UiElementAction::Select(页内下标 + 页起点)`，同步等 `Update`；
- `Finalize()` → `UiElementAction::Finalize`，应答的 `commit` 走现有「鼠标点候选」上屏路径；
- `Abort()` → `UiElementAction::Abort`，清组句、收窗。

所有调用都在 TSF 线程（STA）上；getter 只读本地快照，绝不做 IPC。

### 2.5 服务端抑制

命中 `ui_suppressed_by_host()` 时：

- `dispatch/candidates/mod.rs::reconcile_candidates`：`hide_candidate_window`，不显示自绘候选窗；
- `dispatch/composed/mod.rs::effective_preedit_mode`：不管配置是 `both` / `window`，强制 `Inline`
  （与 P0 的全屏强制 inline 同一套理由：候选窗不画了，编码只剩组合区一个出口）；
- 状态切换提示（`status_tip`）也不弹；悬浮工具栏沿用 P0 的全屏规则，不因 UI-less 额外收；
- 状态翻转时立即 `reconcile_candidates(&self.self_drawn_frame())`，不能等下一次按键。

配置：`[candidate] host_drawn_candidates = []`（exe 文件名列表，缺省空，大小写不敏感）。
先只做配置文件键，不加设置页；配置模板里写清「IMM32 桥接读取候选串的宿主才需要」。

### 2.6 协议 v24

`PROTOCOL_VERSION` 23 → 24。新增：

```rust
ClientMessage::UiElementState {
    session: SessionId,
    host_draws: bool,
    ui_less: bool,
    host_reads: bool,
}
// 异步、变化时才发，不回话。

ClientMessage::UiElementAction {
    session: SessionId,
    action: CandidateAction,
}
// 同步，回 ServerMessage::Update { frame, commit }（复用现有变体）。

enum CandidateAction {
    Select(usize), // 页内下标
    Finalize,
    Abort,
}
```

`InputSettings` 增加 `host_reads_opt_in: bool`（服务端按会话的 exe 名查名单算好）；
`OpenSession` / 会话建立时不需要额外字段——`SessionInfo.app` 已有 exe 名，抑制判定按 `focused` 会话取。

兼容性：新 DLL 对新 Server；老 DLL 不认识新消息，但服务端不会主动发新变体。DLL/Server 必须同源，
升级时两端一起换（协议注释与安装器都按这个前提）。

### 2.7 本地 UI-less 测试宿主

新 crate `apps/windows/tools/cloudime-uiless-host`（Rust + `windows`）：

- 普通 Win32 窗口 + `ITfThreadMgrEx::ActivateEx(TF_TMAE_UIELEMENTENABLEDONLY)`；
- advise `ITfUIElementSink`，`BeginUIElement` 一律 `pbShow=FALSE`；
- 最小 `ITextStoreACP`（只够 TSF 组句与取文本）；
- 控制台打印 Begin / Update / End、`GetCount` / `GetString` / `GetPageIndex` / `GetSelection`；
- `--auto`：自动敲 `nihao`、翻页、`SetSelection`、`Finalize`，退出码表示全链路成功。

不随安装包发布，只作为开发验证工具。

## 3. P2：系统目录 + 签名

**前置门槛：系统目录部署只对签名版启用。**

- 现状：发布包没有代码签名（`release.yml` 仍是 `CLOUDIME_UIACCESS=0`，`docs/notes/release.md`
  写着 Certum 证书办下来后再接 signtool）；
- 风险：“未签名 DLL + 系统目录 + 注册成文本服务（加载进每个进程）”是杀软启发式最敏感的组合之一。
  公开先例：小狼毫 `weasel.dll` 被 Windows Defender 误杀（rime/weasel#922），早期 NOD32 误报
  `imm32.dll` 导致输入法全挂。**不能保证不误报**；
- 而且未签名放进 System32 对 CS2 没有收益：Trusted Mode 要的是“系统目录 **且** 有效签名”，
  未签名照样被拒；
- 因此：
  - **未签名构建**（开发版 / alpha）：只装 `{app}`，不碰系统目录；
  - **签名构建**（Certum release）：系统目录为主 + `{app}` 兜底；
  - 由构建脚本显式开关（`-SystemDeploy` / CI 变量）决定，**不按文件签名状态在运行时猜**
    （开发自签证书看起来也是“已签名”，但不受其它机器信任）；
- **安装时由用户选安装方式**（仅签名构建提供）：
  - **推荐安装**（缺省）：只写 `{app}`，就地注册；不碰系统目录；
  - **完整安装**：写 `{app}` + `System32\IME\CloudIME` / `SysWOW64\IME\CloudIME`，
    注册系统副本（CS2 等游戏需要）；
  - 未签名构建的安装包不出现这个选择，只有推荐安装；
  - 选择结果写入 `HKLM\Software\CloudIME\SystemDeploy`（1 = 完整，0 = 推荐），
    升级时按旧值判断要不要清理/补装系统副本；
- 签名版的缓解措施：所有二进制（x64/x86 DLL、Server、设置、安装包）都签、带时间戳；
  发版前向 Microsoft Defender 与主流杀软提交样本/误报申诉；DLL 放在产品专属子目录
  `System32\IME\CloudIME`（不是 System32 根）；DLL 不做网络、自更新、系统目录以外的持久化；
  发布 SHA-256 与明确的下载来源。

### 3.1 部署布局

**完整安装**（签名版；`-SystemDeploy`，用户选“完整安装”）：

```text
%WINDIR%\System32\IME\CloudIME\
    cloudime_tsf_x64.dll        ← 64 位注册副本（InprocServer32 指向它，主要生效路径）
%WINDIR%\SysWOW64\IME\CloudIME\
    cloudime_tsf_x86.dll        ← 32 位注册副本（WOW6432Node）
C:\Program Files\CloudIME\
    cloudime_tsf_x64.dll        ← 兜底副本（不注册；修复 / 系统目录不可用时用）
    cloudime_tsf_x86.dll
    cloudime-server.exe
    cloudime-settings.exe
    …（数据、模型、词库、短语、脚本）
```

**推荐安装**（缺省；未签名版唯一选项）：

```text
C:\Program Files\CloudIME\
    cloudime_tsf_x64.dll        ← 唯一副本，就地注册（与现在一致）
    cloudime_tsf_x86.dll
    cloudime-server.exe
    …
```

- **主路径（仅完整安装）**：安装器把 DLL 复制到系统目录，用**系统目录里的** `regsvr32`
  （x86 用 `SysWOW64\regsvr32.exe`）注册系统副本；CS2 的 Trusted Mode 只认这条；
- **兜底副本（仅完整安装）**：`{app}` 里同时保留一份同名 DLL，不注册。系统目录写入失败、修复安装、
  或开发机不想动系统目录时，可以改注册 `{app}` 副本（功能完整，只是 CS2 加载不到它）；
- 安装器注册完系统副本后校验 `InprocServer32` 是否指向系统副本；没指过去就退回注册
  `{app}` 副本，并在安装日志里写明（不让用户以为 CS2 兼容已生效）；
- 注册前写 `HKLM\Software\CloudIME\InstallDir = {app}`（REG_SZ）；
- 升级时两处副本一起更新，注册仍指向系统副本。

### 3.2 DLL 找 Server

DLL 从系统目录加载后 `GetModuleFileName` 指向 System32，不能再按“DLL 同目录”找 Server：

- `com/service/launch.rs::server_exe()`：先读 `HKLM\Software\CloudIME\InstallDir`，拼
  `cloudime-server.exe`；读不到再回退 DLL 同目录（开发/便携）；
- 该键在 `regsvr32` **之前**写入，装完第一次就可能被宿主加载；
- 开发脚本 `apps/windows/scripts/reconfigure.ps1` 的安装目录反查也要先读 `InstallDir`：
  系统副本注册后 `InprocServer32` 指向 `System32\IME\CloudIME`，按旧逻辑会去那里找 Server
  （那里没有），必须改成“先 `InstallDir`，再 `InprocServer32` 同目录，再自启快捷方式”。

### 3.3 安装 / 升级 / 卸载

- **安装方式（`cloudime.iss`）**：
  - `[Tasks]` 两个互斥项（`Flags: exclusive`）：`recommended`（缺省选中）与 `full`（`unchecked`）；
  - `完整安装` 任务、系统目录的 `[Files]` / `[Run]` / `[Registry]` / `[UninstallRun]` /
    `[UninstallDelete]` 条目只在 `SystemDeploy` 定义（签名构建）时编译进来；
  - `{app}` 的 `regsvr32` 用 `Tasks: not full`，系统副本的 `regsvr32` 用 `Tasks: full`；
  - 任务页文字用 `[Messages]` 改成「选择安装方式」并说明两种方式的区别；
  - 未签名构建不定义 `SystemDeploy`，安装包不出现任务页，只有推荐安装。
- **安装模式记账**：完整安装写 `HKLM\Software\CloudIME\SystemDeploy=1`，推荐安装写 `0`；
  升级时先读旧值：
  - 旧 = 1、新 = 0（完整 → 推荐）：反注册并删除两处系统副本；
  - 旧 = 0、新 = 1（推荐 → 完整）：部署并注册系统副本；
  - 其他组合同现有流程。
- **升级**：DLL 正被每个应用进程加载，不能覆盖。系统目录与 `{app}` 两处的旧 DLL 都先改名
  `cloudime_tsf_x64.dll.old-<随机>`，复制新文件，`regsvr32` **系统副本**；旧文件删得掉就删，
  删不掉登记重启后删。不要对旧 DLL `regsvr32 /u`（会把整个 CLSID 注销）。
- **卸载**：`regsvr32 /u` 系统副本与 `{app}` 副本都试一次（幂等，谁注册的就注销谁）→
  删 `IME\CloudIME` 空目录 → 删 `InstallDir` / `SystemDeploy` 键 → 删 `{app}`；
  被占用的 DLL 同升级一样登记重启后删。
- **开发部署**：`apps/windows/scripts/push-local.ps1` 默认只装 `-InstallDir`（与现在一致）；
  加 `-SystemDir` 时把 x64/x86 DLL 同时复制进系统目录、用系统 `regsvr32` 注册**系统副本**、
  写 `InstallDir`（需要管理员）；实际覆盖/注册在 `reconfigure.ps1`，`-SystemDir` 要随提权
  重入一起传过去。`-SystemDir` 只用于签名版/签名后的 CS2 联调，未签名构建不要用它。
- 安装器与开发脚本都要处理“系统目录副本”的所有权：卸载/重装只动自己部署的那两处副本。

### 3.4 签名

- **证书是 P2 系统目录部署的前置条件**：没有 Certum 证书就不启用 `-SystemDeploy`，
  发布包继续只装 `{app}`，安装向导也不出现「完整安装」选项；
- 签名版的所有二进制（系统 DLL、`{app}` 兜底 DLL、Server、设置、安装包）一起签名、带时间戳；
- 开发用 `apps/windows/installer/sign-local.ps1` 的自签证书只能验证 uiAccess / 本机流程，
  **不能替代发版证书**（别的机器不信任，杀软也不认）；
- `apps/windows/installer/build.ps1` 在打包前签 DLL 与 Server，再交给 Inno；CI 的
  `-SystemDeploy` 开关与签名步骤绑定，签不了就不启用系统目录；
- uiAccess 的 Server 签名要求不变；P2 新增的是“系统目录里的 TSF DLL 也要有有效签名”。

### 3.5 CS2 / Dota 2

- **CS2**：按“系统目录 + 代码签名”验证；确认 `InprocServer32` 指向系统副本、
  `steamapps/.../trustedlaunch.cfg` 里没有拒绝记录、游戏内聊天能出候选窗。
- **Dota 2**：不做兼容名，保持无候选窗；在用户文档里写清原因（白名单按注册名全等匹配，
  改名等于全局冒用别家名字）。

## 4. 失败与恢复

| 场景 | 处理 |
|---|---|
| 宿主声明 UI-less 但根本不画 | `host_draws` 只在 `pbShow=FALSE` 时置位；声明了却没画属于宿主问题，我们按声明不弹（规范语义），不额外兜底 |
| 宿主读候选串但不画 | 只有命中 `host_drawn_candidates` 名单才收窗；名单外照常弹 |
| 元素回调里重入 | getter 只读本地快照，不做 IPC；`UpdateUIElement` 期间不持可变借用 |
| 动作同步 IPC 失败 | 断开并按现有退避重连；本轮按键不落文本，下一次 `Poll` 自愈 |
| 系统目录 DLL 被占用 | 两处副本都改名让路 + 重启后删；不 `regsvr32 /u` 旧文件 |
| 系统目录写入 / 注册失败 | 退回注册 `{app}` 兜底副本，输入法保持可用；安装日志写明 CS2 兼容未生效 |
| 没有代码签名证书 | 不启用系统目录部署，发布包只装 `{app}`；CS2 兼容等 Certum 证书 |
| 用户选了“推荐安装” | 不写系统目录；CS2 不可用（预期），其余功能完整 |
| “完整安装” 写入 / 注册失败 | 退回推荐布局（注册 `{app}` 副本），安装日志写明 |
| 签名版仍被杀软误报 | 发版前提交 Microsoft / 主流杀软申诉；保留 `{app}` 兜底安装路径 |
| 自签证书 CS2 不认 | 开发环境先用真证书验证；发版必须 Certum |
| 老 DLL / 新 Server | 协议 v24，两端同源；安装器与 `push-local` 一起换 |

## 5. 验收

P1：

1. 本地 UI-less 测试宿主：不弹服务端候选窗；宿主打印的候选、页、选中与本地一致；
   `SetSelection` / `Finalize` / `Abort` 都生效；编码仍在组合区；
2. 回归：Notepad / Chromium / QQ 不创建 UIElement、不走新消息、候选窗行为不变；
3. 单元测试：协议往返、状态机（声明/读取/名单）、抑制判定、强制 inline、快照 getter；
4. 真机：先测试宿主，再上 UE 游戏（黑神话 / 逸剑风云决）；Civ VI 仍按 P0 行为（IMM32，不在 UI-less 范围）。

P2：

1. **签名门槛**：未签名构建只装 `{app}`，不写系统目录、不注册系统副本，安装向导不出现安装方式选择；
2. **安装方式**（签名版）：向导出现「推荐安装 / 完整安装」两个互斥选项，缺省推荐；
   - 推荐：只写 `{app}`，系统目录无文件、`InprocServer32` 指向 `{app}`；
   - 完整：DLL 同时落在 `{app}` 与 System32 / SysWOW64，`InprocServer32` 指向系统副本，
     Server 从 Program Files 启动；两处文件签名一致；
3. **模式切换**：完整 → 推荐卸载系统副本并改注册 `{app}`；推荐 → 完整补装并注册系统副本；
   `SystemDeploy` 记账正确；
4. 兜底路径：人为让系统目录注册失败时，安装器能退回注册 `{app}` 副本，输入法仍完整可用
   （CS2 不保证，安装日志里写明）；
5. 升级 / 卸载：两处旧 DLL 都改名让路、重启后删；卸载后系统目录与 `{app}` 都无残留；
6. 开发 `push-local` 默认只装 `{app}`；`-SystemDir` 只在签名后的 CS2 联调用；
7. CS2：`trustedlaunch.cfg` 无拒绝、游戏内可输入中文且有候选窗。

## 6. 涉及文件

P1：

- `apps/windows/tsf/src/com/uielement/`（新：`mod.rs`、`element.rs`、`state.rs`）；
- `apps/windows/tsf/src/com/service/processor.rs`（`ITfTextInputProcessorEx`）、
  `com/service/key_sink.rs` / `document.rs`（写帧缓存）、`com/composition/shared.rs`（`last_frame`）；
- `apps/windows/server/src/dispatch/{candidates,composed,status_tip}/mod.rs`、
  `dispatch/session/info.rs`、`dispatch/message.rs`；
- `crates/cloudime-platform/src/protocol/{client,server,mod}.rs`（v24）；
- `crates/cloudime-platform/src/config/candidate.rs`（`host_drawn_candidates`）；
- `apps/windows/tools/cloudime-uiless-host/`（新 crate）。

P2：

- `apps/windows/installer/cloudime.iss`（`[Tasks]` 推荐/完整、条件编译的系统目录
  `[Files]` / `[Run]` / `[Registry]` / `[Uninstall*]`、`SystemDeploy` 记账、模式切换、
  `[Messages]` 的安装方式说明）、`build.ps1`（`-SystemDeploy` 与签名绑定）、`README.md`；
- `apps/windows/scripts/push-local.ps1`、`reconfigure.ps1`（默认 `{app}`，`-SystemDir`
  用于签名后联调；安装目录反查先读 `InstallDir`）；
- `apps/windows/tsf/src/com/service/launch.rs`（InstallDir）；
- `apps/windows/installer/sign-local.ps1`（系统 DLL 签名）；
- `docs/notes/release.md`（签名版才开 `-SystemDeploy`、安装方式选择）；
- `docs/user/getting-started/install.md`（推荐 / 完整安装的区别）、
  `docs/user/help/uninstall.md`（系统目录副本的卸载）。

## 7. 后续（不在本轮）

- `ITfIntegratableCandidateListUIElement`（Win8+ 搜索框）；
- 非 UI-less 独占全屏的内联候选（P3）；
- Dota 2 白名单申请（向 Valve 提交）；
- Valorant / LoL 等内核反作弊的 per-app 策略。
