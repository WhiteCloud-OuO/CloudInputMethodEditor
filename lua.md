# 在云朵输入法里写 Lua 脚本

面向脚本作者的技术说明：清单怎么写、输入法给了哪些方法、事件载荷与动作表、运行环境的限制、
常见坑，以及一批可以直接抄的例子。

权威出处（改代码时以它们为准）：

| 文件 | 内容 |
|---|---|
| [`docs/design/script.md`](docs/design/script.md) | 脚本功能的设计与规则（清单、闸门、派发点） |
| [`docs/design/online-translate.md`](docs/design/online-translate.md) | 「在线翻译由脚本自己实现」这一条的来龙去脉 |
| [`docs/notes/crate-notes.md`](docs/notes/crate-notes.md) | 实现要点（`crates/cloudime-script`、Server 的「用户脚本」两节） |
| [`Scripts/template.lua`](Scripts/template.lua) | 「新建脚本」用的模板（只有清单骨架） |
| [`Scripts/lib/example-theme-by-app.lua`](Scripts/lib/example-theme-by-app.lua) | 示例：按前台程序换主题（组合键触发 + `cloudime.foreground_app()` / `cloudime.apply_theme()`） |
| [`Scripts/lib/example-auto-number.lua`](Scripts/lib/example-auto-number.lua) | 示例：自动序号（`combination_key` + `none` 单键 + `cloudime.context` + `commit`「换行 + 下一条序号」） |

实际使用的脚本（小牛候选翻译、小牛整句翻译窗口、在线查询）在
[CloudIME_ScriptWarehouse](https://github.com/WhiteCloud-OuO/CloudIME_ScriptWarehouse) 的 `src\` 下；
本仓库源码的 `Scripts\lib` 只保留示例与共享工具。**安装包只装 `Scripts\template.lua`**，
示例、共享模块和实际脚本都不随包，需要时自己下载到 `Scripts\`。

## 目录

1. [能做什么、在哪跑](#1-能做什么在哪跑)
2. [五分钟上手](#2-五分钟上手)
3. [脚本清单（manifest）](#3-脚本清单manifest)
4. [事件与载荷](#4-事件与载荷)
5. [返回值：动作表](#5-返回值动作表)
6. [`cloudime` 表：API 参考](#6-cloudime-表api-参考)
7. [标准库与运行环境](#7-标准库与运行环境)
8. [稳定性闸门](#8-稳定性闸门)
9. [状态与持久化](#9-状态与持久化)
10. [设置页、启用与禁用](#10-设置页启用与禁用)
11. [调试与排错](#11-调试与排错)
12. [示例集](#12-示例集)
13. [版本与边界](#13-版本与边界)

## 1. 能做什么、在哪跑

| 能做的 | 用什么 |
|---|---|
| 文本扩展、快捷输入 | `key` 事件返回 `commit` |
| 在特定应用里放行按键（游戏、终端） | `key` 事件返回 `passthrough` |
| 给候选加权 / 降权、改顺序、改显示 | `adjust` / `order` / `display` |
| 弹一行提示 | `notice` |
| 接厂商接口做在线翻译，写在候选窗底部那一行 | `http_post` + `online` |
| 改候选窗尺寸、缩放、一页几个 | `cloudime.candidate.*` |
| 量文字宽、截断 / 折行（按窗口宽度排版） | `cloudime.ui.*` |
| 建自己的面板 / 悬浮小窗（可点、可悬停） | `cloudime.window.*` |
| 起外部程序、读写自己的文件 | `os.execute` / `io.*` |

**跑在哪儿**：脚本引擎在 **Server 进程**里（`crates/cloudime-script`，mlua + LuaJIT），
**不在 TSF DLL 里** —— DLL 被加载进每一个应用，逻辑只留一份在 Server。

**最重要的一条**：Server 是**一个进程、一条工人线程服务所有应用**。脚本卡住不是「某个应用卡」，
而是**全都不打字**。所以有一套闸门（见[第 8 节](#8-稳定性闸门)），以及「只允许用异步 HTTP 等」这条硬规矩。

**风险自担**：设置页「脚本」页顶部的原文声明是：

> 云朵输入法提供lua脚本的执行，但不对lua脚本安全性负责。一切由lua运行导致的负面后果由用户个人自行承担。

输入法只保证两件事：脚本出错**不影响打字**（那一次跳过、按键照旧走引擎），以及下面那几道闸门。

## 2. 五分钟上手

1. **放哪儿**：安装目录下的 `Scripts\`（与 `Phrases\`、`WordBank\` 同级）。
   相对路径按 Server 的工作目录（就是安装目录）算。安装包只带 `template.lua`；示例与 `md5.lua`
   在源码仓库的 `Scripts\lib` 和脚本仓库里，要引共享模块就自己建 `Scripts\lib\` 再放进去。
2. **怎么建**：设置 → 脚本 → 「新建脚本」会用模板建一个文件并用记事本打开。
   也可以自己往 `Scripts\` 里丢一个 UTF-8 的 `.lua`。
3. **什么时候生效**：**改完要重启输入法服务** —— 任务栏「中 / 英」图标右键 → 「重启输入法服务」，
   或者「设置 → 脚本」页「新建脚本」右边那个同名按钮。没有热重载。
4. **只加载根目录的 `Scripts\*.lua`**：`Scripts\lib\` 是子目录、加载器不认，适合放共享模块
   （用 `dofile` 引）；`template.lua` 是模板，**不执行**；在 `[script] disabled` 名单里的也跳过。

最小可用脚本（和模板的默认骨架一样：Ctrl+Alt+J 上屏；模板里还留了时间段的 `time_range`、
候选拼音的 `candidate_pinyin`、焦点程序的 `apps` 这几行，换触发方式时取消注释即可）：

```lua
cloudime.script{
    name        = "我的脚本",
    description = "Ctrl+Alt+J 出「云朵输入法」",
    api         = 2,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",   -- 要哪一套修饰键（"none" = 不按）
    keys        = { "j" },                -- 具体键
    budget      = 200000,          -- 单次调用的指令数上限（只能收窄）
    timeout     = 200,             -- 单次调用的墙钟上限（毫秒）
    sync        = true,            -- 同步：处理函数算完就返回
    handover    = "return",        -- 必须与 sync 一致
    on_error    = function(event, message)
        cloudime.log("出错：" .. event .. " → " .. message)
    end,
    priority    = 0,
}

-- Ctrl+Alt+J 上屏「云朵输入法」（清单已经把键筛过了）
cloudime.on("key", function()
    return { commit = "云朵输入法", notice = "脚本上屏" }
end)
```

`cloudime.log` 写进运行日志：`%LOCALAPPDATA%\CloudIME\logs\server.<日期>.log`（按天分文件、保留 7 天）。
加载失败、清单不合法、处理函数出错的原因都在里面。

## 3. 脚本清单（manifest）

每个脚本**必须在文件最前面**声明一次 `cloudime.script{…}`。缺清单、字段缺失 / 类型不对、
`api` 不认识、`sync` 与 `handover` 对不上 —— **这个脚本判无效**（跳过 + 日志写明原因，别的脚本照常）。

| 字段 | 必写 | 类型 | 语义 |
|---|---|---|---|
| `api` | **是** | 整数 | 面向的脚本 API 版本，现在只有 `2`。不认识 → 无效（`1` 是触发条件改版前的旧版，已不再接受） |
| `trigger_condition` | **是** | 字符串 | **由什么触发**，五选一：`"combination_key"` 按下组合键（收 `key`）；`"time_range"` 某个时间段（收 `time`，每段只触发一次）；`"candidate_context"` 候选窗口出现且拼音串等于指定内容（收 `candidates`）；`"focus_app"` 编辑光标位于指定程序（收 `focus`）；`"manual"` 手动触发（i 模式预留，永不自动派发）。写别的 → 无效 |
| `combination_modifiers` | `combination_key` 时**是** | 字符串 | 要哪一套**修饰键**，11 选一：`"none"`（不按修饰键，单键）/ `"ctrl"` / `"ctrl+alt"` / `"ctrl+shift"` / `"alt+shift"` / `"alt"` / `"win+ctrl"` / `"win+alt"` / `"win+shift"` / `"win+alt+ctrl"` / `"win+ctrl+shift"`（`+` 连接的顺序随意）。**精确匹配**，多按一个修饰键就不算 |
| `keys` | `combination_key` 时**是** | 字符串数组 | 要哪几个键：`"enter"` / `"tab"` / `"space"` / `"backspace"` / `"delete"` / `"esc"` / `"left"` / `"up"` / `"right"` / `"down"` / `"home"` / `"end"` / `"pageup"` / `"pagedown"` / `"f1"`…`"f12"`，或单个字母 / 数字 / 标点（`a`–`z`、`0`–`9`、`` ` - = [ ] \ ; ' , . / ``）。可写多个；去重、忽略大小写与首尾空白。有一个不认识 → 整个脚本无效 |
| `time_range` | `time_range` 时**是** | 表 | `{ from = "20:00", to = "06:00" }`（本地时间，左闭右开；`from > to` 表示跨夜）。**每个时间段窗口只触发一次** |
| `candidate_pinyin` | `candidate_context` 时**是** | 字符串 | 候选窗口出现且拼音串等于它才触发；比较时忽略 `'` 与空白、不区分大小写（`Ni'Hao` = `nihao`） |
| `needs_context` | 否 | 布尔 | `true` = 每次派发前刷新 `cloudime.context`（按键前多读一次光标前文）；读前文的脚本要写它 |
| `sync` | **是** | 布尔 | `true` = 同步（处理函数算完就返回）；`false` = 只用异步回调（配 `http_get` / `http_post`） |
| `handover` | **是** | 字符串 | `"return"` / `"callback"`，**必须与 `sync` 一致**（`true`↔`"return"`、`false`↔`"callback"`） |
| `timeout` | **是** | 整数 | 单次调用的**墙钟上限**（毫秒，`1`–`60000`）：同步处理函数与异步回调都按它算，到点中止这一次 |
| `on_error` | **是** | 函数 / `false` | 形式 `function(event, message)`：这个脚本出错时调它。写 `false` = 不处理，直接中止这一次 |
| `budget` | 否 | 整数 | 单次调用的**指令数**上限；缺省用全局 `100000000`（一亿）。**只能收窄不能放宽**（写大了按全局走并记一条日志） |
| `apps` | `focus_app` 时**是**；其余可选 | 字符串数组 | 光标在哪些程序里触发（`focus_app`）；其它触发条件下是「只在哪些应用里跑」（**exe 文件名**，大小写不敏感，如 `"notepad.exe"`）。空 / 不写 = 所有应用 / 不触发 |
| `name` | 否 | 字符串 | 日志与设置页里点名用；缺省 = 文件名 |
| `description` | 否 | 字符串 | 设置页列表里的一句话介绍（**扫文本**取出来，不执行脚本）；缺省显示文件名 |
| `priority` | 否 | 整数 | 多个脚本要改同一项时：大的盖小的；同值按文件名顺序 |

几条铁律：

- **一个文件一个脚本、UTF-8 编码**；`cloudime.on` 只能在**加载期**调（也就是在清单之后、文件里直接写），
  运行期再登记会被拒绝。
- **不许改 `cloudime` 表**：脚本拿到的是只读代理，写会报错。脚本自己的状态放普通全局变量里。
- **持久状态写数据目录**（`%APPDATA%\CloudIME\` 下），别写安装目录 —— 普通用户写不进去，
  而且**升级 / 卸载会删掉安装目录整棵**（`Scripts\` 里的脚本也会没，记得备份）。
- **不许长时间占住工人线程**：唯一允许的「等」是两个**异步** HTTP（时限必给）。
- `apps` 除了当 `focus_app` 的触发条件，还会过滤 `key` / `candidates` / `time`（都按当时聚焦的程序）；
  `startup` 早于任何应用、异步回调在请求发出时就已经判过。
- **一个脚本只有一个触发条件**（清单的 `trigger_condition`）：Server 只把对应的事件派给它 ——
  `combination_key` 收 `key`、`candidate_context` 收 `candidates`、`focus_app` 收 `focus`、
  `time_range` 收 `time`，`manual` 一个都不收；`startup` / `window` / HTTP 回调不受它限制。
- **组合键脚本**（`combination_key`）：修饰键 + 键都要声明，**精确匹配**（`ctrl` 与 `ctrl+shift`
  是两套）。**不管候选窗口开没开都收**：输入法让 DLL 在没组句时也走 TSF 的按键路径先问一趟 Server
  （脚本不吃就**重放回应用**，见 `replay_to_app`）；修饰键**本身**（`Ctrl` / `Alt` / `Shift` / `Win`）
  不会被动。`key` 载荷里的 `ctrl` / `alt` / `shift` / `win` 就是那一刻按着的修饰键。
  设置页「脚本 → 为脚本提供全局热键」打开后，带修饰键的组合还会注册成 TSF 全局热键；
  只在中文状态抢占，英文 / 禁用输入时放行，修改后需重启输入法服务。
  ⚠️ `Alt+Tab` / `Alt+F4` / `Win+L` / `Win+D` 这类**系统 / 外壳自己会处理的**组合，输入法根本收不到，
  声明了也没用；`AltGr` 就是 `Ctrl+Alt`（打字用），声明 `ctrl+alt` 要当心吃掉它。
- **单个键**（比如「光按 Enter」）：`combination_modifiers = "none"` + `keys = { "enter" }`。
  `Shift+Enter` / `Ctrl+Enter` 是另外两套组合，没声明就归应用。
- **输入法自己占用的组合键不能绑**：`Ctrl+Enter`（原样上屏）、`Ctrl+反引号`（翻译 Tip）、
  `Shift+反引号`（发音）、`Ctrl+数字`（主键盘 / 小键盘）、`Ctrl+Alt+空格`（中英切换，如果开了）、
  `Ctrl+Alt+,` / `Ctrl+Alt+.`（简繁 / 标点），以及**预留给非全拼 v/u/i 模式**的
  `Ctrl+Alt+V` / `Ctrl+Alt+U` / `Ctrl+Alt+I`。清单里写了这些组合的脚本**判无效**（日志写明是哪一个）；
  另外「键位映射参考」那套组合键（设置页选的那一个）在启用时也不会派给脚本。
- **在 `key` 里返回 `passthrough` 等于本来就会发生的事**；真正有意义的是 `commit`（吃掉这一键、自己上屏）——
  所以只对确实需要的输入框（多行编辑的自动序号之类）生效才好，`Enter` / `Tab` 被吃掉会改变输入框行为。

## 4. 事件与载荷

用 `cloudime.on(事件名, 处理函数)` 登记。现在会派发的就下面几种（别的名字登记了也不会被调）；
**只有触发条件对上的那一种会派给你**（见清单的 `trigger_condition`）：

| 事件 | 什么时候调 | 能改什么 |
|---|---|---|
| `startup` | 全部脚本加载完（**所有脚本**都收） | 只做自己的准备（开文件、发首个请求）。**返回值会被丢掉** |
| `key` | 一次按键、**引擎处理之前**（只有 `combination_key` 脚本收，且修饰键 + 键要对得上清单） | 见[动作表](#5-返回值动作表)全部项；`passthrough` / `commit` 只有这一拍认 |
| `candidates` | **引擎排完之后**（只有 `candidate_context` 脚本收；拼音串要对得上清单的 `candidate_pinyin`） | `order` / `display` / `notice` / `adjust` / `online` / `theme`；`passthrough` / `commit` 给了只记日志 |
| `focus` | 焦点**切进**清单 `apps` 里的程序（前台程序报 `SyncMode` / 第一次按键时更新；同一程序里换输入框不重复派） | `notice` / `adjust` / `online` / `theme`；载荷 `{ app }` |
| `time` | 本地时间第一次进入清单 `time_range` 那个时间段（只有 `time_range` 脚本收；**每段只触发一次**） | `notice` / `adjust` / `online` / `theme`；`passthrough` / `commit` 给了只记日志。载荷同 `cloudime.get_time()`，另加 `app`（那一刻前台程序） |
| `window` | 脚本窗口上的鼠标 / 滚轮 / 焦点动作（`click` / `right_click` / `hover` / `leave` / `wheel` / `focus` / `blur`） | 同 `candidates`：`notice` / `adjust` / `online` / `theme`；窗口要 `interactive`（辅助窗口还要 `aux:focus(true)`）才接鼠标 |
| `caret` | 脚本调过 `cloudime.caret()`，DLL 量到插入点之后（见下面「光标位置与安装目录」） | 同 `candidates`；载荷是插入点的屏幕矩形 + 所在显示器工作区（屏幕坐标 / 点） |
| 异步回调 | HTTP 结果回来那一拍（**所有脚本**） | 同 `candidates`；`passthrough` / `commit` 给了只记日志 |

多个脚本按 `(priority, 文件名)` 顺序合并，**后面返回的盖前面的**（表里没写的项不动）。
返回值类型严格匹配，写错记一条日志当没写。

### `key` 的载荷

| 字段 | 类型 | 说明 |
|---|---|---|
| `app` | 字符串 / `nil` | 宿主应用 exe 名（用来按应用分支） |
| `vk` | 整数 | 虚拟键码（如 `84` = `T`、`0xDE` = `'`、`13` = 回车） |
| `char` | 字符串 / `nil` | 这一键产生的字符（没有则 `nil`） |
| `ctrl` `alt` `shift` `caps` `english_mode` | 布尔 | 修饰键状态 |
| `composing` | 布尔 | 是不是在组句（**这一拍是「引擎处理之前」，所以反映的是上一拍**） |
| `mode` | 字符串 | `"chinese"` / `"english"` / `"disabled"` |
| `highlight` | 字符串 | 这一刻**高亮候选的文本**（按键那一刻窗口里高亮的那条；没有候选时是空串）—— 想自己翻它就用它 |

**组合键脚本**大多不在组句里：这类脚本要读光标前文就用 **`cloudime.context`**（不是 `cloudime.text.*`）——
清单写了 `needs_context = true` 时输入法在**没组句的每个按键之前**现刷一份，所以你按 `Enter` 时它已经包含
**上一次按键**的结果（刚敲的 `1.` 就在里面）—— 自动序号那类脚本靠的就是这个。**能不能读到取决于宿主**：
Windows 11 记事本不给光标前的文字，那里 `cloudime.context` 一直是空串（见[第 11 节](#11-调试与排错)）。
`cloudime.text.*` 那份整篇快照只随**组句**更新，组合键脚本一般用不上（可能 `nil` 或偏旧）。
⚠️ **脚本自己刚上屏的那一行，紧接着读回来可能少最后一个字符**（宿主差异，Firefox 实测）—— 靠它判断
「上一行是不是序号」时要能容忍；`Scripts/lib/example-auto-number.lua` 记着自己刚写的序号来补全。

### `candidates` 的载荷

一张 **1 起**的数组（`list[1]` 是第一个候选），另有一个 `app` 字段：

```lua
cloudime.on("candidates", function(list)
    for index, item in ipairs(list) do
        -- item.text    上屏的文本
        -- item.display 显示的文本（默认 = text）
        -- item.pinyin  词库读音（音节用空格连）
        -- item.weight  Core 算出来的排名权重（快捷候选没有这一项）
        -- item.kind    "chinese" / "english" / "shortcut" / "custom" / "sentence"
    end
end)
```

### 异步回调的载荷

`cloudime.http_get(url, timeout_ms, callback)` 与
`cloudime.http_post(url, body, options, callback)` 的 `callback` 收两个参数：

```lua
callback(result, candidates)
```

- `result`：成功是 `{ url = …, status = 整数, body = 字符串 }`；连不上 / 超时是 `{ url = …, error = 字符串 }`。
- `candidates`：回调**这一刻**的候选表（与 `candidates` 事件那张一样，可能是空的表）。
  这张表里也带 `app`（回调那一刻**聚焦**的宿主程序名）；要按「发请求时那个应用」分支，才需要自己用闭包记下 `event.app`。

## 5. 返回值：动作表

处理函数返回一张表就是「要改的东西」。现在认这几项：

| 键 | 类型 | 意思 | 哪几拍认 |
|---|---|---|---|
| `passthrough` | `true` | 这一键**不吃**、原样交给应用（游戏里抢键就靠它） | 只有 `key` |
| `commit` | 字符串 | 吃掉这一键、**直接上屏**这段文本（当前组句作废） | 只有 `key` |
| `notice` | 字符串 | 候选窗顶部那一行右侧显示一句提示（随这一帧下发，下一次按键清） | 都认 |
| `order` | `{2, 1}` | 候选排序：1 起的下标按这个顺序排到前面，没列到的按原顺序接在后面 | 都认 |
| `display` | `{[1] = "译①"}` | 候选显示：原下标（1 起）→ 显示的文本（**上屏的仍是 `text`**） | 都认 |
| `adjust` | `{["云朵"] = 2.0}` | 加权 / 降权：词文本 → 系数（`> 1` 加权、`< 1` 降权、给 `0` 沉底） | 都认 |
| `online` | 字符串 / 表 / `false` | 候选窗**底部单独那一行**（在线翻译）：写 / 清 | 都认 |
| `theme` | 字符串 / `false` | 换主题：给名字（可带 / 不带 `.json`）**写进配置并重启输入法服务**换上，`false` 回本进程启动时配置里那份（**候选窗开着时等它关掉再换/重启**） | 都认 |

细节：

- **`order` / `display` 不绕开引擎**：它们改的是这一屏候选的布局本身，之后的方向键、数字键、空格、
  鼠标点选都按新的走；真正上屏什么仍然由引擎决定，高亮也跟着原来那个候选走。
- **`adjust` 是一整份替换**：给的是一张新表，没列到的词回到原始词频。它乘在**词频那一层**
  （结构键、读法层级、纠错与联想折扣都不动），所以「覆盖字母少的词」不会因此跑到「覆盖满的词」前面。
  写在 `key` 里当场生效（这一键的排序就用上）；写在 `candidates` / 回调里则影响**下一次查询**。
- **`online`** 三种写法：`online = "文本"`（完成态）、`online = false`（清掉）、
  `online = { text = "…", state = "waiting" | "result" | "error" }`（`state` 不认识按 `result` 处理并记日志）。
  写进去的就是那一行的**原文**：`Ctrl + 反引号` 上屏的是它，「在线 」这个前缀是输入法画的 —— **脚本别自己再加**。
  这一行**支持多行**（文本里有 `\n` 就按几行画，窗口跟着变高），也参与 `Ctrl + 反引号`
  （在线那条排在释义列表最前面、标 `(在线)`），详见 [`docs/design/online-translate.md`](docs/design/online-translate.md)。
- 只有那一次调用的返回值算数：**没有按键的两拍**给 `passthrough` / `commit` 只记一条日志。
- **`theme` 会写配置并重启输入法服务**：给名字就把它写进 `[theme] curr_theme` 再重启服务（和设置页「应用主题」同一套 —— 重启才一定换上，选择也留了下来）；`false` 回**本进程启动时**配置里那份。它与 `cloudime.apply_theme(名字)` 是同一条路，一拍里两个都给了**返回值优先**；
  **候选窗开着（正在组句）时先挂起、等候选窗关掉那一拍才换/重启**（免得正在看候选的人被打断）；
  名字找不到时按缺省主题画、记一条日志，**不写配置也不重启**（免得下次启动按缺省画）。
  ⚠️ 重启会把输入法服务换成新进程，正在打字的程序会断一下再连上（约 1–3 秒）—— 别在「每按一键」的场景里反复换主题。

## 6. `cloudime` 表：API 参考

### 加载期

```text
cloudime.script{ … }               -- 清单，必须最先调一次（见第 3 节）
cloudime.on(事件名, function(载荷) … end)   -- 登记处理函数，只能在加载期调
```

### 随时可用

| 方法 | 说明 |
|---|---|
| `cloudime.log(消息)` | 往运行日志写一行（info 级）。`消息` 要字符串 |
| `cloudime.context` | **字段**（不是函数）：光标前文 —— 应用里已经输入、不在候选窗口里的那段文本。每次派发前刷新，第一次派发前是空串 |
| `cloudime.text.all(上限?)` | 输入框**整篇**文本 → `{ text = …, truncated = … }`；这一拍还拿不到时是 `nil`（见下） |
| `cloudime.text.before(上限?)` | 同上，但只取**光标之前**那一段 |
| `cloudime.text.after(上限?)` | 同上，但只取**光标之后**那一段（从光标那个字符起） |
| `cloudime.clipboard.settext(文本)` | 把 `文本` 写进剪贴板（覆盖原来的内容）。写不进去（被别的程序占着）**报错** |
| `cloudime.clipboard.gettext()` | 读剪贴板里的文本 → 字符串；里面没有文本（图片 / 文件 / 空）给 `nil`；打不开**报错** |
| `cloudime.keyboard.press(组合键)` | 模拟一次组合键，例如 `"ctrl+c"` / `"ctrl+enter"`；支持 `ctrl` / `alt` / `shift` / `win` + 一个普通键。发送前会短暂等待当前按住的修饰键松开，再发给前台窗口 |
| `cloudime.run(命令)` | 起外部程序，**启动就返回、不等子进程**（等同 `os.execute`） |
| `cloudime.caret()` | 请求一次**光标位置**：DLL 在下一次按键处理完之后量插入点，结果以 **`caret` 事件**送来（不是返回值，见下） |
| `cloudime.install_dir()` | 安装目录的绝对路径（`Scripts\` / `Images\` 的上一级）；测试 / 没有安装目录时给 `nil` |
| `cloudime.install_path(相对路径)` | 安装目录里的文件绝对路径（如 `"Images/SVG/打叉.svg"`）；文件不存在给 `nil`，`..` 跑出安装目录**报错** |
| `cloudime.get_theme()` | 当前生效主题的颜色表（`{ candidate = { … }, bar = { … }, tip = { … } }`，颜色是 `#AARRGGBB`）；详见下面「主题颜色」 |
| `cloudime.image.load(路径)` | 读一张图（PNG / BMP / JPG）→ 图像句柄（用户数据），给 `static_image` 的 `image` 字段；同一文件按长度 + 修改时间缓存 |
| `cloudime.get_time()` | 当前**本地时间** → `{ year, month, day, hour, minute, second, weekday, unix }`；`weekday` 1 = 周一 … 7 = 周日，`unix` 是 Unix 秒（标准库的 `os.date` / `os.time` 也能用，这一份是不依赖标准库的稳定形状） |
| `cloudime.get_curr_config()` | 当前**配置**的一张表：**键名与 `config.toml` 一致**、分节同名（`candidate` / `input` / `general` / `theme` / `translate` / `status_bar` / `debugging` / `script` / `word_bank` / `phrase` / `update`）；`theme` 那一节多一个 `available`（可选主题名列表）。每次调用读的都是**当前**那份（配置热加载后跟着变） |
| `cloudime.foreground_app()` | 当前**前台程序**的 exe 名（如 `"notepad.exe"`）；没聚焦任何程序给 `nil`。**随时可问**，不必等事件 —— 事件载荷里的 `app` 也是同一个值（`key` / `candidates` / `time` / 回调都带） |
| `cloudime.apply_theme(名字)` | 换上这份主题：**写进配置 `[theme] curr_theme` 并重启输入法服务**（与设置页「应用主题」同一套；重启才一定换上、选择也留下）。名字可带 / 不带 `.json`；`false` 回本进程启动时配置里那份；名字写错按缺省主题画并记一条日志（**不写配置、不重启**）。**候选窗开着时等它关掉再换/重启** |

`cloudime.clipboard.*` 两条要知道的：

- 剪贴板是**全局资源**：别的程序正占着它时开头几次会失败（Server 短等重试几次，仍不行就报错）→ 建议 `pcall` 接住。
- `gettext()` 读得到**任何程序**刚放进剪贴板的东西（密码管理器复制的密码也算）—— 这是你自己写的脚本、风险自担；
  输入法不拦也不记。空的剪贴板给 `nil`（与「打不开」区分：后者报错）。

`cloudime.keyboard.press(组合键)`：

- 组合键是 `+` 分隔的字符串，修饰键支持 `ctrl` / `alt` / `shift` / `win`，后面跟一个普通键：
  `"ctrl+c"`、`"ctrl+enter"`、`"alt+tab"`；字母大小写不敏感，也支持 `enter` / `tab` / `esc` /
  `space` / `backspace` / `delete` / 方向键 / `home` / `end` / `pageup` / `pagedown` / `f1`–`f12`。
- 发送给**当前前台窗口**。脚本里请先确认目标窗口仍然在前台；脚本窗口自己拿到焦点后，
  组合键会发给脚本窗口。
- 快捷键触发脚本时，Ctrl / Alt 往往还按着；接口会先短暂等待这些修饰键松开（最多约 450 ms），
  再注入组合键，避免把 `Ctrl+Alt+Y` 里的 Alt 带进 `Ctrl+C`。
- 只用于简单的本机快捷操作；它不是自动化测试工具，也不保证所有以管理员权限运行的窗口都接收注入按键。

`cloudime.text.*` 的几件事：

- **上限按显示宽度算**：西文 / 半角 **1**、中文 / 全角 **2**；三个入口的缺省都是 **2^64-1**（等于不限）。
  超上限时**留光标附近**：`before` 从头部丢、`after` 从尾部丢、`all` 是「前一半 + 后一半」（哪边不够就多留
  另一边），返回值里 `truncated = true`。
- **可能给 `nil`**：整篇只有输入法 DLL 读得到（它在应用进程里），Server 只能等它送 ——
  **刚启动、或换了输入框 / 会话（重连）**之后第一次调用是 `nil`，**换一段组句再试**就有。DLL 的读发生在
  起组句那一键的编辑会话里、晚于按键本身，所以**起组句那一键**的脚本拿到的还是上一份。**组句结束不清快照**：
  同一段组句从第二键起、以及**下一段组句的第一键**都能拿到，而**下一段组句起始又会把新的一份送上来** ——
  快照跟着文档走（刚上屏的字也在里面；当前还没上屏的拼音不在）。
  **组合键脚本**（一般不在组句里）读前文请用 **`cloudime.context`**：清单写了 `needs_context = true` 时
  没组句的每个按键之前现刷一份，读到的是**上一次按键之后**的光标前文；`cloudime.text.*` 这份整篇快照仍只随
  **组句起始**更新（组合键脚本一般拿不到）。
- **读不到的地方永远 `nil`**：密码框与声明了私密的输入框（浏览器无痕窗口就是）、不支持读光标外范围的控件，
  以及**只把输入框暴露成「局部上下文」的宿主**（Windows 11 记事本实测：它的 TSF 上下文按选区 / 组句
  圈成局部，整篇读不到，这时一律 `nil`）。整篇最多 20 万个 UTF-16 单元（约 10 万汉字，写死不可配）；
  **没有任何脚本时完全不读**（零开销），有需要整篇的脚本时每段组句起始读一次（成本与文档大小成正比），
  有 `key` 脚本时每个按键前读一次光标前文（很短，几十个字）。

### 光标位置与安装目录

`cloudime.caret()` 是**请求**，不返回位置：Server 在下一次按键的 `KeyResult` 里请 DLL 量一次插入点，
DLL 用只读编辑会话量完发回来，Server 再派一次 **`caret`** 事件（在按键处理函数里调用时，就是这一键）。

```lua
cloudime.on("key", function(event)
    if event.vk == 0x72 then                 -- F3
        cloudime.caret()
    end
end)

cloudime.on("caret", function(pos)
    -- 屏幕坐标，单位是点（100% 缩放下 1 点 = 1 像素）
    -- pos.left / top / right / bottom                       插入点矩形
    -- pos.work_left / work_top / work_right / work_bottom   所在显示器工作区
    -- pos.scale                                             那一屏的缩放（DPI / 96）
    win:show{ x = pos.left, y = pos.top - 30 - 140 }
end)
```

- 拿不到插入点时 DLL 退回鼠标位置（一个零宽的近似矩形），所以事件总会来；不在文本输入里时位置没有意义。
- 这些值已经是**点**，可以直接喂给 `win:show{ x, y }`（不用自己乘 DPI）。
- `cloudime.install_dir()` 给安装目录绝对路径；`cloudime.install_path("Images/SVG/打叉.svg")` 给它下面
  文件的绝对路径（文件不存在给 `nil`，跑到安装目录外**报错**）。`svg` 元素的 `path` 可以直接吃这个结果。

### 主题颜色

`cloudime.get_theme()` 每次调用都读**当前生效**的主题（`cloudime.apply_theme` 换过也跟上），返回：

```lua
{
    candidate = { background, highlight, shadow, pinyin, pinyin_caret,
                  highlight_text, text, highlight_index, index, page_number,
                  badge, translate_meta, translate_learned, translate_fresh, extra },
    bar = { background, icon, shadow },
    tip = { background, icon, shadow },
}
```

颜色都是 `#AARRGGBB` 字符串，可以直接用到元素的 `color` / `tint` / 区域颜色上：

```lua
local theme = cloudime.get_theme()
local background = theme and theme.candidate.background or "#FFFFFFFF"
local text       = theme and theme.candidate.text or "#FF222222"
```

加载脚本的那一刻 Server 还没接上主题钩子，可能给 `nil`；在 `startup` / `key` / `window` 等事件里调用总是有的。

### HTTP（异步，时限必给）

```text
cloudime.http_get(url, timeout_ms, function(result, candidates) … end)

cloudime.http_post(url, body, {
    timeout_ms = 6000,                                  -- 必给，> 0，且不能超过清单里的 timeout
    headers    = { ["Content-Type"] = "application/json" },
}, function(result, candidates) … end)
```

- **时限必给**：给 `0` 或不给直接报 Lua 错、请求不发；超过清单 `timeout` 也当场报错。
- 请求在自己的线程上跑，工人线程只收结果；**超过时限才回来的结果一律丢掉**。
- `http_post` 的表头名字 / 值会当场校验，不合法当场报错（不给后台线程 panic 的机会）。
- 输入法**不解析 JSON**：`result.body` 是原始响应文本，要抠字段就自己用 `string.match` 或带个小解析
  （脚本仓库「小牛候选翻译」里的 `example-niutrans.lua` 用最朴素的
  `:match('"tgtText"%s*:%s*"(.-)"')`）。
- 想让回调生效，清单按 `sync = false, handover = "callback"` 写（两者必须自洽：`true` 配 `"return"`）。

### 候选窗

| 方法 | 说明 |
|---|---|
| `cloudime.candidate.width()` | 候选窗**内容区宽度**（单位点）。不在组句 / 还没画过时是 `nil`，脚本自己回退。折行就按它来 |
| `cloudime.candidate.height()` | 候选窗**内容区高度**（单位点）。不在组句 / 还没画过时是 `nil`；用于把独立窗口摆在候选窗旁边 |
| `cloudime.candidate.redraw()` | 请 Server 把当前这一屏**重算一遍再重画**（会重新派发 `candidates`）。异步回调里改了自己的状态之后用它；在 `candidates` 里调没意义（那一拍本来就在重算） |
| `cloudime.candidate.set_min_width(点)` | 候选窗口的最小宽度（竖排、横排都生效）。范围 `0`–`2000`；**不带参数（`nil`）恢复默认** |
| `cloudime.candidate.set_page_size(5–9)` | 一页几个候选（直接改窗口高度）。不带参数恢复默认 |
| `cloudime.candidate.set_scale(倍数)` | 整个窗口按倍数缩放（等同 `Ctrl + 滚轮`）。范围约 `0.58`–`2.99`；不带参数恢复默认 |

- 单位是**点**：100% 缩放下 1 点 = 1 像素，渲染时随系统 DPI 与缩放一起放大。
- 尺寸三项**只在本次组句内有效**：组句结束回配置 / 滚轮的值（与 `Ctrl + 滚轮` 同级）。
  用户一滚滚轮就由滚轮接管缩放；脚本再设一次又会盖回来。
- `set_min_width` 竖排、横排都生效（展开成网格时窗口宽度由格子决定，不受它影响）。
- 想按窗口宽度排版（折行）：`online` / 提示那一行**左右各有 8 点内边距**，
  所以一行能用的宽度 = `width() - 16 - 前缀宽度`（见 [12.7](#127-按候选窗当前宽度折行长译文多行显示)）。

### 文字排版

**只能在处理函数 / 回调里用**：量尺由 Server 在脚本加载完之后才装上，加载期调 `cloudime.ui.*`
会报「量不了」；同理加载期读 `cloudime.candidate.width()` 总是 `nil`。

```text
cloudime.ui.measure(文本, 字体名?)   -- → { width = 点, height = 点 }
cloudime.ui.measure_tip(文本)        -- 同上，用「底部那一行（翻译 Tip / 在线）」的字体
```

字体名可选 `"pinyin"` / `"candidate"`（缺省）/ `"item_number"` / `"translate"`；写错当场报错。
量出来的是**点**，可以直接喂 `set_min_width`（同一个口径，装得下）。

```text
local box = cloudime.ui.truncate(文本, 行宽 [, 选项])   -- → { text, lines, truncated, width }
```

选项表 `{ mode = "ellipsis"（缺省）| "wrap", max_lines = 5, font = "candidate" }`：

- `"ellipsis"`：单行，放不下就从尾部去字、补「…」；
- `"wrap"`：按行宽贪心折行（**优先在空白处断**，中文这类没有空白的就从任意处断），
  最多 `max_lines` 行（夹到 1–20），剩下的在**末行**补「…」。

返回的 `text` 是最终文本（`"wrap"` 时里面带换行，直接写进 `online` 就按多行显示）、
`lines` 是行数、`truncated` 说明有没有截断过、`width` 是最宽一行的宽度。

### 脚本窗口

脚本可以自己建窗口：**独立窗口**（自己的生命周期）与**辅助窗口**（挂在某块脚本窗口旁边的小面板）。
内容是一份**元素表**（矩形 / 圆 / 线 / 文字 / SVG / 图片 / 路径），窗口由输入法在 UI 线程上按模板建、
用与候选窗同一个渲染器画 —— 脚本不直接画，也卡不住输入。

> **默认规则（2026-10-18）**：凡是创建脚本窗口（独立窗口 / 辅助窗口），除非确实需要
> **运行时动态生成大量控件**、或**整份布局频繁变化**，否则控件布局一律写进 **JSON 布局文件**，
> 用 `cloudime.window.create{ layout = … }` 或 `win:load_layout(…)` 加载；Lua 只负责交互、
> 文本内容、位置和少量属性更新。`win:set_content{…}` 只作为确有必要的例外。
> 这样布局可单独编辑、对比和复用，脚本不会把控件树和业务逻辑混在一起。

```lua
-- 建一块 240 × 120 点的面板（interactive = true 才接鼠标）
local win = cloudime.window.create{
    name = "统计", width = 240, height = 120, interactive = true,
    layout = "Scripts/lib/main.json",  -- 推荐：控件布局在 JSON 里；尺寸以布局文件为准
    -- background = false  -- 不要输入法画的面板底色 / 圆角，改由元素表自己画背景
}

-- 例外：确有需要时，才换一整份内联内容；坐标与尺寸都是点，左上角原点
win:set_content{
    { type = "rect", x = 0, y = 0, w = 240, h = 32, radius = 8, color = "#33111111" },
    { type = "text", x = 12, y = 6, text = "今日 12 条", size = 16, color = "#FF222222" },
    { type = "line", from = { x = 0, y = 32 }, to = { x = 240, y = 32 }, thickness = 1, color = "#22000000" },
}

win:show{ x = 100, y = 100 }      -- 屏幕坐标（点）；不写 = 上次的位置 / 光标旁
-- win:hide()                     -- 收起（内容留着）
-- win:close()                    -- 销毁

-- 辅助窗口：挂在上面那块窗旁边（贴宿主自动选边）
local aux = cloudime.window.create_auxiliary{ owner = win, width = 160, height = 120, gap = 16 }
aux:set_content{ { type = "text", x = 8, y = 8, text = "附注", size = 12, color = "#FF666666" } }
aux:show()
```

`background = false` 时窗口只有一块透明的内容区：输入法不画面板底色 / 圆角，脚本用一整块圆角
`rect` 当背景；阴影仍按 `shadow` 决定（再设 `shadow = false` 就连阴影也不画），窗口长什么样完全由脚本决定。

**窗口句柄上的方法**：

| 方法 | 说明 |
|---|---|
| `win:set_content{ … }` | 换一整份内容（整份替换；`width` / `height` 可省，缺省用建窗口时那份） |
| `win:load_layout(path, data?)` | 读一份 JSON 布局文件并换成整份内容；`data` 是 v2 页面树的数据上下文 |
| `win:set_data(data)` | 用当前数据重跑最近一次 v2 布局（不重新读文件）；普通控件按控件名更新 |
| `win:get_text(name)` | 读某个 `text_edit` / `text_box` 最近一次文本；控件不存在 / 还没有文本时给 `nil` |
| `win:set_text(name, text)` | 设置某个 `text_edit` / `text_box` 的文本（按控件名找，整份替换、光标放到末尾） |
| `win:update_content(name, content)` | 更新静态控件显示内容：`text_block` 传字符串；`label` 传 `{ { text = …, color = … }, … }`，每段都必须带颜色。控件仍是静态的，不参与鼠标 / 键盘交互 |
| `win:set_button_background(name, color)` | 改一个 `flat_svg_button` 的背景色（normal / hovered / pressed 一起改，SVG 染色不变）；颜色写 `#RRGGBB` 或 `#AARRGGBB` |
| `win:set_selection_background(name, color)` | 改一个 `text_edit` / `text_box` 的选区背景色 |
| `win:select_all(name, all?)` | 全选；`all` 缺省 `true`，传 `false` 取消选择（选区收在末尾） |
| `win:replace(name, find, text)` | 把文本里的 `find` **全部**替换成 `text`（`find` 为空不做事） |
| `win:clear(name)` | 清空某个 `text_edit` / `text_box` |
| `win:show{ x = …, y = … }` | 显示；坐标是**内容区左上角**的屏幕坐标（点）。不写就沿用上次的位置，没有过就摆在光标右下 |
| `win:hide()` | 收起（不销毁） |
| `win:close()` | 销毁；独立窗口连名下资产一起放掉 |
| `win:focus(true / false)` | 独立窗口：拿 / 放**键盘焦点**（之后 `key` 载荷里带 `window` / `window_id`）；辅助窗口：把逻辑焦点指定给它 / 交还给宿主 |
| `win:focus_text(name)` | 把键盘焦点交给某个 `text_edit` / `text_box` 宿主（面板打开后自动聚焦输入框用） |
| `win:combine_region(name, 形状)` | 把形状并进已有区域（只改命中、不重画） |
| `win:prune_region(name, 形状)` | 从已有区域里挖掉形状（只改命中、不重画） |
| `win:region_contains(name, x, y)` | 点是否在区域里（含边界）；没有这个区域给 `nil` |
| `win:region_relation(a, b)` | 两块区域的关系：`{ relation = "disjoint" / "intersect" / "contains" / "coincide", contains = "a" / "b" }`；没有区域给 `nil` |

`win:update_content(name, content)` 只更新 JSON 布局里有 `name` 的 `label` / `text_block`；它们仍不参与
鼠标 / 键盘交互。`text_block` 传字符串；`label` 必须传带颜色的 runs 表，例如
`{ { text = "提示", color = "#FFCC3333" } }`，不会猜颜色。内联 `set_content` 创建的静态元素没有
控件名表，不能用这个方法按名字更新。

**JSON 布局文件**：`win:load_layout(path)` 或 `cloudime.window.create{ layout = path }` 可以直接读一份前台 UI 文件，
控件字段与上面的元素表一致（`type` 必写，其余没写的项用元素表缺省值）。顶层格式：

```json
{
  "width": 500,
  "height": 360,
  "controls": [
    { "name": "background", "type": "rect", "x": 0, "y": 0, "w": 500, "h": 360, "radius": 12, "color": "#FFFFFFFF" },
    { "name": "close", "type": "flat_svg_button", "x": 468, "y": 8, "w": 24, "h": 24,
      "svg_path": "Images/SVG/打叉.svg", "shape": "round_rect", "radius": 4,
      "normal": { "background": "#00000000", "svg": "#FF000000" },
      "events": { "mouse_clicked": "close_window" } }
  ],
  "regions": [
    { "name": "content", "x": 0, "y": 0, "w": 500, "h": 360, "drag": false }
  ]
}
```

- `controls` 与 `elements` 等价；`width` / `height` / `regions` 可省（尺寸缺省用窗口那份）。根也可以直接写控件数组。
- **每个控件必须写 `name`**：英文字母开头，只能包含英文字母 / 数字 / 下划线，最长 255 个字符，
  同一份布局里唯一（区分大小写），且不能是 Rust 关键字。这个名称会作为控件标识符出现在
  窗口事件的 `event.control` 里。
- 动态控件（`flat_svg_button` / `text_edit` / `text_box` / `hscroll_bar` / `vscroll_bar`）可以写
  `events = { 事件名 = "处理器名" }`；事件名用
  `click` / `right_click` / `hover` / `leave` / `wheel` / `focus` / `blur` / `mouse_enter` / `mouse_leave` /
  `mouse_pressed` / `mouse_released` / `mouse_clicked` / `mouse_double_clicked` / `text_changed` /
  `key_pressed` / `focused` / `unfocused` / `value_changed`（`text_edit` / `text_box` 也接受 `on_key_pressed` / `onKeyPressed`、
  `on_focused` / `onFocused`、`on_unfocused` / `onUnfocused`；滚动条也接受 `on_value_changed`），
  处理器名由脚本自己分发。
  窗口事件载荷里会多一个 `event.handler`（没声明时是 `nil`），例如：

  ```lua
  cloudime.on("window", function(event)
      if event.handler == "close_window" then
          win:close()
      end
  end)
  ```

  没有声明 `events` 的普通交互仍按 `event.region` / `event.element` + `event.action` 分发。
- 布局文件里的 `svg_path` / `img.path` 先按布局文件所在目录找，再逐级向上找；因此放在
  `Scripts/lib/` 的布局可以直接写 `Images/...`（落到安装根目录），也可以把素材放在布局旁边。
  改布局文件后要重新 `load_layout` 或重启输入法服务；不会自动监听文件变化。

**页面树 v2（实验）**：布局文件顶层写 `"version": 2` 时，使用页面树 / 布局算法，而不是上面的绝对坐标。
完整契约见 [`docs/design/ui-layout.md`](docs/design/ui-layout.md)；脚本侧只需要记住：

```lua
local win = cloudime.window.create{
    layout = "Scripts/lib/list.json",
    data = { items = { { title = "A" }, { title = "B" } } },
}
-- 或：win:load_layout("Scripts/lib/list.json", { items = ... })
win:set_data{ items = { { title = "C" } } }
```

子页面用 `subpage` 节点 + `repeat` / `key`，模板里写 `{{item.title}}`，实例子页面内控件用局部名事件
（例如 `event.control = "title"`），完整路径在 `event.control_path`（例如 `rows[1].title`）；
`subpage.required = { "title", "meta" }` 可声明每个 item 必填的数据字段，缺一项整份布局失败。
v2 目前是实验接口，细节和限制以设计文档为准。

**元素表**（坐标 / 尺寸都是点，超出内容区的部分被裁掉）：

| `type` | 字段 |
|---|---|
| `rect` | `x` / `y` / `w` / `h` 必给；`radius`（圆角，0 = 直角）；`color` 必给 |
| `circle` | `cx` / `cy` / `radius` / `color` |
| `line` | `from` / `to`（`{ x = …, y = … }` 或 `{ x, y }`）、`thickness`（缺省 1 点，100% 缩放下 1 像素）、`color` |
| `text` | `x` / `y`（行框左上角）/ `text` / `size`（点，缺省 16）/ `font`（`candidate` 缺省、`pinyin`、`item_number`、`translate`）/ `color`（缺省黑）/ `italic` |
| `svg` | `x` / `y` / `size`（见方）/ `source`（整段 SVG 文本）**或** `path`（SVG 文件，常用 `cloudime.install_path`）/ `tint`（可选，按 alpha 染色） |
| `img` | `x` / `y` / `w` / `h` / `path`（PNG / BMP / JPG；相对路径按安装目录算） |
| `path` | `commands` = `{ {"M",x,y}, {"L",x,y}, {"Q",cx,cy,x,y}, {"C",…}, {"Z"} }`、`color` |
| `label` | `x` / `y` / `size` / `font`（缺省 `candidate`）；`runs = { { text = "云朵", color = "#FF0000FF" }, … }`，每段可单独着色 |
| `text_block` | `x` / `y` / `text`（可含 `\n`）/ `size` / `font`（缺省 `item_number`）/ `line_spacing`（点，缺省 2）/ `color`（整块同色） |
| `static_shape` | `shape` = `line` / `rect` / `round_rect` / `circle` / `path`；字段与该形状同名（`from`/`to`/`x`/`y`/`w`/`h`/`radius`/`cx`/`cy`/`commands`）+ `color` |
| `static_image` | `x` / `y` / `w` / `h`（可省，缺省图片像素尺寸）+ `image`（`cloudime.image.load` 的句柄）或 `path`（PNG / BMP / JPG） |
| `panel` | `x` / `y` / `w` / `h` / `radius`（缺省 6）/ `color`（**边缘**颜色，缺省 `#FFE0E0E0`）/ `border_width`（缺省 1 点，100% 缩放下 1 像素）/ `fill`（可选填充，不写 = 无填充）/ `children = { …元素… }`；子控件用面板内相对坐标 |
| `flat_svg_button` | `x` / `y`；`w` / `h` 可省（缺省取 SVG 画板）；`svg`（整段 SVG）或 `svg_path`（文件）；`shape = rect / round_rect / circle / path` + `radius`；`name`（自动区域名）；`normal` / `hovered` / `pressed = { background = …, svg = … }`；`events = { 动作名 = "处理器名" }` |
| `text_edit` | `x` / `y` / `w` / `h`；`radius`（缺省 `h/2`）；`read_only` / `passkey`；`back_color`（背景）/ `edge_color`（边框）/ `edge_width`（缺省 1 点，100% 缩放下 1 像素；0 不显示）；`text_color` / `cursor_color` / `sel_back_color` / `sel_text_color` / `fill_color`；`svg` / `svg_path` + `svg_align = left / right`；`font_name` / `font_size` / `font_flag`；`text` 初始文本；`font`（缺省 `candidate`）；`name`（自动区域名）；`events`；旧名 `fill` / `color` / `border_width` / `caret_color` / `selection_background` / `selection_color` / `svg_color` / `size` 仍兼容 |
| `text_box` | 多行编辑框：`x` / `y` / `w` / `h`；`radius`（缺省 6）；`read_only`；`back_color` / `edge_color` / `edge_width`（缺省 1 点，100% 缩放下 1 像素；0 不显示）；`text_color` / `cursor_color` / `sel_back_color` / `sel_text_color`；`font_name` / `font_size` / `font_flag`；`auto_line_wrap`（缺省 true，启用时无水平滚动条）/ `line_spacing`（u16，缺省 8）；`text` / `font` / `name` / `events`；没有 `passkey` 和 SVG 属性；`text_edit` 的文本方法 / 事件都适用 |
| `hscroll_bar` | 水平滚动条：`x` / `y` 必给；`w` / `h` 可省，缺省都是 8（水平条通常写长度 `w`，只写 `h` 就是 8 点厚）；`value` / `min_value` / `max_value`（非负整数，缺省 0 / 0 / 100）/ `step`（滚轮和点轨道的步长，u64，缺省 1，不能为 0）；`background_color`（兼容 `back_color` / `background` / `color` / `fill`）/ `thumb_color`（兼容 `slider_color` / `thumb` / `highlight`）；`name`；`events` |
| `vscroll_bar` | 垂直滚动条，字段同 `hscroll_bar`，`w` / `h` 缺省都是 8；也接受拼写 `vhscroll_bar` / `vhsroll_bar` |

颜色写 `"#RRGGBB"` / `"#AARRGGBB"` / `"#RGB"`，或 `{ r = …, g = …, b = …, a = … }`（0–255）。

**静态控件**（`label` / `text_block` / `static_shape` / `static_image` / `panel`）只是显示内容：
**没有命中框**，不参与 hover / click / `event.element`。面板只做分组：`children` 的坐标相对面板左上角，
绘制时整体平移；面板缺省**无填充**，只画 `#FFE0E0E0`、1 点的圆角边框（要底色写 `fill`）。

```lua
win:set_content{
    width = 240, height = 120,
    { type = "panel", x = 8, y = 8, w = 224, h = 104,
      children = {
          { type = "label", x = 12, y = 12, size = 18,
            runs = { { text = "云朵", color = "#FF2F6FED" },
                     { text = "输入法", color = "#FF222222" } } },
          { type = "text_block", x = 12, y = 44, size = 12, line_spacing = 4,
            text = "第一行\n第二行", color = "#FF666666" },
          { type = "static_shape", shape = "round_rect", x = 12, y = 78,
            w = 80, h = 20, radius = 6, color = "#FF4A90E2" },
      } },
}
```

`static_image` 的 `image` 句柄：`local icon = cloudime.image.load(cloudime.install_path("Images/a.png"))`，
之后 `{ type = "static_image", x = 8, y = 8, w = 32, h = 32, image = icon }`。句柄是用户数据，Lua 丢掉它
之后已经放进内容的位图仍由元素持有；`path` 直接给文件也行（只解码一次，按文件长度 + 修改时间缓存）。
`w` / `h` 可省，缺省用图片自身像素尺寸。

图像句柄上的方法（**直接改句柄里的位图**，下次 `set_content` 生效；坐标 / 尺寸都是图片像素）：

```lua
icon:scale(0.5)                 -- 按比例；也可以 icon:scale(64, 64) 指定宽高
icon:rotate(90)                 -- 顺时针 90 / 180 / 270
icon:flip("horizontal")         -- 或 "vertical"
icon:to_gray()                  -- BT.601 灰度（0–255），alpha 不变
icon:use_path_mask({ { "M", 0, 0 }, { "L", 16, 0 }, { "L", 16, 16 }, { "L", 0, 16 }, { "Z" } })
-- use_path_mask 也接受 { commands = { … } }；只保留路径内部（非零环绕）
```

### 扁平 SVG 按钮（动态控件）

`flat_svg_button` 把「区域 + 背景形状 + SVG」打包成一个按钮，鼠标状态由窗口自动维护：

```lua
{ type = "flat_svg_button", name = "close", x = 152, y = 8,
  svg_path = cloudime.install_path("Images/SVG/打叉.svg"),
  shape = "round_rect", radius = 3,
  normal  = { background = "#00000000", svg = "#FF000000" },
  hovered = { background = "#FFFF3300", svg = "#FFFFFFFF" },
  pressed = { background = "#FFCC2900", svg = "#FFFFFFFF" } }
```

- **区域就是背景形状**：自动注册一个 `name`（缺省 `flat_svg_button_N`）、`drag = false` 的区域；
  点击事件里 `event.region` 是这个名字，`event.element` 是按钮下标。
- `w` / `h` 不写时，背景尺寸取 **SVG 画板尺寸**（`viewBox` / 宽高）；只写一个就按画板比例补另一个。
  `shape` 缺省 `rect`，`round_rect` 的 `radius` 缺省 3；`circle` 缺省内切圆。
- 背景先画、SVG 后画（居中，取短边）；颜色只在三个状态里取，窗口在 hover / 按下 / 松开时自动切换并重画。
- **SVG 可以随时换**：重新 `set_content` 时给新的 `svg` / `svg_path` 即可（例如 hover 时把叉换成勾）。
  按钮状态会从 normal 重新开始，下一次鼠标移动再按位置切换。

**按钮事件**（仍走 `cloudime.on("window", …)`，用 `event.action` 区分）：

| action | 什么时候 |
|---|---|
| `mouse_enter` | 鼠标进入按钮区域 |
| `mouse_leave` | 鼠标离开按钮区域 |
| `mouse_pressed` | 在按钮上按下左键；载荷带 `ctrl` / `alt` / `shift`，任意组合都派 |
| `mouse_released` | 在按钮上松开左键 |
| `mouse_clicked` | 单击（没有拖拽、**没有修饰键**） |
| `mouse_double_clicked` | 双击（第二次单击，仍要求没有修饰键） |

`event.element` / `event.region` 都指向这个按钮；`mouse_clicked` 在每次单击都派，双击时第二次会依次收到
`mouse_clicked` 和 `mouse_double_clicked`。按下 / 松开 / 单击 / 双击的载荷都带 `ctrl` / `alt` / `shift`。

### 单行编辑框（`text_edit`）

```lua
{ type = "text_edit", name = "search", x = 24, y = 172, w = 320, h = 32,
  svg_path = cloudime.install_path("Images/SVG/打勾.svg"), svg_align = "left",
  text = "在这里输入…", size = 14 }
```

- 背景缺省是**胶囊**（`radius = h/2`）、透明填充、边框缺省 `#FFC8C8C8`，
  线宽缺省 1 **物理像素**（不随缩放变化）。
- 属性（新名字；括号里是兼容旧写法）：
  - `read_only`：是否只读。
  - `passkey`（`password`）：密码输入，显示 `*`，真实文本仍可由 API 读写。
  - `back_color`（`fill`）：背景颜色。
  - `edge_color`（`color`）：边框颜色。
  - `edge_width`（`border_width`）：边框粗细，缺省 1 像素且不缩放；显式写数值时按点缩放，`0` 不显示。
  - `text_color`：正常文本颜色，缺省主题正文色。
  - `cursor_color`（`caret_color`）：光标颜色。
  - `sel_back_color`（`selection_background`）：选中文字背景。
  - `sel_text_color`（`selection_color`）：选中文字颜色。
  - `fill_color`（`svg_color`）：SVG 图形填充 / 染色。
  - `font_name`：字体名称；不写用字体槽位的主题字体。
  - `font_size`（`size`）：字号（点）；不写用主题字号。
  - `font_flag`：字体标志，`0` 正常、`1` 加粗、`2` 斜体、`4` 下划线、`8` 删除线，可相加；
    例如 `3` = 加粗 + 斜体，只接受 `0–15`。
- 文本区由 **RichEdit** 承载（Windowless RichEdit，不是系统 `EDIT` 控件），点一下就能用鼠标 / 键盘输入，
  支持选区、剪贴板和 **CloudIME / 系统 TSF 输入法**。`text` 是初始文本；`font` 仍是字体槽位
  （`candidate` 缺省 / `pinyin` / `item_number` / `translate`），`font_name` 覆盖字体族。
- 没有 SVG：文本区宽 `w - h`，在背景里水平居中。
- 有 SVG：SVG 按 `h` 缩放，文本区宽 `w - h - svg宽度 - 4`，放在图标旁边；`svg_align` 决定图标在左 / 右。
- 自动区域名取 `name`（缺省 `text_edit_N`），`drag = false`；点文本区边距也会把焦点交给 RichEdit。
- **嵌套面板**：`panel.children` 里的 `text_edit` 也会建宿主；坐标按所有父面板偏移累加，
  自动区域名用 `name`（没写时按路径生成 `text_edit_0_1` 这种合法名字）。
- **横向滚动**：单行文本超过编辑区时，内容跟随光标横向滚动；点击 / 拖选按滚动后的位置命中。
- **读写文本**：`win:get_text(name)` 读最近文本，`win:set_text(name, text)` 整份替换（排队到 UI 线程）。
  宿主每次文字变化会派 `text_changed` 事件，载荷带 `event.control` / `event.text`；脚本收到后
  缓存已在运行时刷新，所以事件处理函数里再 `get_text` 能读到新值。
- **编辑方法**：`win:select_all(name[, false])` 全选 / 取消选择；`win:replace(name, find, text)` 把
  `find` 的全部出现替换成 `text`；`win:clear(name)` 清空。
- **事件**：`on_key_pressed` / `onKeyPressed`（`event.key` 虚拟键码，`event.modifier` 位：
  1 shift / 2 ctrl / 4 alt / 8 win，可相加）；`on_focused` / `onFocused` 进入编辑；
  `on_unfocused` / `onUnfocused` 离开编辑。可以在 JSON 的 `events` 里声明，也可以直接按
  `event.action == "key_pressed" / "focused" / "unfocused"` 和 `event.control` 分发。

### 多行编辑框（`text_box`）

```lua
{ type = "text_box", name = "notes", x = 24, y = 300, w = 680, h = 150,
  auto_line_wrap = true, line_spacing = 8, font_size = 16,
  text = "第一行\n第二行\n内容超过可视区域时右侧出现滚动条。" }
```

- 背景是普通圆角矩形（`radius` 缺省 6），没有 SVG，也没有 `passkey`。
- 边框缺省 `#FFC8C8C8`，线宽缺省 1 **物理像素**（不随缩放变化）；`edge_width` 显式写数值时按点缩放。
- `auto_line_wrap` 缺省 `true`：打开 RichEdit 的自动换行并隐藏水平滚动条；设为 `false` 可横向滚动。
- `line_spacing` 为 `u16`，缺省 `8`，表示两行的行距（点，100% 缩放下等于像素）。
- 文本区由 Windowless RichEdit 多行排版；内容超出可视区时才显示内部水平 / 垂直滚动条，
  滚动条厚 8 点、与文本区间隔 4 点。拖滑块、点轨道和滚轮都能滚动；内部滚动条不是脚本控件，
  不派 `value_changed`。
- 文本方法、`text_changed` / `key_pressed` / `focused` / `unfocused` 事件与 `text_edit` 相同。
  嵌套在 `panel.children` 里的 `text_box` 也会建宿主并接收文本事件。

**滚动条（`hscroll_bar` / `vscroll_bar`）**

```lua
{ type = "hscroll_bar", name = "hours", x = 24, y = 300, w = 320,
  value = 40, min_value = 0, max_value = 100,
  background_color = "#FFE0E0E0", thumb_color = "#FF4A90E2",
  events = { value_changed = "hours_changed" } }
```

- 背景和滑块都是胶囊形状、纯填充无边框；背景就是鼠标命中区域。
  不写颜色时，背景缺省 `#FFC8C8C8`、滑块缺省 `#FF888888`；多行编辑框的内部滚动条也用
  这一组缺省颜色。
- `value` / `min_value` / `max_value` 是非负整数，缺省 `0 / 0 / 100`；`step` 是滚轮 / 点击轨道的步长
  （u64，缺省 1，不能为 0）；解析时把 `value` 夹到值域。
- 滑块长度为轨道 1/4（至少一个厚度），点击滑块可以拖动；点击**非滑块**区域按方向改 `step`
  （左 / 上减，右 / 下加），等价于滚轮那一格。滚轮 / 点轨道这类非拖拽改变有约 140 ms 的平滑过渡，
  拖动滑块时则立即跟手。
- 鼠标滚轮落在滚动条上时按 `step` 改值（向上滚减、向下滚加），并吞掉这次滚轮；值变化派
  `value_changed`，载荷是 `event.control` / `event.value`，也可以声明 `events.value_changed` 处理器。
- 垂直滚动条同字段；`vhscroll_bar` / `vhsroll_bar` 是兼容拼写。

**区域（region）**：一块可以被鼠标**点击 / 拖拽**的范围 —— 元素只管画，交互看区域：

```lua
win:set_content{
    width = 220, height = 96,
    regions = {
        { name = "counter", x = 12, y = 36, w = 196, h = 44, radius = 10 },
        { name = "label",   x = 12, y = 6,  w = 196, h = 24, drag = false },
    },
    { type = "rect", x = 12, y = 36, w = 196, h = 44, radius = 10, color = "#FF4A90E2" },
    -- …元素照常写…
}
```

- `name` 必写、同一份内容里唯一；`drag` 缺省 `true` = 按住它拖会移动整块窗口，
  `false` = 按住它不拖窗口（点击照常）。
- **形状不必是矩形**。不写 `shape` 时按字段猜（有 `commands` 当路径、有 `cx` / `cy` 当圆、其余当矩形），
  也可以显式写 `shape`：
  - `shape = "rect"` / `"round_rect"`：`{ x, y, w, h, radius }`，`radius > 0` 就是圆角矩形；
  - `shape = "circle"`：`{ cx, cy, radius }`；
  - `shape = "path"`：`{ commands = { {"M",…}, … } }`，与 `path` 元素同一套命令（自动闭合填充）。

  ```lua
  regions = {
      { name = "dot",    shape = "circle", cx = 60, cy = 60, radius = 24 },
      { name = "corner", x = 0, y = 0, w = 120, h = 120, radius = 12 },
      { name = "arrow",  shape = "path",
        commands = { {"M", 10, 80}, {"L", 60, 30}, {"L", 110, 80}, {"Z"} } },
  }
  ```
- 鼠标事件里的 **`event.region`** 就是命中的区域名；没命中 / 没声明区域是 `nil`。
- 后写的区域盖住先写的（与元素绘制顺序一致）。没声明 `regions` 时仍按元素命中（`event.element`）。
- **合并（combine）/ 剪除（prune）**：区域声明里可以带这两个字段（一个形状，或形状数组）：

  ```lua
  regions = {
      { name = "keys", x = 0, y = 0, w = 200, h = 120,
        combine = { x = 200, y = 0, w = 40, h = 120 },       -- 并进来（绿框并蓝框）
        prune   = { { x = 20, y = 20, w = 30, h = 30 } },    -- 挖掉（红框减黑框）
      },
  }
  ```

  命中判定：**先看 `prune`**（任一剪除形状命中就不算命中），再看原始形状或 `combine` 里的任一形状。
  运行时也能改**已有区域**：`win:combine_region("keys", { x, y, w, h })` /
  `win:prune_region("keys", { x, y, w, h })` —— 只改命中、不重画；下一次 `set_content` 会重置成
  内容里声明的那份。
- **查询区域**：`win:region_contains(name, x, y)` 判断点是否在区域里（与鼠标命中同一套判断，含边界），
  没有这个区域给 `nil`；`win:region_relation(a, b)` 判断两块区域的关系，返回
  `{ relation = … , contains = … }`（没有区域给 `nil`）：

  | `relation` | 含义 |
  | --- | --- |
  | `"disjoint"` | 相离（只贴一条边也算相离） |
  | `"intersect"` | 相交（有面积上的重叠） |
  | `"contains"` | 内含；`contains = "a"` 表示前者包住后者，`"b"` 反过来 |
  | `"coincide"` | 重合 |

  ```lua
  if win:region_contains("dot", event.x, event.y) then … end
  local rel = win:region_relation("dot", "corner")
  if rel and rel.relation == "contains" and rel.contains == "a" then … end
  ```

  查的是脚本这一侧的区域镜像（`set_content` / 合并 / 剪除之后同步更新），不占 UI 线程。

**命中按形状**：圆角矩形抠掉四个角、圆按边界、线段按粗细、路径按非零环绕填充；文字 / 图像 / SVG 按外框。
`img` 按路径**缓存**解码结果（最多 8 张 / 32M 像素，文件改过下一次 `set_content` 生效），同一张图不用反复解码。

**限额**（超了当场报 Lua 错误，窗口保持原样）：一份内容最多 512 个元素；一段文字最多 4096 字符；
图片不超过 4096 × 4096；内容区尺寸夹到 8–2048 点。

**鼠标 / 滚轮 / 拖动 / 键盘**（`interactive = true` 的窗口才会收到）：

```lua
cloudime.on("window", function(event)
    -- event.window / event.id      哪块窗口（名字 / 编号）
    -- event.action                 "click" / "right_click" / "hover" / "leave" / "wheel" / "focus" / "blur" / "text_changed" / "key_pressed" / "focused" / "unfocused" / "value_changed"
    -- event.x / event.y            内容区坐标（点）
    -- event.element               命中的元素（内容数组里 1 起的下标；没命中是 nil）
    -- event.region                命中的区域名（内容里的 regions；没命中是 nil）
    -- event.control               命中的控件名（布局文件里的 name；子页面实例给局部名，没命中是 nil）
    -- event.subpage / event.instance / event.item_index / event.item_key / event.control_path
    --                              v2 子页面实例信息；普通窗口事件是 nil
    -- event.handler               区域 / 布局声明的事件处理器名（events 表；没声明是 nil）
    -- event.text                  text_changed 时的完整文本；其它事件是 nil
    -- event.key                   key_pressed 的虚拟键码；其它事件是 nil
    -- event.modifier              key_pressed 的修饰键位（1/2/4/8 可相加）
    -- event.value                 value_changed 的新值；其它事件是 nil
    -- event.delta                 滚轮量（一格 ±120，正数往上；只有 wheel 有）
    if event.action == "wheel" then
        count = math.max(0, count + math.floor(event.delta / 120))
        win:set_content{ { type = "text", x = 8, y = 8, text = "计到 " .. count } }
    elseif event.action == "click" then
        win:set_content{ { type = "text", x = 8, y = 8, text = "点到了 " .. tostring(event.element) } }
    end
end)
```

- `click` / `right_click` 在按键抬起时派；`hover` 只在停到**另一个元素**上时派一次；`leave` 是鼠标离开窗口。
- **拖动**：按住左键在窗口上拖就移动整块窗口（超过 4 像素才算拖，拖过就不算点击），辅助窗口跟着摆；
  拖到另一块 DPI 不同的显示器上会按新 DPI 重画。
- **键盘焦点**：点一下窗口（或 `win:focus()`）它就拿到键盘焦点，`window` 事件收到 `focus`；
  点别的窗口 / 收起 / 关闭时收到 `blur`。拿到焦点期间 `key` 载荷多两个字段 `window` / `window_id`：

  ```lua
  cloudime.on("key", function(event)
      if event.window_id ~= win.id then return end    -- 不是这块面板的键
      if event.vk == 0x1B then win:hide() end         -- Esc 收起
  end)
  ```

  ⚠️ 窗口不抢系统的键盘焦点：能收到哪些键由清单决定（`combination_key` 的 `combination_modifiers` +
  `keys` 那一组），**不是**有焦点就能拿到所有键；需要文字输入时放一个 `text_edit`。
- `window` 事件**不看清单的 `trigger_condition`**（与 `startup`、HTTP 回调一样，所有脚本都能收）。
- 辅助窗口默认鼠标穿透：`create_auxiliary{ interactive = true }` 之后还要 `aux:focus(true)` 才真正接鼠标
  （焦点一组里只有一个：给了它，宿主那块窗就让开）。
- 返回值按「没有按键的那一拍」处理：`notice` / `online` / `theme` / `adjust` 认，`passthrough` / `commit` 忽略。

窗口活到 `close` 或输入法服务退出；`hide` 不销毁。还没有多行编辑、富文本，以及拖元素（滑块之类）；
拖动只移动整块窗口。

## 7. 标准库与运行环境

- 语言是 **Lua 5.1**（LuaJIT）。`string` / `table` / `math` / `os` / `io` / `package` 都在；
  **没有 `debug` 库**；LuaJIT 的 `bit` 库可用（源码仓库 `Scripts\lib\md5.lua` 就靠它）。
- 脚本的 **JIT 关掉**了（LuaJIT 编成 trace 之后不再检查调试钩子，预算就失效了），`jit.on` 也收掉了。
- 刻意改过的几处（都是「会退进程 / 会卡住」的入口）：

| 原本 | 现在 |
|---|---|
| `os.exit(…)` | 只记一条日志，不退进程（退了所有应用都会断线） |
| `os.execute(命令)` | 转发到 `cloudime.run`：**启动就返回、不等子进程**（`os.execute('notepad.exe')` 照常能开程序） |
| `io.popen(…)` | 直接报错（它是同步的，会把输入法卡住）。发网络请求用 `cloudime.http_get` |
| `io.stdin` | 关掉（Server 由控制台启动时 `io.read()` 会一直等输入） |
| `coroutine.create` / `coroutine.wrap` | 直接报错（Lua 的预算钩子每个线程一份，协程里的死循环拦不住） |

**没用 `debug` / 没协程 / 关 JIT** 这三条是同一件事的三个角度：让「指令预算」这道闸真的拦得住。

## 8. 稳定性闸门

脚本卡住堵的是**所有应用**，所以有四道闸：

| 闸 | 做法 | 防住什么 |
|---|---|---|
| 指令预算 + 墙钟 | 每次调用按**该脚本清单**里的 `budget`（缺省一亿条，只能收窄）与 `timeout`（必写）上紧限额；超了从钩子里报错中断**这一次** | 死循环、跑太久的脚本。中断只毁这一次：记日志、调它的 `on_error`，按键照旧走引擎 |
| 脚本的 JIT 关掉 | `jit.off()`，并把 `jit.on` 收掉 | 「热循环被 JIT 编走、预算钩子失效」 |
| 没有 `debug` 库 | mlua 的 `Lua::new()` 只开安全子集 | 脚本主动 / 无意摘掉预算钩子 |
| 不给协程 | `coroutine.create` / `wrap` 报错 | 「把死循环藏在协程里」 |

**允许的「等」只有 `cloudime.http_get` 与 `cloudime.http_post`**（都在自己的线程上跑、时限必给）。
在同步处理函数里自己等 = 卡住所有应用。

已知的残余风险（没法在 Lua 层便宜地堵）：`io.open` 打开**设备 / 管道路径**（`\\.\pipe\…`、`COM1:`）
与**慢速网络盘**上的读写仍可能长时间阻塞。路径是脚本自己写死的 —— 自担。

## 9. 状态与持久化

- **Lua 全局变量**：跨事件保留（`startup` 里初始化、`key` / 回调里用），
  但**跨应用共用**、**Server 重启就没了**（重启输入法服务等于清空）。
- 想真持久化就写文件，**写数据目录**：`%APPDATA%\CloudIME\` 下（那个目录一定在）。
  别写安装目录：普通用户可能写不进去，升级 / 卸载还会删。

```lua
local file = os.getenv("APPDATA") .. "\\CloudIME\\我的脚本.data"

local function load_lines()
    local handle = io.open(file, "r")
    if not handle then return {} end
    local lines = {}
    for line in handle:lines() do
        lines[#lines + 1] = line
    end
    handle:close()
    return lines
end

local function save_lines(lines)
    local handle = io.open(file, "w")
    if not handle then return end
    for _, line in ipairs(lines) do
        handle:write(line, "\n")
    end
    handle:close()
end
```

## 10. 设置页、启用与禁用

设置 → 脚本：

- **脚本列表**：文件名 / 介绍（清单里的 `description`，扫文本取出来、不执行脚本）/
  启用开关 + 「删除此脚本」+ 「编辑此脚本」（用记事本打开）。
- **新建脚本**：在 `Scripts\` 下建一个不重名的文件（`script.lua` → `script-2.lua` …）写入模板并打开。
- 开关写进配置 `[script] disabled = ["a.lua"]`（`config.toml`），Server **启动时**按它跳过；
  禁用只是不加载，文件还在。
- 列表**不显示 `template.lua`**。
- **卸载会删掉 `{app}` 整棵**，包括 `Scripts\` —— 自己的脚本记得在别处留一份。

## 11. 调试与排错

日志：`%LOCALAPPDATA%\CloudIME\logs\server.<日期>.log`（设置 → 调试页的「打包日志到桌面」
会把日志与配置打成 zip）。`cloudime.log(…)` 就写在这里。几个关键行：

- `脚本已加载 dir=… scripts=N handlers=M`：加载了几个脚本、登记了几个处理函数（`scripts=0` 说明一个都没加载）。
- `脚本执行出错，判无效，跳过` / `脚本读不了，跳过`：哪一条为什么没生效，都带 `script=` 文件名。
- `脚本处理事件出错，中止这一次`：某次调用出错（同时会调它的 `on_error`）。
- `脚本的 HTTP 结果超过了它给的时限，丢掉`：结果回来太晚。

常见坑：

| 现象 | 原因 |
|---|---|
| 改了脚本没反应 | 没重启输入法服务（不做热重载） |
| 脚本完全没加载 | 清单没写在最前面 / `api` 不是 `1` / `sync` 与 `handover` 对不上 / 缺必写字段 / 文件不在 `Scripts\` **根部**（`lib\` 里的不加载）/ 在禁用名单里 |
| 运行期登记处理函数报错 | `cloudime.on` 只能在加载期调 |
| `cloudime.http_get` 报「时限」错 | 第二个参数给 `0` / 没给，或给了比清单 `timeout` 更大的值 |
| 返回 `commit = 42` 没上屏 | `commit` 要**字符串**，数字不会被悄悄转成字符串（写错会记日志） |
| 在 `candidates` 里返回 `passthrough` / `commit` | 没有按键的那两拍不认这两项，只记日志 |
| `startup` 里返回动作没生效 | `startup` 的返回值会被丢掉（只用来做准备、发首个请求） |
| 卡了一下、所有应用都打不了字 | 处理函数里做了同步等待 / 死循环（只有异步 HTTP 允许「等」） |
| `cloudime.text.all()` 返回 `nil` | 刚启动 / 换了输入框之后的第一次调用常见（Server 已请 DLL 读一份，下一次就有）；私密输入框或那个控件读不到光标外的文本也一样 |
| `cloudime.context` 一直是空串、自动序号没反应 | 这个宿主不把光标**前**的文字给输入法（Windows 11 记事本实测：TSF 上下文按选区 / 组句圈成局部，读不到前面的字）—— 与「整篇读不到」同源；换 Firefox / 多数编辑器就正常 |
| 候选窗尺寸改了没反应 | 尺寸只在**本次组句**内有效、组句结束就回配置值 |
| 在游戏里按 `Ctrl` 组合没反应 | 清单里没声明这一组（`combination_modifiers` + `keys`，要精确对上）；声明了的话**组句与否都先到脚本**，不吃就返回 `passthrough = true`。输入法自己占用的组合脚本抢不走：`Ctrl+数字` / `Ctrl+Enter` / `Ctrl+反引号` / `Shift+反引号` / `Ctrl+Alt+空格` / `Ctrl+Alt+,` `.`，以及预留给 v/u/i 模式的 `Ctrl+Alt+V` / `U` / `I`（写了这些的脚本判无效） |
| `key` 触发的脚本收不到某个键 | `keys` 里没声明它 / 修饰键对不上（声明的是**精确组合**；单键要 `combination_modifiers = "none"`）；键名拼错时整个脚本判无效，日志里有原因；或那一键被系统 / 外壳先拿走了（`Alt+Tab` 之类） |

## 12. 示例集

### 12.1 文本扩展 + 提示

```lua
cloudime.on("key", function(event)
    if event.char == ";" then
        return { commit = "云朵输入法", notice = "脚本上屏" }
    end
end)
```

### 12.2 在特定应用里放行按键（游戏 / 终端里的补全不被抢）

```lua
cloudime.on("key", function(event)
    if event.app == "notepad.exe" and event.char == "q" then
        return { passthrough = true }
    end
end)
```

### 12.3 给候选加显示、把某一个提到最前

```lua
cloudime.on("candidates", function(list)
    local display = {}
    local order = {}
    for index, item in ipairs(list) do
        if item.text == "云朵" then
            display[index] = "☁ " .. item.display     -- 只是显示，上屏仍是 text
            table.insert(order, 1, index)             -- 提到第一
            break
        end
    end
    if #order > 0 then
        return { display = display, order = order }
    end
end)
```

### 12.4 加权 / 降权

```lua
-- 每一键都返回一次（`adjust` 是整份替换，内容没变时 Core 那侧什么都不做）：
-- 写在 key 里当场生效，这一键的排序就用上；写在 candidates / 回调里则影响下一次查询
cloudime.on("key", function()
    return { adjust = { ["云朵输入法"] = 50.0, ["输入法"] = 0.2 } }
end)
```

### 12.5 看光标前文

```lua
cloudime.on("key", function(event)
    if event.char == "n" and cloudime.context ~= "" then
        return { commit = "上文是：" .. cloudime.context }
    end
end)
```

### 12.6 在线翻译（异步 + 底部那一行）

清单要配 `sync = false, handover = "callback"`，且 `timeout` ≥ `http_post` 的时限。

```lua
cloudime.on("key", function(event)
    if not (event.ctrl and event.alt and event.vk == 84) then  -- Ctrl + Alt + T
        return
    end
    local text = event.highlight
    if text == "" then
        return                                       -- 这一格没有候选
    end
    cloudime.http_post("https://example.com/translate", "text=" .. text, {
        timeout_ms = 3000,
        headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    }, function(result)
        if result.status == 200 then
            -- 只放译文本身：「在线 」前缀由输入法画，别自己再加
            return { online = { text = result.body, state = "result" } }
        end
        return {
            online = {
                text = tostring(result.error or result.status),
                state = "error",
            },
        }
    end)
    return { online = { text = "在线翻译中…", state = "waiting" } }   -- 先显示等待
end)
```

完整的厂商实现（签名、表单编码、按窗口宽度折行、最多 5 行）见脚本仓库
[「小牛候选翻译」](https://github.com/WhiteCloud-OuO/CloudIME_ScriptWarehouse/tree/main/src/小牛候选翻译)
里的 `example-niutrans.lua`。

### 12.7 按候选窗当前宽度折行（长译文多行显示）

```lua
-- 把译文折到候选窗现在的宽度（最多 5 行），再写进 online 那一行
local function show_translation(text)
    -- 内容区左右各有 8 点内边距、「在线 」前缀只有第一行有；拿不到窗口宽度就退回一个常量
    local width = cloudime.candidate.width() or 460
    local prefix = cloudime.ui.measure_tip("在线 ").width
    local box = cloudime.ui.truncate(text, math.max(width - 16 - prefix, 32), {
        mode = "wrap", max_lines = 5, font = "translate",
    })
    return { online = { text = box.text, state = "result" } }   -- text 里带换行，窗口自己变高
end

cloudime.on("key", function(event)
    if not (event.ctrl and event.alt and event.vk == 84) then                  -- Ctrl + Alt + T
        return
    end
    local text = event.highlight
    if text == "" then
        return
    end
    cloudime.http_post("https://example.com/translate", "text=" .. text, {
        timeout_ms = 3000,
    }, function(result)
        if result.status == 200 then
            return show_translation(result.body)
        end
        return {
            online = {
                text = tostring(result.error or result.status),
                state = "error",
            },
        }
    end)
    return { online = { text = "在线翻译中…", state = "waiting" } }
end)
```

### 12.8 改候选窗尺寸 / 量文字

```lua
cloudime.on("key", function(event)
    if event.ctrl and event.vk == 90 then             -- Ctrl + Z
        cloudime.candidate.set_page_size(5)           -- 一页 5 个（5–9）
        cloudime.candidate.set_min_width(320)         -- 最小宽度（点；0–2000）
        cloudime.candidate.set_scale(1.2)             -- 整体放大 20%（约 0.58–2.99）
        -- 想单独恢复某一项：不带参数就行，例如 cloudime.candidate.set_scale()
    end
end)

-- 量一段文字要多宽：结果直接喂 set_min_width 就装得下（量尺只在处理函数里能用）
cloudime.on("key", function(event)
    if event.char == "m" then
        local box = cloudime.ui.measure_tip("一段很长的在线译文")
        cloudime.candidate.set_min_width(math.ceil(box.width) + 24)
    end
end)
```

### 12.9 异步回调里改了自己的状态再重画

```lua
local cache = {}

cloudime.on("key", function(event)
    if event.char == "r" then
        cloudime.http_get("https://example.com/list", 1500, function(result)
            if result.status == 200 then
                cache[result.body] = true
                cloudime.candidate.redraw()           -- 请 Server 重算这一屏（会重新派发 candidates）
                return { notice = "缓存更新了" }
            end
        end)
    end
end)
```

### 12.10 只在某几个应用里跑 + 出错记日志

```lua
-- （清单永远要写在文件最前面；这里单独贴出来只为看写法）
cloudime.script{
    name        = "编辑器辅助",
    api         = 1,
    timeout     = 300,
    apps        = { "Code.exe", "notepad.exe" },      -- exe 文件名，大小写不敏感
    sync        = true,
    handover    = "return",
    on_error    = function(event, message)
        cloudime.log(string.format("脚本在 %s 出错：%s", event, message))
    end,
    priority    = 10,                                  -- 大的盖小的
}
```

### 12.11 共享模块（放 `Scripts\lib\`）

`lib\` 里的文件不会被加载，用 `dofile` 引：

```lua
local helpers = dofile("Scripts/lib/helpers.lua")   -- 相对路径按安装目录算
```

### 12.12 复制全文到剪贴板（`cloudime.text.all`）

组句里按 `Ctrl+A`，把输入框**整篇文本**写进剪贴板，候选窗提示复制了多少字：

```lua
cloudime.on("key", function(event)
    if not (event.ctrl and event.vk == 65) then     -- Ctrl+A
        return
    end
    local box = cloudime.text.all()
    if not box then
        return { notice = "整篇还没读到，再按一次 Ctrl+A" }
    end
    -- 剪贴板可能被别的程序占着：接口会报错，用 pcall 接住
    local ok, error = pcall(cloudime.clipboard.settext, box.text)
    if not ok then
        return { notice = "写剪贴板失败：" .. tostring(error) }
    end
    return { notice = "已复制到剪贴板" }
end)
```

两个要点：`Ctrl+A` **只在组句里**到得了脚本（不然应用自己的「全选」就被抢了）；第一次调用可能是 `nil`
（刚启动 / 换了输入框），按提示再按一次即可。

### 12.13 自动序号（`combination_key` 单键 + 光标前文 + `commit`）

在 `1.` / `一、` 后面敲回车，自动接下一条的序号（像 Word）。完整版（含中文数字 `一、`→`二、`）见
[`Scripts/lib/example-auto-number.lua`](Scripts/lib/example-auto-number.lua)。

```lua
cloudime.script{
    name              = "自动序号",
    api               = 2,
    trigger_condition = "combination_key",
    combination_modifiers = "none",   -- 不按修饰键：光按 Enter
    keys              = { "enter" },
    needs_context     = true,         -- 读光标前文（判断上一行的序号）
    timeout           = 200,
    sync              = true,
    handover          = "return",
    on_error          = false,
}

cloudime.on("key", function(event)
    local line = cloudime.context:match("[^\r\n]*$")   -- 光标前那一行
    local digits, tail = line:match("^(%d+)%.(%s*)$")
    if digits then
        -- Enter 由脚本自己吃：换行 + 新序号一起上屏
        return { commit = "\r\n" .. tostring(tonumber(digits) + 1) .. "." .. tail }
    end
end)
```

要点：`Enter` 必须**由脚本吃**（`commit`），不能「放行 `Enter` 再插文本」—— 那样会先换行、序号再插到下一行；
`\r\n` 是 TSF 文档里的换行。只对多行编辑框有意义（单行框里换行会被忽略或变空格）。

## 13. 版本与边界

- **API 版本是 `2`**（清单里的 `api`）。2026-10-17 的触发条件改版（五种触发：组合键 / 时间段 /
  候选窗拼音 / 光标所在程序 / 手动）把 `api` 升到了 `2`：写 `api = 1` 的旧脚本会被判无效
  （而不是悄悄按别的方式工作）—— 旧的 `key` / `sys_time` 请改成 `combination_key`（单键用
  `combination_modifiers = "none"`）/ `time_range`。将来不兼容的改动还会继续升版本号。
- **不做热重载**：改脚本 → 重启输入法服务。
- **还没做的**（设计文档里记着）：惰性 Lua 状态（没脚本时不建状态）、
  改组句里「已选文本」的内容、把翻译 Tip 交给脚本写（现在是内置的本地词典 Tip + 独立的 `online` 行）、
  脚本窗口里的多行编辑 / 富文本（单行 `text_edit`、IME、选区、剪贴板已支持）。
- **没有脚本时不付代价**：`Scripts\` 不存在或里面没有真脚本时，按键路径上一次查表就返回，
  行为与「没有脚本功能」时逐字节一致。
