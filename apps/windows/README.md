# 云朵 Windows 输入法

Windows 端是**一个产品、两个产物**，各自一个 package，同放本目录：

| 目录 | package | 产物 | 职责 |
| --- | --- | --- | --- |
| `server/` | `cloudime-windows-server` | `cloudime-server.exe` | 持有唯一的输入内核 `cloudime-core::Engine`，跑在所有应用进程之外 |
| `tsf/` | `cloudime-windows-tsf` | `cloudime_tsf_x64.dll` / `cloudime_tsf_x86.dll` | TSF 文本服务，被加载进每个应用进程，只做按键转发与文档写入（候选窗口由 Server 自绘） |

```
应用进程 A ── cloudime_tsf_x64.dll ─┐
应用进程 B ── cloudime_tsf_x64.dll ─┼─ 命名管道 \\.\pipe\cloudime ─▶ cloudime-server（唯一的 Engine）
应用进程 C ── cloudime_tsf_x64.dll ─┘
```

## 为什么核心逻辑要在进程外

Windows 的文本服务（TSF，Text Services Framework）是一个 COM DLL（`ITfTextInputProcessor`），
系统会把它加载进**每一个**接受文本输入的应用进程。因此输入内核不能待在 DLL 里（会被复制进几十个进程、
状态无法共享、崩溃会连累宿主应用）。云朵输入法照 Weasel（WeaselServer + WeaselTSF）、水杉（Server 进程）的做法：
Engine 只此一份，跑在独立的 Server 进程；每个应用进程里的 TSF DLL 只做两件事——把系统按键翻成协议消息发来、
把 Server 回的候选画到候选窗口。

## 为什么是两个 package 而不是一个

两个产物的依赖集合刻意不同：DLL 只依赖 `cloudime-core`、`cloudime-platform` 与官方 `windows` crate（COM
`implement` 宏），Server 才依赖词库 / 学习 / 语言模型 / 本地整句模型整棵树。合成一个 package 后，DLL 的
编译单元会拉进 Server 的依赖；用 feature 区分也不行，workspace 一起构建时 feature 会统一。crate 边界就是
「DLL 不含 Engine」这条约束的强制手段。判断标准：换掉平台适配层，不应该需要改 Core 的任何一行。

## 三个部分

- **Server 进程**（`server/`）：装配并持有 Engine（词库 / 语言模型 / 学习），按 `SessionId` 为每个
  应用会话维护各自的组句状态，处理按键、产出候选与上屏文本，把本地整句模型的异步结果主动推给
  对应会话。
- **IPC 协议**（`cloudime-platform::protocol`，两端共用）：`ClientMessage`（DLL → Server：开 / 关会话（带宿主 exe 名，记进输入日志）、按键、上屏、回上下文）与 `ServerMessage`（Server → DLL：按键结果、上屏结果、异步重绘、请求上下文），一次要绘制的状态是
  `Frame`（preedit 分段 + 候选页）。长度前缀 JSON 帧的编解码与缺省管道名也在这里，DLL 不必依赖整个 Server 库。
- **TSF DLL**（`tsf/`）：分「引擎层」`client`（平台无关的管道客户端 `EngineClient`，泛型在任意 `Read + Write`
  上，本机就能接真 Server 端到端测）与「COM 层」`com`（`cfg(windows)`：`DllGetClassObject` → `IClassFactory`
  → `#[implement(ITfTextInputProcessor, ITfKeyEventSink)]`，编辑会话上屏，把组句位置报给 Server 摆候选窗口，
  `DllRegisterServer` 注册文本服务）。

设计细节见 `docs/design/architecture.md`「Windows：TSF」。

## 构建

非 Windows 的开发机只做交叉 `check`，产出不了可用二进制，但协议层的端到端测试能跑：

```bash
cargo check --target x86_64-pc-windows-gnu -p cloudime-windows-server -p cloudime-windows-tsf
cargo test -p cloudime-windows-server -p cloudime-windows-tsf
```

真正编译与试用都在 Windows 机器上（MSVC 工具链）。

### 本地联调：`scripts/test-local.ps1`

改了 Server 或 DLL 之后要试手，用这个脚本，别手敲：

```powershell
pwsh -File apps\windows\scripts\test-local.ps1            # 编译 + 换 Server + 起，DLL 那步要管理员
pwsh -File apps\windows\scripts\test-local.ps1 -SkipBuild -SkipRegister
```

它做四件事：编 Server 与 TSF DLL（带 `CLOUDIME_UIACCESS=0`，没签名的 exe 带 uiAccess 起不来）、
停掉在跑的 Server、把**已安装目录整份拷到 `~\cloudime-devtest`** 再换上新的 Server 与随包数据、
起 Server 并打印剩下要手动做的事（注册 DLL、设方案、敲哪几组键）。

> **脚本存成带 BOM 的 UTF-8**（`sign-local.ps1`、`reconfigure.ps1` 也是）。Windows PowerShell 5.1 对没有 BOM 的 `.ps1`
> 按系统 ANSI 码页解析，中文会变成乱码、引号配对跟着崩，报出来的却是「字符串缺少终止符」这种语法错。
> 编辑时别把 BOM 去掉。PS7 两种都读得对，所以只用 `pwsh` 验会漏掉这个问题。

**为什么不在仓库里直接跑**：`bundled_root()` 按 exe 位置找数据，`target\debug\` 下会落到仓库根，
而 `data\generated\` 是 gitignore 的、本机多半没有，于是退回 `assets\sample\` 样例——只有几百条词，
与装好后的候选差得很远。

**为什么两边要一起换**：`cloudime_core::Candidate` 是线上格式的一部分（见 `protocol/mod.rs`）。
Core 加一个 `CandidateKind` 变体，老 DLL 就解不出整条帧、把按键原样放行——表现是「输入法突然只出英文」，
日志里只有一句 `unknown variant`。所以 Server 与 DLL 必须同一份源码编出来的；协议版本号对不上时
Server 会记警告，但**它只警告、不拒绝**，别指望它兜住。

### 手工步骤（脚本里也在做，供对照）

```bat
:: 1) 编译出 DLL 与 Server
cargo build -p cloudime-windows-tsf -p cloudime-windows-server

:: 2) 注册文本服务（改 HKEY_CLASSES_ROOT，图标写到 %ProgramData%\CloudIME\cloudime.ico，要管理员）
regsvr32 target\debug\cloudime_tsf.dll

:: 3) 起 Server（引擎在这里；没起时 DLL 吃掉字母键但没有候选，起来后下一键 / 下次聚焦自动重连）
cargo run -p cloudime-windows-server

:: 4) 在系统「语言 / 输入法」里应能看到「云朵输入法」，切到它，在任意输入框敲字
::    日志都在 %LOCALAPPDATA%\CloudIME\logs\：server.<日期>.log / tsf.<日期>.log / settings.<日期>.log（按天，留 7 天）

:: 反注册
regsvr32 /u target\debug\cloudime_tsf.dll
```

### 覆盖程序文件并重配：`scripts/reconfigure.ps1`

把本机这一版用起来：先停掉在跑的 Server / 设置程序，从 `target\release` 与 `target\i686-pc-windows-msvc\release`
把 Server、两份 TSF DLL、设置程序与状态栏图标覆盖进安装目录，再重注册 / 刷新，**免得为生效去注销或重启电脑**。
覆盖用的是现成产物，所以请先用 `CLOUDIME_UIACCESS=0` 编好 release（`push-local.ps1` 干这件事）；
本脚本不代替安装包：随包数据（词库、模型）或版本号变了照旧跑安装包，覆盖安装之后也可以用它收尾。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File apps\windows\scripts\reconfigure.ps1
powershell -File apps\windows\scripts\reconfigure.ps1 -SkipCopy                 # 不动程序文件，只重注册 / 刷新
powershell -File apps\windows\scripts\reconfigure.ps1 -SkipSettings             # 覆盖时不动设置程序
powershell -File apps\windows\scripts\reconfigure.ps1 -SkipExplorer             # 不重启 Explorer（开着的文件夹窗口会关掉）
powershell -File apps\windows\scripts\reconfigure.ps1 -RestartApps notepad,edge  # 点名关掉重开；`*` = 所有还加载着旧 DLL 的
powershell -File apps\windows\scripts\reconfigure.ps1 -NoElevate                 # 不弹 UAC，跳过管理员步骤并打印手工命令
```

拆成两个阶段，各自要的身份不同（文件头有完整说明）：

- **阶段 1（管理员）**：停掉在跑的 Server / 设置程序，覆盖 `cloudime-server.exe`、两份 `cloudime_tsf-<版本>[-x86].dll`、
  `cloudime-settings.exe` 与 `data\icons*`（被映射 / 被占用的文件先改名让开再写新文件），随后重注册 64 / 32 位 TSF DLL 并核对
  注册表指向与 profile、补 `ALL APPLICATION PACKAGES` 权限、查登录自启快捷方式与随包词库、清 `cloudime-server.old-*` 与旧 DLL 残留。
- **阶段 2（普通权限）**：停掉在跑的 Server 换新、重启 `ctfmon` 与输入切换器（输入法列表不刷新的免注销处理）、
  重启 Explorer（任务栏搜索 / 资源管理器加载着旧 DLL）、ShellExecute 起新 Server、扫描还加载着旧 DLL 的
  应用（点名才动手）、核对当天 Server 日志有没有协议 / 词库告警。

安装目录缺省从注册表 `InprocServer32` 反查（系统盘不一定是 `C:`），再退到自启快捷方式与
`%ProgramFiles%\CloudIME`，也可 `-InstallDir` 显式指定。普通窗口跑会为阶段 1 弹一次 UAC；管理员窗口跑会借
Explorer 另起一个普通权限窗口跑阶段 2（关着 UAC 的机器没有普通 / 提升之分，两段都就地跑）。

脚本管不了的两条（见 `docs/user/help/feedback.md`）：覆盖安装前就开着的应用要关掉重开才用上新版；
Win+Space 里要是还看不到云朵，注销重登一次。

### 编译 + 覆盖一条命令：`scripts/push-local.ps1`

`reconfigure.ps1` 用的是 `target\release` 里现成的产物；`push-local.ps1` 负责先把它编出来（带 `CLOUDIME_UIACCESS=0`）
再调它，所以「改代码 → 本机生效」是一条命令：

```powershell
pwsh -File apps\windows\scripts\push-local.ps1                    # 编译 release + 覆盖 + 刷新（覆盖那步弹一次 UAC）
pwsh -File apps\windows\scripts\push-local.ps1 -SkipBuild         # 已经编好了，只覆盖与刷新
pwsh -File apps\windows\scripts\push-local.ps1 -SkipSettings      # 设置程序没改，不编也不拷
pwsh -File apps\windows\scripts\push-local.ps1 -SkipExplorer      # 刷新时不重启 Explorer
```

两条要留意：

- **不能直接 `Copy-Item` 覆盖**：Explorer 与各应用映射着 TSF DLL，Server / 设置程序也占着 exe，Windows 不许写这些文件。
  `reconfigure.ps1` 的覆盖步骤照安装器的套路先改名让开（`*.old-<时间>.*`）再写新的。
- **协议版本要两边同源**：Server 与 DLL 必须是同一次编译的产物（这一步保证），别把别处编的 DLL 混进来。

## 设置程序与 Windows App Runtime

`settings/`（`cloudime-settings.exe`）用 Windows Reactor（WinUI 3）画界面，是三个产物里唯一依赖 Windows App Runtime 的。
它的部署方式是**自包含**：`build.rs` 让 `windows-reactor-setup` 把 `Microsoft.WindowsAppSDK.Runtime` 的 MSIX 解到
`target\release\` 并按自包含标记嵌清单，安装包把这些文件装到 exe 同级——不依赖机器上装没装框架包。
Windows 10 上框架依赖的引导走不通（它要先调 Windows 11 才有的 `TryCreatePackageDependency`），
同一个 `build.rs` 还把这两个 API 改成延迟加载：否则它们会进 exe 的导入表，Windows 10 在加载期就起不来。
定位过程、上游 issue / PR 与取舍见 `docs/notes/windows-win10.md`；打包侧见 `apps/windows/installer/README.md`。

## 版本与发布

壳与 Server 的版本号各自写死（见 `docs/notes/release.md`）：`apps/windows/{server,tsf,settings}/Cargo.toml` 三个一起改，
不跟 workspace 走；发布标签用 `windows-v<版本>`。
