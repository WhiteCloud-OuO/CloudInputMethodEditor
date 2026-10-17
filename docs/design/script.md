# 用户脚本

2026-10-07 定。规则先立在这里，实现要点在 `docs/notes/crate-notes.md`（`crates/cloudime-script` 与
`apps/windows` 的「用户脚本」两节）。

## 目标与边界

- 用户能用 Lua 脚本改输入法的行为：文本扩展、按词加权 / 降权、给候选加显示、贴在线翻译、游戏里放行按键。
- 脚本按「用户自己写的本机程序」对待：给的是 **mlua 的「安全子集」标准库**（`io` / `os` / `package` 都在，
  `debug` 没有）+ 下面几处刻意改动 —— **风险由用户自担**；输入法保证「脚本出错不影响输入」与那几道闸门。
  **刻意的例外**（2026-10-07，稳定性优先）：`os.exit` 换掉、`os.execute` 不等子进程、
  `io.popen` 与 `coroutine.create` / `wrap` 报错、`io.stdin` 关掉。
- 引擎跑在** Server 进程**里（`crates/cloudime-script`，mlua + LuaJIT，源码随包编）。
  **不进 TSF DLL**：DLL 被加载进每个应用、脚本 JIT 在开了 ACG 的宿主里会失败、脚本崩了会连累宿主应用。

## 脚本清单（manifest）：每个脚本必须声明

文件**最前面**、加载期调用一次 `cloudime.script{…}`。**缺清单、字段缺失 / 类型不对、`api` 不认识、
`sync` 与 `handover` 对不上 —— 这个脚本判无效**（跳过 + 日志写明原因，其余脚本照常）。

```lua
cloudime.script{
    name     = "我的文本扩展",      -- 日志里点名用；缺省用文件名
    api      = 2,                   -- 面向哪版脚本 API；不认识的版本判无效
    -- 必写：由什么触发（五选一，见下表）
    trigger_condition = "combination_key",
    combination_modifiers = "ctrl+alt",  -- combination_key 必写：要哪一套修饰键（含 "none" = 不按修饰键）
    keys     = { "j" },             -- combination_key 必写：具体按键（字母 / 数字 / 标点 / enter / f1…）
    -- time_range = { from = "20:00", to = "06:00" },  -- time_range 必写（跨夜也行）
    -- candidate_pinyin = "nihao",  -- candidate_context 必写（比较时忽略 ' 与大小写）
    -- apps = { "Code.exe" },       -- focus_app 必写；其它触发条件下是可选的应用过滤
    budget   = 200000,              -- 单次调用的指令数上限（只能 ≤ 全局 1 亿，不能放宽）
    timeout  = 50,                  -- 单次调用的墙钟上限（毫秒）
    needs_context = false,          -- 可选：每次派发前刷新 cloudime.context（按键前多读一次光标前文）
    sync     = true,                -- 必写：处理函数当场算完就返回（false = 只用异步回调）
    handover = "return",            -- 必写：必须与 sync 一致（true↔return、false↔callback）
    on_error = function(event, message) end,  -- 必写：错误处理回调；写 false = 不处理
    priority = 10,                  -- 多个脚本都改同一项时大的盖小的；缺省 0、同值按文件名
}
```

| 字段 | 必写 | 语义 |
|---|---|---|
| `trigger_condition` | **是** | 五选一：`"combination_key"` 按下组合键（收 `key`）/ `"time_range"` 某个时间段（收 `time`，每段只触发一次）/ `"candidate_context"` 候选窗口出现且拼音串等于指定内容（收 `candidates`）/ `"focus_app"` 编辑光标位于指定程序（收 `focus`）/ `"manual"` 手动触发（i 模式预留，永不自动派发）；写别的判无效 |
| `combination_modifiers` | `combination_key` 时**是** | 要哪一套修饰键：`"none"`（不按修饰键，单键）/ `"ctrl"` / `"ctrl+alt"` / `"ctrl+shift"` / `"alt+shift"` / `"alt"` / `"win+ctrl"` / `"win+alt"` / `"win+shift"` / `"win+alt+ctrl"` / `"win+ctrl+shift"`；**精确匹配**，多按一个修饰键就不算 |
| `keys` | `combination_key` 时**是** | 要哪几个键：`"enter"` / `"tab"` / `"space"` / `"backspace"` / `"delete"` / `"esc"` / `"left"` / `"up"` / `"right"` / `"down"` / `"home"` / `"end"` / `"pageup"` / `"pagedown"` / `"f1"`…`"f12"`，或单个字母 / 数字 / 标点（`"a"`–`"z"`、`"0"`–`"9"`、`` "`" `` `"-"` `"="` `"["` `"]"` `"\\"` `";"` `"'"` `","` `"."` `"/"`）。可写多个、自动去重；有一个不认识判无效 |
| `time_range` | `time_range` 时**是** | `{ from = "20:00", to = "06:00" }`（本地时间，左闭右开；`from > to` 表示跨夜）。**每个时间段窗口只触发一次**（例如 20:00 开的那一晚到次日 06:00 只派一次） |
| `candidate_pinyin` | `candidate_context` 时**是** | 候选窗口出现且拼音串等于它才触发；比较时去掉 `'` 与空白、忽略大小写（`Ni'Hao` 与 `nihao` 相同） |
| `name` | 否 | 日志（以后还有设置页）里点名；缺省 = 文件名 |
| `api` | **是** | 脚本面向的 API 版本，现在只有 `2`。不认识 → 无效（`1` 是触发条件改版前的旧版，已不再接受） |
| `budget` | 否 | 单次调用允许的**指令数**上限；缺省 = 全局 1 亿。**只能收窄不能放宽**。超了中止这一次 —— 这一次当没做（脚本的全局变量可能只改了一半，脚本自己要能接受） |
| `timeout` | **是** | 单次调用允许的**墙钟**上限（毫秒）：同步处理函数与异步回调都按它算，到点中止这一次 |
| `needs_context` | 否 | `true` = 每次派发前刷新 `cloudime.context`（DLL 会在按键前多读一次光标前文）。读前文的脚本（自动序号之类）要写它 |
| `apps` | 否（`focus_app` 时**是**） | 只在哪些应用里跑（**exe 文件名，大小写不敏感**，与「不显示候选框的程序名单」同一套口径）。空 / 不写 = 所有应用。**`startup` 与异步回调不按它过滤**：前者早于任何应用，后者在请求发出时已经判过 |
| `sync` | **是** | `true` = 同步：处理函数当场算完就返回；`false` = 只用 `cloudime.http_get` 的异步回调 |
| `handover` | **是** | `"return"` = 算完返回就交回控制权；`"callback"` = 回调返回时交回。**必须与 `sync` 一致**，对不上判无效 |
| `on_error` | **是** | 形式为 `function(event, message)` 的函数：这个脚本的处理函数出错时调它（`event` 是事件名、`message` 是错误）。写 `false` = 不处理，运行时**直接中止这一次**、把控制权交回 Server |
| `priority` | 否 | 多个脚本都要改同一项时的合并顺序：大的盖小的；同值按文件名 |

**铁律**：Server **从不等待脚本** —— 处理函数返回（或被中止）的那一刻，控制权就回到 Server。

### 其余硬规则

- **一个文件一个脚本、UTF-8 编码**，清单必须在最前面（在清单之前调 `cloudime.on` 也算无效）。
- **不许改 `cloudime` 表**（及其上的宿主函数）：运行时把它锁成**只读代理**（`__index` 指向本体、`__newindex` 报错）。
  `rawset` 能绕过这道锁，但那只**改到脚本自己看到的那一份** —— 运行时手里握着函数本体，脚本怎么改都影响不到它。
- **持久状态写数据目录**（`%APPDATA%\CloudIME` 下，比如 `scripts\<脚本名>.data`），
  **别写安装目录**：普通用户写不进去，升级 / 卸载还会删。
- **不许长时间占住工人线程**：允许的「等」只有两个**异步** API —— `cloudime.http_get` 与
  `cloudime.http_post`（都在自己的线程上跑、时限必给）；同步处理函数里自己等 = 卡住所有应用。
- 加载期违规只进日志（「哪个脚本无效、为什么」）—— 以后设置页的「脚本」页要把它显示出来。

## 稳定性闸门（脚本卡住堵的是「所有应用」）

Server 是一个进程、一条工人线程服务所有应用，所以脚本卡住不是「某一个应用卡」，而是**全都不打字**。
四道闸：

| 闸 | 做法 | 防住什么 |
|---|---|---|
| **指令预算 + 墙钟** | 每次调用前按**该脚本清单**里的 `budget`（缺省 = 全局 1 亿条，只能收窄）与 `timeout`（必写，毫秒）上紧限额（加载期用全局上限与 5 秒）；mlua 的钩子每 1 万条指令醒一次，超了就从钩子里报错中止这一次 | 写错的死循环（`while true do end`）与跑太久的脚本。中止只毁这一次：记一条日志、调它的 `on_error`，按键照旧走引擎 |
| **脚本的 JIT 关掉** | LuaJIT 编成 trace 之后就**不再检查调试钩子**（实测 `while true do end` 会真的跑飞），所以 `jit.off()`、并把 `jit.on` 收掉 | 「热循环被 JIT 编走、预算钩子失效」 |
| **没有 `debug` 库** | mlua 的 `Lua::new()` 只开**安全子集**：脚本连 `debug.sethook` 都没有 | 脚本主动 / 无意摘掉预算钩子 |
| **不给协程** | `coroutine.create` / `coroutine.wrap` 直接报错（Lua 的钩子每个线程一份，协程里的循环预算拦不住） | 「把死循环藏在协程里绕过预算」 |
| **关掉标准输入** | `io.stdin = nil` | `io.read()` 在 Server 由控制台启动时一直等输入，把工人线程堵死 |
| **换掉会退进程 / 会卡住的入口** | `os.exit` → 只记日志；`os.execute` → 走 `cloudime.run`（**启动就返回、不等子进程**）；`io.popen` → 报错并指向 `cloudime.http_get` | 脚本退掉整个 Server（所有应用断线几秒）；同步等子进程把工人线程堵死 |

`cloudime.run` 与 `os.execute` 保留「启动可执行文件」的能力（例：`os.execute('notepad.exe')`），只是**不等它**。

**不再做的：看门狗**（单独线程盯工人线程、超时重启进程）。它确实能兜住「任何卡住」，但代价是**用户会看到
输入法闪一下、正在打的句子还丢**；相比之下「按上面的清单把能卡住的入口逐个堵掉」更干净。于是做过的取舍是：
**宁可少一样能力（协程 / 管道读）也不让进程重启**。

**残余（明知没堵的那两条）**：`io.open` 打开**设备 / 管道路径**（`\\.\pipe\…`、`COM1:`）与**慢速网络盘**上的
读写仍可能长时间阻塞 —— 这两条没法在 Lua 层便宜地堵，而且路径是脚本自己写死的（「风险自担」那一档）。
**「预算拦不住的洞」已经没有了**：脚本能跑的代码全在装了钩子的主状态上。

## 规则一：没有任何脚本 → 走老路

「没有任何脚本」= 安装目录的 `Scripts\` 不存在，或里面一个真脚本都没有（`template.lua` 不算 —— 它是「新建脚本」的模板，加载器按文件名跳过）。
这时**按键路径上一点额外工作都没有**：

| 位置 | 没脚本时 |
|---|---|
| 每次按键 | `has_handlers("key")` 一次查表就返回，**不拼载荷、不调 Lua** |
| 候选排完 | `has_handlers("candidates")` 为假，**不拼载荷、不留权重** |
| 每一拍 tick | 没有挂着的 HTTP 请求就直接返回（一次布尔判断） |
| 启动 | 没人关心 `startup` 就不派发 |

行为与脚本功能存在之前**逐字节一致**：同一份源码、同一套排序、同一个帧、同一个上屏。

唯一的残留是进程里有一个**空的 Lua 状态**（Server 启动时就建，约百来 KB；只有一个进程，不是每个应用一份）。
要连它都不建（惰性创建）是下一步的优化，见文末。

## 规则二：有脚本 → 在固定四个位置运行

脚本在**加载时**执行一遍（按文件名排序），用 `cloudime.on(事件名, 处理函数)` 登记；之后由 Server 在
下面几处派发。每个位置为什么在那儿，与它能改什么：

| 时机 | 位置 | 脚本能做什么 |
|---|---|---|
| `startup` | 全部脚本加载完 | 只做自己的准备（开文件、发首个请求）；返回值按「没有按键的那一拍」处理 |
| `key` | 一次按键、**引擎处理之前** | 只有**组合键对得上清单**（`combination_modifiers` + `keys`）的脚本才收 —— 组句与否都一样；看这一键与当时状态（载荷里有 `vk` / `ctrl` / `window` 与高亮候选文本 `highlight`）；`passthrough`（不吃、交给应用）、`commit`（吃掉并上屏）、`notice`、`adjust`、`order` / `display`（下一屏生效）、`online`（候选窗底部那一行；`Ctrl + 反引号` 会把它当一条译文上屏，见 `docs/design/online-translate.md`）、`theme`（换主题：写进配置 `[theme] curr_theme` 并重启输入法服务，`false` 回本进程启动时那份） |
| `candidates` | **引擎排完之后**（`recompose`） | 只有**拼音串等于清单 `candidate_pinyin`** 的脚本才收；看这一屏候选（含 `pinyin` 与 Core 的 `weight`）；`order`（临时接管顺序）、`display`（改显示）、`notice`、`adjust`、`online`、`theme`；`passthrough` / `commit` 不认 |
| `focus` | 焦点**切进**清单 `apps` 里的某个程序（前台 DLL 的 `SyncMode` 与首次按键都会更新焦点） | 同「没有按键的那一拍」：`notice` / `adjust` / `online` / `theme`；载荷 `{ app }`。同一程序里换输入框不重复派 |
| `caret` | 脚本调过 `cloudime.caret()`，DLL 量到插入点之后（`ClientMessage::Caret`） | 同「没有按键的那一拍」；载荷是插入点矩形 + 所在显示器工作区（屏幕坐标 / **点**）。不受 `trigger_condition` 限制 |
| 异步回调 | 结果回来那一拍 `tick`（`http_get` / `http_post`） | 同 `candidates`：`order` / `display` / `notice` / `adjust` / `online` / `theme`；`passthrough` / `commit` 不认 |
| `time` | 本地时间**第一次**落进清单 `time_range` 那个时间段（跨夜按「昨晚开的窗口」算） | 同「没有按键的那一拍」：`notice` / `adjust` / `online` / `theme`；载荷同 `cloudime.get_time()` 另加 `app`。**每个时间段窗口只派一次** |

- 多个脚本按登记顺序合并，**后面的盖前面的**；返回值类型严格匹配（写错记一条日志当没写）。
- **只有触发条件对上的事件才派**：清单的 `trigger_condition` 决定收哪种（`combination_key` → `key`、
  `candidate_context` → `candidates`、`focus_app` → `focus`、`time_range` → `time`；`manual` 一个都不收），
  `startup` / `window` / HTTP 回调不受它限制。
- **组合键不受候选窗口开没开影响**：清单的键折进下发给 DLL 的 `InputSettings` —— 没组句时 DLL 也在
  TSF 的按键路径上先问一趟 Server（`script_keys` 位图 + `script_key_modifiers` 修饰键集合），脚本不吃就
  由 DLL 重放回应用。这就是「像输入法自己的简繁切换那样」的那条路，不用注册系统级全局热键。
- **时间段只触发一次**：一个 `time_range` 窗口（例如 20:00 到次日 06:00）里每个脚本只派一次 `time`；
  服务重启后重新算一次（状态不落盘）。
- **清单说了算**：`apps` 不符的脚本在这一拍不调（`startup` / HTTP 回调不按它过滤）、`priority` 大的排在后面
  （于是盖住前面的）、`budget` / `timeout` 按各自清单收窄（见「脚本清单」）。
- **载荷里带 `app`**（宿主 exe 名）：`key`、`candidates` 都有，脚本据此按应用分支（「在游戏里放行按键、
  在编辑器里照旧」）。HTTP 回调拿到的第一个参数是「发请求时那个应用」拿不到 —— 它在请求时由脚本自己记着
  （闭包捕获 `event.app`），回调的第二个参数带的是**那一刻**的候选表与当时的 `app`。
- **`cloudime.candidate.redraw()`**：请 Server 把当前这一屏**重算一遍再重画**（会重新派发 `candidates`
  事件）—— 异步回调里改了自己的状态之后用它；在 `candidates` 事件里调没意义（那一拍本来就在重算），
  Server 会忽略掉。
- **候选窗尺寸**（`cloudime.candidate.*`）：`set_min_width(点)`（窗口的宽度下限，竖排、横排都生效；**100% 缩放下 1 点 = 1 像素**，
  渲染时会随 DPI 与缩放一起放大）、`set_page_size(5–9)`（一页几个，直接改窗口高度）、`set_scale(倍数)`
  （整个窗口按倍数缩放，等同 `Ctrl + 滚轮`）。**只在本次组句内有效**：组句结束回配置 / 滚轮的值，
  与 `Ctrl + 滚轮` 同级；三项都可以不带参数（`nil`）单独恢复默认。越界会被夹取（宽度 0–2000、
  一页 5–9、缩放 0.58–2.99）；用户一滚滚轮就由滚轮接管缩放（脚本那边再设一次又会盖回来）。
- **量文字**：`cloudime.ui.measure(文本, 字体名?)` → `{ width = 点, height = 点 }`；字体名可选
  `pinyin` / `candidate`（缺省）/ `item_number` / `translate`。`cloudime.ui.measure_tip(文本)` 是
  「底部那一行（翻译 Tip / 在线译文）」那个字体的捷径。量出来的**点**可以直接喂 `set_min_width`
  （同一个口径，装得下）。它走 Server 自己那份渲染器（懒加载、与候选窗同源的字族 / 字号），
  所以和画出来的一致；量的是「点」——不随滚轮缩放变，这正是「要多宽才装得下」要的那个数。
- **截断 / 折行**：`cloudime.ui.truncate(文本, 行宽 [, 选项])` → `{ text, lines, truncated, width }`；
  行宽与上面同一个口径（点）。选项表 `{ mode = "ellipsis"（缺省）| "wrap", max_lines = 5, font = … }`：
  `"ellipsis"` 单行放不下就从尾部去字补「…」；`"wrap"` 按行宽贪心折行（优先在空白处断，中文这类没有
  空白的就从任意处断），最多 `max_lines` 行，剩下的在末行补「…」。`text` 是最终文本（折行时里面带换行），
  `width` 是最宽一行的宽度、`truncated` 说明有没有截断过。**`online` 那一行支持多行**：把带换行的 `text`
  写进去就按多行显示（渲染器按 `\n` 分行、窗口随之变高）。
- **问候选窗有多大**：`cloudime.candidate.width()` → 候选窗**内容区宽度**（点）；不在组句 / 还没画过时是 `nil`。
  折行就可以按它来（`online` 那一行左右各有 8 点内边距，再扣掉前缀宽度）。它是 UI 线程最近一次画出来的尺寸，
  由 Server 在每次派发前刷给脚本 —— 和 `cloudime.context` 同一个时机，看到的是「这一刻窗口多大」。
- **读输入框文本**：`cloudime.text.all(上限?)` / `before(上限?)` / `after(上限?)` → `{ text, truncated }`；
  拿不到（刚启动 / 换输入框后的第一次调用 / 私密框 / 那个控件不支持）给 `nil`。上限按**显示宽度**
  （中文 / 全角 2、西文 1），缺省 2^64-1；超上限时留**光标附近**（`before` 丢头、`after` 丢尾、
  `all` 前一半 + 后一半）。整篇由 DLL 在**每段组句起始**读一次（Server 用 `ModeSync` 的 `want_document`
  一直请 —— 有脚本就请，没脚本永远不读）；快照**组句结束不清**，只在换焦点 / 会话或转私密时清，
  下一段组句起始会把新的一份送上来。**有的宿主只给「局部上下文」**（Windows 11 记事本实测：TSF 上下文
  按选区 / 组句圈成局部），这类宿主里读不到整篇、一律 `nil`；「绕开 TSF 从窗口层读」那条路暂时打住，
  见 `docs/notes/crate-notes.md` 的「读输入框文本」。
  **组合键脚本**一般不在组句里，整篇那条路走不到它们 —— 清单写了 `needs_context = true` 时，
  DLL 在**没组句时的每个按键之前**开一个**异步只读编辑会话**现读一份**光标前文**送上来
  （`InputSettings.script_wants_text` 为真才做）：读到的是**上一次按键之后**的文档，正好是这次按键需要的。
  这份前文进 `set_surrounding`、也就是脚本看到的 **`cloudime.context`**（不是 `cloudime.text.*`
  那份整篇快照），自动序号就靠它。
- **剪贴板**：`cloudime.clipboard.settext(文本)` 写、`cloudime.clipboard.gettext()` 读（→ 字符串 / `nil`）。
  实现在 Server 侧（Windows 剪贴板 API，被别的程序占着时短等重试、仍不行报错），不经 DLL。
  `gettext` 读得到任何程序放进剪贴板的文本（含密码管理器）—— 用户自担，文档里写明。
- **光标位置**：`cloudime.caret()` 是**请求**（没有返回值）：Server 在下一拍的 `KeyResult.want_caret`
  里请 DLL 量一次插入点（异步只读编辑会话，`com/edit/caret.rs`；拿不到退到鼠标位置），DLL 用
  `ClientMessage::Caret` 送回来，Server 派 `caret` 事件。载荷是插入点矩形与所在显示器工作区
  （屏幕坐标 / **点**）：`{ left, top, right, bottom, work_left, work_top, work_right,
  work_bottom, scale }`。在 `key` 处理函数里调用时量的是这一键处理完之后的位置。
- **安装目录**：`cloudime.install_dir()` → 安装目录绝对路径；`cloudime.install_path("Images/SVG/…")`
  → 目录里文件的绝对路径（`canonicalize` 后必须仍在安装目录内，越界报错、不存在给 `nil`）。
  `svg` 元素因此可以写 `path` 直接读文件（见元素表），不必让脚本把 SVG 文本抄进 Lua。
  `Runtime::load` 从脚本目录的上一级取安装目录，加载期就能用；Server 建 Router 时再用
  `bundled_root()` 校正一次。
- **输入法自己的组合键优先**：`Ctrl + 数字`（主键盘 / 小键盘）、`Ctrl + Enter`、`Ctrl + 反引号`、
  `Shift + 反引号`、`Ctrl + Alt + 空格`（中英切换的可选组合）、`Ctrl + Alt + ,` `.`（TSF 保留键），
  以及**预留给非全拼 v/u/i 模式**的 `Ctrl + Alt + V` / `U` / `I`（2026-10-17 起，带不带 Shift 都保留）——
  **这些组合不派发给脚本**（谁先定义谁优先，脚本抢不走），清单里声明它们的脚本直接**判无效**
  （规则在 `cloudime_script::is_reserved_combination`，Server 的 `reserved_combo` 与清单解析共用同一份）。
  它们是杀词 / 原样上屏 / 翻译 Tip / 发音 / 模式切换，被脚本抢了用户就没法用；「键位映射参考」那套
  组合键（设置页选的那一个）在启用时由 `is_key_map_hotkey` 在派发脚本之前挡掉。
  别的 `Ctrl` 组合（含 `Ctrl+Shift+…`）与所有不带修饰键的键
  都归脚本随便绑（**组句里**一律送；**没组句**时只有存在 `trigger_condition = "combination_key"` 的脚本才送 ——
  Server 把它折进下发给 DLL 的 `InputSettings.script_key_modifiers`（修饰键位图的并集），见 `eats_key` / `wants_script_key`（**精确匹配**：
声明哪一套就只送哪一套）；**脚本不吃的按键由 DLL 用 `SendInput` 重放回应用**（`replay_to_app`，配 `is_replay` 那个「刚重放、别再吃」的标记）—— `OnTestKeyDown` 一旦答「吃」，光在 `OnKeyDown` 里返回 false 应用是拿不到的；另外**修饰键本身**（Ctrl / Alt / Shift / Win）一律不吃
（`is_modifier_key`）—— 吃了 `Ctrl` 的 key-down，后面的 `Ctrl+A` 就会被当成普通 `a`；一个这类脚本
  都没有时这些键原样交给应用，与以前逐字节一致）。
- **`key` 触发脚本声明具体的键**（`keys`）：Server 把它折进下发给 DLL 的 `InputSettings.script_keys`
  （虚拟键码的 256 位位图 `[u64; 4]`），DLL 在**没组句**时也把命中的键送来问一趟（组句里引擎本来就要所有键，
  但运行时只把「修饰键 + 键」都对得上的那些转给脚本）—— 脚本不吃就**重放回应用**（同组合键那条；**带修饰键的组合立刻注入**，
  **不带修饰键的功能键延后 ~150ms** 再注入：正按着的那一刻注入会被 Chromium 当自动重复丢掉，而组合键延后
  会因用户松开修饰键而失效）。⚠️ 脚本自己上屏的文本带 `\r\n` 时，紧接着读回的前文
  可能少最后一个字符（宿主把光标停在最后一个字符之前），脚本自己上屏后要能容错。
  **修饰键是精确匹配的**：`combination_modifiers = "none"` + `keys = { "enter" }` 只收光按回车；
  `Shift+Enter` / `Ctrl+Enter` 是另外两套组合，没声明就归应用。这给「按键后要看一眼
  光标前文再决定」的脚本（自动序号）开了路：配合 `needs_context = true` 每键前现读前文，
  `Enter` 那一拍就能读到刚敲完的那一行。
  只声明了键、**没吃**就等于没做事（按键本来也会到应用），所以这类脚本的意义在 `commit`。
- 「不认」的两项（`passthrough` / `commit`）要**有按键**才谈得上：没有按键的那两拍给了只记一条日志。
- 脚本能看到 `cloudime.context`（光标前文 —— 应用里已经输入、不在候选窗口里的那段文本）。
- `key` 的载荷里另有 **`highlight`**：这一刻高亮候选的文本（没有候选时是空串）—— 想自己翻它就用它
  （`cloudime.http_post(event.highlight, "……", { timeout_ms = 6000 }, …)`，见 `docs/design/online-translate.md`）。
- 脚本能看到的 `cloudime.http_get` / `cloudime.http_post` 都是异步、时限必给（见「其余硬规则」）；
  改脚本本身要**重启 Server**（托盘右键「重启输入法服务」）才生效：不做热重载。
- 改脚本要**重启 Server**（托盘右键「重启输入法服务」）才生效：不做热重载。

## 脚本窗口：`cloudime.window`（2026-10-17）

脚本可以自己建**独立窗口**（有自己那块窗与生命周期）与**辅助窗口**（挂在某块脚本独立窗口名下的资产），
用来做面板、悬浮提示、小工具 —— 两类窗口的定义与规矩见 `docs/design/windows.md` §1–§5，这里只定
「脚本能看见什么」。

**为什么内容是声明式的**：脚本跑在工人线程、窗口在 UI 线程，两边不能直接互相调用（从 UI 线程回调 Lua
会卡住所有应用的输入）。所以脚本不「画」，只交一份**元素表**（`Document` = 内容区尺寸 + 有序元素），
Server 在 UI 线程上经云朵渲染器（`Renderer::render_document`）一次出图、`UpdateLayeredWindow` 整块贴上 ——
与候选窗同一条渲染路（`docs/design/windows.md` §6）。

### 建窗口

```lua
local win = cloudime.window.create{
    name = "统计",        -- 日志与 window 事件里点名；缺省「脚本窗口」
    width = 240,          -- 内容区宽（点，8–2048；缺省 240）
    height = 120,         -- 内容区高（点，8–2048；缺省 120）
    interactive = true,   -- 接不接鼠标（缺省 false = 鼠标穿透，纯展示）
    shadow = true,        -- 要不要与候选窗同源的那层阴影（缺省 true）
    background = true,    -- 要不要输入法画的面板底色 / 圆角（缺省 true）；false = 全由元素表自己画
}

local aux = cloudime.window.create_auxiliary{
    owner = win,          -- 必给：脚本建过的独立窗口（句柄上的 id）
    width = 160,          -- 可选：内容尺寸缺省值（set_content 里再给也行）
    height = 200,
    gap = 16,             -- 与宿主之间的间隙（点；缺省 16）
    interactive = false,
    shadow = true,
}
```

句柄上的方法（两类窗口一套）：

| 方法 | 意思 |
|---|---|
| `win:set_content{…}` | 换一整份内容（整份替换；见下），尺寸可以跟着变 |
| `win:show{ x = …, y = … }` | 显示；`x` / `y` 是内容区左上角的**屏幕坐标（点）**，不写 = 上次的位置 / 光标右下 16 点 |
| `win:hide()` | 收起（不销毁；内容留着，再 `show` 原样回来） |
| `win:close()` | 销毁；独立窗口连名下资产一起放掉 |
| `win:focus(true/false)` | 独立窗口：拿 / 放**键盘焦点**（`key` 载荷里会带 `window`）；辅助窗口：把逻辑焦点指定给它 / 交还给宿主 |
| `win:combine_region(name, 形状)` | 把形状并进已有区域（只改命中、不重画） |
| `win:prune_region(name, 形状)` | 从已有区域里挖掉形状（只改命中、不重画） |
| `win:region_contains(name, x, y)` | 点是否在区域里（含边界）；没有这个区域给 `nil` |
| `win:region_relation(a, b)` | 两块区域的关系：`{ relation = "disjoint" / "intersect" / "contains" / "coincide", contains = "a" / "b" }`；没有区域给 `nil` |

- 辅助窗口**默认不接鼠标**：`interactive = true` 只是「能」拿焦点，真要它接还得 `aux:focus(true)`
  （§4 的规矩：只有所有者能指定）。宿主被收起时名下资产跟着收起，`win:show()` 时自己回来。
- 窗口都是置顶、不抢系统焦点（`WS_EX_NOACTIVATE`）；`interactive = false` 的整块鼠标穿透。
- 建窗口 / 换内容 / 显示只是**记一个请求**：脚本不会等 UI 线程，UI 线程按顺序执行。

### 内容（`set_content`）

```lua
win:set_content{
    width = 240, height = 120,          -- 可省：缺省用建窗口时那份
    { type = "rect",   x = 0, y = 0, w = 240, h = 32, radius = 8, color = "#33111111" },
    { type = "text",   x = 12, y = 6, text = "今日 12 条", size = 16,
      font = "candidate", color = "#FF222222" },
    { type = "line",   from = { x = 0, y = 32 }, to = { x = 240, y = 32 },
      thickness = 1, color = "#22000000" },
    { type = "circle", cx = 220, cy = 16, radius = 8, color = "#FF66AAFF" },
    { type = "svg",    x = 8, y = 8, size = 20, path = cloudime.install_path("Images/SVG/打叉.svg"), tint = "#FF888888" },
    { type = "img",    x = 0, y = 0, w = 64, h = 64, path = "Scripts/lib/icon.png" },
    { type = "path",   color = "#FF0000FF",
      commands = { { "M", 0, 0 }, { "L", 10, 0 }, { "C", 12, 0, 12, 10, 0, 10 }, { "Z" } } },
}
```

| 元素 | 字段 |
|---|---|
| `rect` | `x` / `y` / `w` / `h` 必给；`radius` 圆角（0 = 直角）；`color` 必给 |
| `circle` | `cx` / `cy` / `radius`；`color` 必给 |
| `line` | `from` / `to`（`{ x = …, y = … }` 或 `{ x, y }`）、`thickness`（点，缺省 1）；`color` 必给 |
| `text` | `x` / `y`（行框左上角）/ `text` / `size`（点，缺省 16）；`font` = `candidate`（缺省）/ `pinyin` / `item_number` / `translate`；`color` 缺省黑；`italic` 可选 |
| `svg` | `x` / `y` / `size`（见方）/ `source`（整段 SVG 文本）或 `path`（SVG 文件）；`tint` 可选（按 alpha 染色） |
| `img` | `x` / `y` / `w` / `h` / `path`（PNG；相对路径按 Server 的工作目录 = 安装目录算） |
| `path` | `commands`（`{"M",x,y}` / `{"L",x,y}` / `{"Q",cx,cy,x,y}` / `{"C",…}` / `{"Z"}`）；`color` 必给 |

- 坐标与尺寸一律是**点**（100% 缩放下 1 点 = 1 像素），随 DPI 一起放大；超出内容区的部分被裁掉。
- 颜色：`"#RRGGBB"` / `"#AARRGGBB"` / `"#RGB"`，或 `{ r = …, g = …, b = …, a = … }`（0–255，`a` 缺省 255）。
- **限额**（当场报错，这一次调用中止、窗口保持原样）：一份内容最多 **512** 个元素；一段文字最多 **4096** 字符；
  图像解码后不超过 **4096 × 4096**（约 1600 万像素）；内容区尺寸夹到 **8–2048 点**。
- **图像缓存**（2026-10-17）：`img` 按路径缓存解码结果（比较文件长度 + 修改时间），同一张图在多份内容里共享
  同一块位图、不重复解码。最多留 **8 张 / 32M 像素**（超了丢最久没用的）；文件改过之后**下一次** `set_content`
  就会用新图。相对路径按 Server 的工作目录（安装目录）算。
- **混合 DPI**（2026-10-17）：`show{ x, y }` 的点坐标先按当前显示器 DPI 猜一个落点、问出那块显示器真正的 DPI
  再换算；窗口拖到另一块 DPI 不同的屏上会按新 DPI 重画（内容尺寸跟着变），名下的辅助窗口一起重新摆。
- **区域（region）**：内容里可以声明 `regions = { … }` —— 区域是**鼠标点击 / 拖拽的命中范围**
  （元素只管画）。`name` 必写、同一份内容里唯一；`drag` 缺省 `true` = 按住它拖会移动整块窗口，
  `false` = 不拖窗口（点击照常）。后写的区域盖住先写的；事件里的 `event.region` 就是命中的区域名
  （没声明 `regions` 时仍按元素命中 `event.element`）。
  - **形状**不必是矩形：不写 `shape` 时按字段猜（有 `commands` 当路径、有 `cx` / `cy` 当圆、其余当矩形），
    也可以显式写 `shape = "rect" | "round_rect" | "circle" | "path"`：矩形 / 圆角矩形
    `{ x, y, w, h, radius }`、圆 `{ cx, cy, radius }`、路径 `{ commands = { … } }`（与 `path` 元素同一套命令）。
  - 区域还可以带 `combine`（并集）与 `prune`（差集），各写一个形状或形状数组；命中先看 `prune`，
    再看原始形状与 `combine`。运行时也能用 `win:combine_region(name, 形状)` /
    `win:prune_region(name, 形状)` 改已有区域（只改命中、不重画）。
  - **查询**：`win:region_contains(name, x, y)` 判断点是否在区域里（与鼠标命中同一套判断，含边界），
    没有这个区域给 `nil`；`win:region_relation(a, b)` 判断两块区域的关系，返回
    `{ relation = "disjoint" | "intersect" | "contains" | "coincide", contains = "a" | "b" | nil }`
    （内含时 `contains` 说明谁包住谁；只贴一条边算相离）。查的是脚本侧的区域镜像
    （`set_content` / 合并 / 剪除后同步），不开销 UI 线程。

### 鼠标、滚轮、拖动（`cloudime.on("window", …)`）

`interactive` 的窗口把鼠标动作派给脚本：

```lua
cloudime.on("window", function(event)
    -- event.window   建窗口时给的名字
    -- event.id       窗口编号
    -- event.action   "click" / "right_click" / "hover" / "leave" / "wheel" / "focus" / "blur"
    -- event.x, event.y   内容区坐标（点）
    -- event.element  命中的元素（内容是 1 起的数组，这里也是 1 起；没命中是 nil）
    -- event.region   命中的区域名（内容里的 regions；没命中是 nil）
    -- event.delta    滚轮量（一格 ±120，正数往上；只有 wheel 有）
    if event.action == "wheel" then
        count = math.max(0, count + math.floor(event.delta / 120))
        win:set_content{ … }
    end
end)
```

- `click` / `right_click` 在**左键 / 右键抬起**时派；`hover` 只在**停到另一个元素**上时派一次
  （同一元素里挪不重复派）；`leave` 在鼠标离开窗口时派；`wheel` 带 `delta`。
- 命中**先看区域、再看元素**（后写的区域 / 元素盖住先写的）：`event.region` 给区域名、`event.element` 给
  元素下标；没声明区域时只按元素命中。**拖动**从按下点所在的区域开始：`drag = false` 的区域按住了不拖窗口。
- **命中按形状**（2026-10-17）：圆角矩形抠掉四个角、圆按边界、线段按粗细、路径按非零环绕填充判断；
  文字 / 图像 / SVG 仍按外框。区域与元素用同一套形状。
- **拖动**：在窗口上按住左键拖动就移动整块窗口（超过 4 像素才算拖，拖过就不派 `click`），
  名下的辅助窗口跟着重新摆；拖到另一块 DPI 不同的显示器上会按新 DPI 重画。
  辅助窗口本身不能拖（摆位归宿主）。
- **键盘焦点**：点一下（或 `win:focus()`）这块窗口就拿到键盘焦点（`window` 事件收到 `focus`），
  点 / 收起 / 关掉时就收到 `blur`。拿到焦点期间，**`key` 载荷多带两个字段**：`window`（名字）与
  `window_id`（编号），脚本据此把按键分给「哪块面板」：

  ```lua
  cloudime.on("key", function(event)
      if event.window_id ~= win.id then return end
      if event.vk == 0x1B then win:hide() end      -- Esc
  end)
  ```

  ⚠️ 窗口**不抢系统的键盘焦点**（`WS_EX_NOACTIVATE`）：能收到哪些键由清单决定 —— 组合键脚本
  `combination_modifiers` + `keys` 声明的那一组（没组句也送）；**不是**「有焦点就能拿到所有按键」。
  窗口里没有文本输入框，也不做组句。
- `window` 事件**不受清单 `trigger_condition` 的限制**（与 `startup`、HTTP 回调一样）：窗口是脚本自己建的，
  事件当然归它。
- 处理函数的返回值按「没有按键的那一拍」处理：认 `notice` / `online` / `theme` / `adjust`，
  `passthrough` / `commit` 给了只记日志。

### 生命周期与出错

- 窗口活到 `close` 或进程退出；`hide` 不销毁。独立窗口销毁时名下资产一起放掉（§1 的寿命规矩）。
- 内容写错（缺颜色 / 元素类型不认识 / 尺寸不是正数…）**当场报 Lua 错误**：这一次调用中止、窗口保持原样，
  输入法照常（与别的脚本 API 一样）。
- 没有脚本要窗口时零开销：`cloudime.window` 只是一张表，一个窗口都不建。

### 还没做

- 窗口内的**文本输入 / 输入法组句**（`cloudime.window` 只做展示与交互面板）。
- 拖动**元素**（滑块、列表滚动）：现在拖动只移动整块窗口。
- 键盘不是「有焦点就拿到所有键」：受清单声明与 DLL 的送键规则限制（见上）。
- 按文件句柄读图（只有路径）；图像缓存靠下一次 `set_content` 发现文件变化，不监听文件系统。

## 调用链路：一次按键走一遍

```text
DLL（应用进程）                     Server 工人线程（独占 Router + Lua 状态）
   │  Key{event}                        │
   └───────────────────────────────────►│ handle_key(session, event)
                                        │  1 ensure_focus（焦点换了就先清组句）
                                        │  2 script_actions(event) ◄── 调 key 处理函数
                                        │       └ 返回的表按登记顺序合并 → ScriptActions
                                        │  3 应用动作：adjust / passthrough / commit / notice / online
                                        │  4（没被接管时）apply_key → 引擎
                                        │  5 recompose() → 引擎查词与排序
                                        │       └ candidates 处理函数 ◄── 排完再调
                                        │  6 self_drawn_frame → reconcile_candidates
   ◄────────────────────────────────────┘  KeyResult{outcome, commit, frame}
```

两个钩子**夹住引擎**：`key` 在引擎处理之前、`candidates` 在引擎排序之后。

**启动**（在 `Router::new` 里，工人线程上）：建 Lua 状态 → 装 `cloudime` 表与几处补丁 → 装预算钩子 →
读 `scripts\*.lua`（按文件名排序）逐个执行（每个脚本在加载时跑一遍、用 `cloudime.on` 登记）→ 全部加载完
派一次 `startup`。目录不存在或一个 `.lua` 都没有 → 不读盘，之后所有钩子点直接返回（见规则一）。

**每次按键**：`key` 是「引擎处理之前」那一拍，所以载荷里的 `composing` 反映的是**上一拍**的状态。返回的动作
按固定顺序落地 —— `adjust` 先递给引擎（因此这一键的排序就用上），`passthrough` / `commit` 直接接管这一键
（引擎完全看不到它），`notice` 随本帧下发，`online` 落在候选窗底部那一行
（脚本给的文本，Server 只负责画），`theme` 写配置 + 重启输入法服务（`cloudime.apply_theme` 那条路）；
**候选窗开着时先挂起、等它关掉那一拍才换/重启**，
`order` / `display` 留到 `recompose` 之后再用。没被接管就走引擎
（能改什么见规则二那张表）。

**候选排完**：`recompose()` 末尾、引擎排好序且高亮已定之后调 `candidates`。载荷里的 `weight` 就是 Core 这一拍
刚算出来的分（Server 在 `recompose` 里顺手按文本留了一份）。`order` / `display` 改的是 `Composed` 里那份
`CandidateLayout` 本身 —— 之后的导航、数字键、空格、鼠标点选都按新顺序走，上屏仍由 `Engine::commit` 决定；
高亮按「原下标 → 新下标」跟着原来那个候选。**这一步与按键在同一拍里做完**，所以 DLL 拿到的帧已经带上脚本的
顺序与显示。

**以后某一拍**：`Router::tick()` 里 `poll_scripts_requests()`（工人循环超时与 DLL 的
`Poll` 都会调它：组句时约 80 ms、闲置约 1 秒）。到点的请求调它的回调 `(结果表, 那一刻的候选表)`，
返回的动作按同一套语义落地并立刻重画候选窗。

**线程与共用**：上面全部在**同一条工人线程**上（DLL 每条连接只把消息投进通道，UI 线程的鼠标 / 点击也投回
同一条通道）→ Lua 状态单线程、没有竞态；HTTP 请求在一次性线程里跑，只把结果字节经通道递回，**回调仍在工人
线程**上执行。Lua 全局变量、`adjust`、`notice` 是**全局一份、跨应用共用**；`cloudime.context` 是当前聚焦
应用的前文；`key` / `candidates` 载荷都带 `app`，脚本据此分支。

## 设置页「脚本」（已实现）

页面在设置程序左侧导航的「脚本」（`apps/windows/settings/src/panel/pages/scripts.rs`），三块内容：

**① 注意事项**：一个 TextBlock，**加粗 + 红色**，原文照抄

> 声明：云朵输入法提供lua脚本的执行，但不对lua脚本安全性负责。一切由lua运行导致的负面后果由用户个人自行承担。

**② 脚本列表**：Grid，三列

| 列 | 内容 |
|---|---|
| 一 | 脚本文件名（`Scripts\` 下的） |
| 二 | 脚本介绍：清单里的 `description`（从文件里扫出来，不执行脚本）；没写就是文件名 |
| 三 | `ToggleSwitch`（启用 / 禁用）+ 「删除此脚本」按钮 + 「编辑此脚本」按钮 |

- 「编辑此脚本」：用**记事本**打开那个文件（`notepad.exe <文件>`）。
- 「删除此脚本」：删文件；删之前确认一次。
- 列表**不显示 `template.lua`**。

**③ 新建脚本**：一个按钮。流程：

1. 在 `Scripts\` 下取一个不重名的文件名（`script.lua` → `script-2.lua` …），写入模板的内容（模板**编进设置程序**：
   `include_str!` 仓库 / 安装目录里那份 `Scripts\template.lua`，模板文件被删也建得出来）；
2. 用记事本打开它（`notepad.exe <文件>`）；
3. **再**把同一份内容用 `WM_SETTEXT` 塞进记事本的编辑控件：`EnumWindows` 找主窗口（**先按我们刚起的进程号**，
   机器上本来就开着别的记事本时才按类名 `Notepad` 兜底）→ `FindWindowExW(hwnd, None, "Edit", None)` →
   `SendMessageW(edit, WM_SETTEXT, 0, …)`。等窗口与填字都在后台线程做（轮询上限 4 秒），不卡界面。

第 1 步保证「用户直接关掉记事本」时文件已经存在、内容就是模板；第 3 步保证「记事本里显示的一定是模板」，
不看记事本读盘的时序。第 3 步只对**经典记事本**有效：Windows 11 商店版记事本是 WinUI 应用、没有 `Edit`
子控件，塞不进去 —— 记一条日志，文件里已经写好模板，用户看到的不会受影响。

**列表本身的实现要点**（`apps/windows/settings/src/panel/pages/scripts.rs`）：

- 介绍是**文本扫**出来的（`describe_in`）：认 `description = "…"` / `'…'`、跳过 `--` 注释行 ——
  设置程序里**不执行脚本**（Lua 运行时只在 Server）。
- 「删除此脚本」先用 `rfd` 确认一次；删完顺手把它从 `[script] disabled` 里去掉（不留永远不起作用的条目）。
- 开关写 `[script] disabled`（`Config::set_array`，配置里缺 `[script]` 分节会补出来）。
- 操作那一列（开关 + 两个按钮）钉死 **300 DIP**，三个控件都放开最小宽度（`min_width(0)`），开关再用
  `ToggleSwitchSlot::OnContent` / `OffContent` 置空自带的「开 / 关」文字：WinUI 的 `Button` 默认最小宽度是 120、
  开关默认还带一段文字，三个控件按默认宽度加起来会顶出列外，最后那个按钮被右边缘切掉一截。

**目录与 `template.lua`**

- 脚本放**安装目录的 `Scripts\`**（与 `Phrases\`、`WordBank\` 同级）。安装包要把这个目录设成**普通用户可写**，否则设置页建不了新文件、用户也存不了盘。
- `template.lua` 是「新建」用的模板：**不进列表、也不执行**（加载器按文件名跳过它）。
- 加载器只认 `.lua`；判无效的脚本不加载（见「脚本清单」）；**禁用**的脚本也不加载（见「决定」第 2 条）。
- 卸载会删掉 `{app}` 整棵 —— 用户的脚本会跟着没（与 `Phrases\Phrase.db`、`WordBank\` 里导入的词库同一个路子，安装器 README 里已经写明）。

### 决定（2026-10-07）

1. **脚本介绍**：清单里的可选字段 `description = '一句话介绍'`（写法见 `Scripts\template.lua`）；不写就显示文件名。
2. **启用 / 禁用**：配置文件 `[script] disabled = ["a.lua"]`（`config.toml` 里，与别的开关同一份）；Server **启动时**按它跳过。
3. **脚本目录**：安装目录的 `Scripts\`（不是数据目录）；安装包给这个目录授权，卸载会连它一起删（与 `Phrases\`、`WordBank\` 同一个路子）。

### 状态

全部实现：`[script] disabled` 分节、运行时 `Runtime::load(dir, disabled)`（跳过 `template.lua` 与禁用名单，
大小写不敏感）、清单的 `description`、Server 从**安装目录**取 `Scripts\`、安装包装 `Scripts\template.lua`
并给目录授权；设置页的三块（声明 / 列表 / 新建脚本）与上面第 ③ 条的流程。

## 出错与风险怎么兜

- **加载期出错**：那一个脚本跳过（记日志），其余照常加载。
- **处理函数出错**：那一次跳过（记日志），按键照旧走引擎 —— 输入法不会因为脚本写错而失灵。
- **HTTP**：请求在一次性线程里跑，不阻塞输入；**响应时间限制必给**，超过时限才回来的结果一律丢掉。
- **风险边界**：脚本拿到的是「安全子集 + 刻意例外」的那套标准库（见「目标与边界」），能读写文件、起进程。
  它跑在 Server 进程里，所以最坏情况是 Server 崩掉（宿主应用不受影响；DLL 发现连不上会重新拉起 Server）。
  「崩」由用户自担，输入法不替脚本兜底。

## 还没做

- **惰性 Lua 状态**：现在 Server 启动就建一个空状态（见规则一）。改成「扫描到 `*.lua` 才建」能让
  「没脚本」彻底零残留。
- **组句内部「已选文本」的改写**：Engine 没有「改组句内容」的入口，暂时不做。
