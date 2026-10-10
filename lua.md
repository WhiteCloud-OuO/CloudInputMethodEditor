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
| [`Scripts/lib/example-niutrans.lua`](Scripts/lib/example-niutrans.lua) | 完整示例：小牛翻译（签名、表单编码、折行、多行显示） |
| [`Scripts/lib/example-theme-by-app.lua`](Scripts/lib/example-theme-by-app.lua) | 示例：按前台程序换主题（组合键触发 + `cloudime.foreground_app()` / `cloudime.apply_theme()`） |
| [`Scripts/lib/example-auto-number.lua`](Scripts/lib/example-auto-number.lua) | 示例：自动序号（`key` 触发 + `cloudime.text.before()` + `commit`「换行 + 下一条序号」） |

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
   相对路径按 Server 的工作目录（就是安装目录）算，所以 `dofile("Scripts/lib/md5.lua")` 能找到东西。
2. **怎么建**：设置 → 脚本 → 「新建脚本」会用模板建一个文件并用记事本打开。
   也可以自己往 `Scripts\` 里丢一个 UTF-8 的 `.lua`。
3. **什么时候生效**：**改完要重启输入法服务** —— 任务栏「中 / 英」图标右键 → 「重启输入法服务」，
   或者「设置 → 脚本」页「新建脚本」右边那个同名按钮。没有热重载。
4. **只加载根目录的 `Scripts\*.lua`**：`Scripts\lib\` 是子目录、加载器不认，适合放共享模块
   （用 `dofile` 引）；`template.lua` 是模板，**不执行**；在 `[script] disabled` 名单里的也跳过。

最小可用脚本（内容和模板里那份一样）：

```lua
cloudime.script{
    name        = "我的脚本",
    description = "打 `;` 出「云朵输入法」",
    api         = 1,
    budget      = 200000,          -- 单次调用的指令数上限（只能收窄）
    timeout     = 200,             -- 单次调用的墙钟上限（毫秒）
    apps        = { },             -- 空 = 所有应用
    sync        = true,            -- 同步：处理函数算完就返回
    handover    = "return",        -- 必须与 sync 一致
    on_error    = function(event, message)
        cloudime.log("出错：" .. event .. " → " .. message)
    end,
    priority    = 0,
}

-- 打 `;` 出「云朵输入法」
cloudime.on("key", function(event)
    if event.char == ";" then
        return { commit = "云朵输入法", notice = "脚本上屏" }
    end
end)
```

`cloudime.log` 写进运行日志：`%LOCALAPPDATA%\CloudIME\logs\server.<日期>.log`（按天分文件、保留 7 天）。
加载失败、清单不合法、处理函数出错的原因都在里面。

## 3. 脚本清单（manifest）

每个脚本**必须在文件最前面**声明一次 `cloudime.script{…}`。缺清单、字段缺失 / 类型不对、
`api` 不认识、`sync` 与 `handover` 对不上 —— **这个脚本判无效**（跳过 + 日志写明原因，别的脚本照常）。

| 字段 | 必写 | 类型 | 语义 |
|---|---|---|---|
| `api` | **是** | 整数 | 面向的脚本 API 版本，现在只有 `1`。不认识 → 无效 |
| `trigger_condition` | **是** | 字符串 | **由什么触发**，四选一：`"combination_key"` 组合键（收 `key`）/ `"key"` 具体的键（收 `key`，见 `keys`）/ `"sys_time"` 系统时间（收 `time`）/ `"candidate_context"` 候选窗里的内容（收 `candidates`）。写别的 → 无效 |
| `trigger_time` | `trigger_condition = "sys_time"` 时**是** | 字符串 | 本地时间 `"HH:MM"`（24 小时制）：每天到那个点收一次 `time` |
| `combination_modifiers` | `trigger_condition = "combination_key"` 时**是** | 字符串 | 要哪一套**修饰键**，10 选一：`"ctrl"` / `"ctrl+alt"` / `"ctrl+shift"` / `"alt+shift"` / `"alt"` / `"win+ctrl"` / `"win+alt"` / `"win+shift"` / `"win+alt+ctrl"` / `"win+ctrl+shift"`（`+` 连接的顺序随意）。写别的 → 无效 |
| `keys` | `trigger_condition = "key"` 时**是** | 字符串数组 | 要哪几个键：`"enter"` / `"tab"` / `"space"` / `"backspace"` / `"delete"` / `"esc"` / `"left"` / `"up"` / `"right"` / `"down"` / `"home"` / `"end"` / `"pageup"` / `"pagedown"` / `"f1"`…`"f12"`。可写多个；去重、忽略大小写与首尾空白。有一个不认识 → 整个脚本无效 |
| `sync` | **是** | 布尔 | `true` = 同步（处理函数算完就返回）；`false` = 只用异步回调（配 `http_get` / `http_post`） |
| `handover` | **是** | 字符串 | `"return"` / `"callback"`，**必须与 `sync` 一致**（`true`↔`"return"`、`false`↔`"callback"`） |
| `timeout` | **是** | 整数 | 单次调用的**墙钟上限**（毫秒，`1`–`60000`）：同步处理函数与异步回调都按它算，到点中止这一次 |
| `on_error` | **是** | 函数 / `false` | 形式 `function(event, message)`：这个脚本出错时调它。写 `false` = 不处理，直接中止这一次 |
| `budget` | 否 | 整数 | 单次调用的**指令数**上限；缺省用全局 `100000000`（一亿）。**只能收窄不能放宽**（写大了按全局走并记一条日志） |
| `apps` | 否 | 字符串数组 | 只在哪些应用里跑（**exe 文件名**，大小写不敏感，如 `"notepad.exe"`）。空 / 不写 = 所有应用 |
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
- `apps` 只过滤 `key` / `candidates` / `time`（都按当时聚焦的程序）；`startup` 早于任何应用、异步回调在请求发出时就已经判过。
- **一个脚本只有一个触发条件**（清单的 `trigger_condition`）：Server 只把对应的事件派给它 ——
  `combination_key` / `key` 收 `key`、`sys_time` 收 `time`、`candidate_context` 收 `candidates`；
  `startup` 与 HTTP 回调不受它限制。
- **组合键脚本要声明修饰键**（`combination_modifiers`）：只把你声明的那一套送上来 ——
  组句与否都一样，`ctrl` 与 `ctrl+shift` 是两套。输入法为此让 DLL 在没组句时也把命中的组合键送来问一趟
  （脚本不吃就**重放回应用**：DLL 用 `SendInput` 注入同一个键，见 `replay_to_app`）；修饰键**本身**
  （`Ctrl` / `Alt` / `Shift` / `Win`）不会被动，照常归应用；没有任何组合键脚本时这条不生效。`key` 载荷里的 `ctrl` / `alt` / `shift` / `win`
  就是那一刻按着的修饰键（`win` 是 Win 键）。
  ⚠️ `Alt+Tab` / `Alt+F4` / `Win+L` / `Win+D` 这类**系统 / 外壳自己会处理的**组合，输入法根本收不到，
  声明了也没用；`AltGr` 就是 `Ctrl+Alt`（打字用），声明 `ctrl+alt` 要当心吃掉它。
- **`key` 触发脚本要声明具体的键**（`keys`）：只把你声明的那几个键送上来 —— **没组句也一样**（输入法为此让
  DLL 把命中的键先问一趟 Server，脚本不吃就**重放回应用**）。**声明的是键本身、不带修饰键**：`keys = { "enter" }`
  时 `Shift+Enter` / `Ctrl+Enter` 也会送到脚本（自己按 `event.ctrl` / `event.shift` 分支）。不声明就一个键都收不到；
  没有任何 `key` 脚本时这条不生效。**声明之外的键不会到脚本手里**（组句里 Server 虽然会把所有按键派出去，但只有
  你声明的那些会转给 `key` 脚本）。
  ⚠️ 在 `key` 里返回 `passthrough` 等于本来就会发生的事；真正有意义的是 `commit`（吃掉这一键、自己上屏）——
  所以只对确实需要的输入框（多行编辑的自动序号之类）生效才好，`Enter` / `Tab` 被吃掉会改变输入框行为。

## 4. 事件与载荷

用 `cloudime.on(事件名, 处理函数)` 登记。现在会派发的就下面几种（别的名字登记了也不会被调）；
**只有触发条件对上的那一种会派给你**（见清单的 `trigger_condition`）：

| 事件 | 什么时候调 | 能改什么 |
|---|---|---|
| `startup` | 全部脚本加载完（**所有脚本**都收） | 只做自己的准备（开文件、发首个请求）。**返回值会被丢掉** |
| `key` | 一次按键、**引擎处理之前**（只有 `combination_key` / `key` 脚本收） | 见[动作表](#5-返回值动作表)全部项；`passthrough` / `commit` 只有这一拍认 |
| `candidates` | **引擎排完之后**（每次重排；只有 `candidate_context` 脚本收） | `order` / `display` / `notice` / `adjust` / `online` / `theme`；`passthrough` / `commit` 给了只记日志 |
| `time` | 到清单里 `trigger_time` 那个点（本地时间 `HH:MM`；只有 `sys_time` 脚本收） | `notice` / `adjust` / `online` / `theme`；`passthrough` / `commit` 给了只记日志。载荷同 `cloudime.get_time()`，另加 `app`（那一刻前台程序） |
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

`key` 触发脚本大多不在组句里：这类脚本要读光标前文就用 **`cloudime.context`**（不是 `cloudime.text.*`）——
有 `key` 脚本时输入法在**没组句的每个按键之前**现刷一份，所以你按 `Enter` 时它已经包含**上一次按键**的结果
（刚敲的 `1.` 就在里面）—— 自动序号那类脚本靠的就是这个。**能不能读到取决于宿主**：Windows 11 记事本不给
光标前的文字，那里 `cloudime.context` 一直是空串（见[第 11 节](#11-调试与排错)）。`cloudime.text.*` 那份整篇快照
只随**组句**更新，`key` 脚本一般用不上（可能 `nil` 或偏旧）。
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
| `cloudime.run(命令)` | 起外部程序，**启动就返回、不等子进程**（等同 `os.execute`） |
| `cloudime.get_time()` | 当前**本地时间** → `{ year, month, day, hour, minute, second, weekday, unix }`；`weekday` 1 = 周一 … 7 = 周日，`unix` 是 Unix 秒（标准库的 `os.date` / `os.time` 也能用，这一份是不依赖标准库的稳定形状） |
| `cloudime.get_curr_config()` | 当前**配置**的一张表：**键名与 `config.toml` 一致**、分节同名（`candidate` / `input` / `general` / `theme` / `translate` / `status_bar` / `debugging` / `script` / `word_bank` / `phrase` / `update`）；`theme` 那一节多一个 `available`（可选主题名列表）。每次调用读的都是**当前**那份（配置热加载后跟着变） |
| `cloudime.foreground_app()` | 当前**前台程序**的 exe 名（如 `"notepad.exe"`）；没聚焦任何程序给 `nil`。**随时可问**，不必等事件 —— 事件载荷里的 `app` 也是同一个值（`key` / `candidates` / `time` / 回调都带） |
| `cloudime.apply_theme(名字)` | 换上这份主题：**写进配置 `[theme] curr_theme` 并重启输入法服务**（与设置页「应用主题」同一套；重启才一定换上、选择也留下）。名字可带 / 不带 `.json`；`false` 回本进程启动时配置里那份；名字写错按缺省主题画并记一条日志（**不写配置、不重启**）。**候选窗开着时等它关掉再换/重启** |

`cloudime.clipboard.*` 两条要知道的：

- 剪贴板是**全局资源**：别的程序正占着它时开头几次会失败（Server 短等重试几次，仍不行就报错）→ 建议 `pcall` 接住。
- `gettext()` 读得到**任何程序**刚放进剪贴板的东西（密码管理器复制的密码也算）—— 这是你自己写的脚本、风险自担；
  输入法不拦也不记。空的剪贴板给 `nil`（与「打不开」区分：后者报错）。

`cloudime.text.*` 的几件事：

- **上限按显示宽度算**：西文 / 半角 **1**、中文 / 全角 **2**；三个入口的缺省都是 **2^64-1**（等于不限）。
  超上限时**留光标附近**：`before` 从头部丢、`after` 从尾部丢、`all` 是「前一半 + 后一半」（哪边不够就多留
  另一边），返回值里 `truncated = true`。
- **可能给 `nil`**：整篇只有输入法 DLL 读得到（它在应用进程里），Server 只能等它送 ——
  **刚启动、或换了输入框 / 会话（重连）**之后第一次调用是 `nil`，**换一段组句再试**就有。DLL 的读发生在
  起组句那一键的编辑会话里、晚于按键本身，所以**起组句那一键**的脚本拿到的还是上一份。**组句结束不清快照**：
  同一段组句从第二键起、以及**下一段组句的第一键**都能拿到，而**下一段组句起始又会把新的一份送上来** ——
  快照跟着文档走（刚上屏的字也在里面；当前还没上屏的拼音不在）。
  **`key` 触发脚本**（一般不在组句里）读前文请用 **`cloudime.context`**：没组句时每个按键之前现刷一份，
  读到的是**上一次按键之后**的光标前文；`cloudime.text.*` 这份整篇快照仍只随**组句起始**更新（`key` 脚本
  一般拿不到）。
- **读不到的地方永远 `nil`**：密码框与声明了私密的输入框（浏览器无痕窗口就是）、不支持读光标外范围的控件，
  以及**只把输入框暴露成「局部上下文」的宿主**（Windows 11 记事本实测：它的 TSF 上下文按选区 / 组句
  圈成局部，整篇读不到，这时一律 `nil`）。整篇最多 20 万个 UTF-16 单元（约 10 万汉字，写死不可配）；
  **没有任何脚本时完全不读**（零开销），有需要整篇的脚本时每段组句起始读一次（成本与文档大小成正比），
  有 `key` 脚本时每个按键前读一次光标前文（很短，几十个字）。

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
  （`Scripts\lib\example-niutrans.lua` 用最朴素的 `:match('"tgtText"%s*:%s*"(.-)"')`）。
- 想让回调生效，清单按 `sync = false, handover = "callback"` 写（两者必须自洽：`true` 配 `"return"`）。

### 候选窗

| 方法 | 说明 |
|---|---|
| `cloudime.candidate.width()` | 候选窗**内容区宽度**（单位点）。不在组句 / 还没画过时是 `nil`，脚本自己回退。折行就按它来 |
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

## 7. 标准库与运行环境

- 语言是 **Lua 5.1**（LuaJIT）。`string` / `table` / `math` / `os` / `io` / `package` 都在；
  **没有 `debug` 库**；LuaJIT 的 `bit` 库可用（随包的 `Scripts\lib\md5.lua` 就靠它）。
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
| 在游戏里按 `Ctrl` 组合没反应 | **组句里**那些键先到脚本：要放行就返回 `passthrough = true`；没在组句时带 `Ctrl` 的键不问脚本、原样归应用。输入法自己占用的四个组合（`Ctrl+数字` / `Ctrl+Enter` / `Ctrl+反引号` / `Shift+反引号`）脚本抢不走 |
| `key` 触发的脚本收不到某个键 | `keys` 里没声明它（键名拼错时整个脚本判无效，日志里有原因）；或那一键被系统 / 外壳先拿走了（`Alt+Tab` 之类） |

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

完整的厂商实现（签名、表单编码、按窗口宽度折行、最多 5 行）见
[`Scripts/lib/example-niutrans.lua`](Scripts/lib/example-niutrans.lua)。

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

### 12.13 自动序号（`key` 触发 + 光标前文 + `commit`）

在 `1.` / `一、` 后面敲回车，自动接下一条的序号（像 Word）。完整版（含中文数字 `一、`→`二、`）见
[`Scripts/lib/example-auto-number.lua`](Scripts/lib/example-auto-number.lua)。

```lua
cloudime.script{
    name              = "自动序号",
    api               = 1,
    trigger_condition = "key",
    keys              = { "enter" },
    timeout           = 200,
    sync              = true,
    handover          = "return",
    on_error          = false,
}

cloudime.on("key", function(event)
    if event.vk ~= 0x0D then                 -- 只认 Enter
        return
    end
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

- **API 版本是 `1`**（清单里的 `api`）。将来改了不兼容的地方会加新版本号，
  老脚本会因为 `api` 不认识被判无效（而不是悄悄行为不对）。
- **不做热重载**：改脚本 → 重启输入法服务。
- **还没做的**（设计文档里记着）：惰性 Lua 状态（没脚本时不建状态）、
  改组句里「已选文本」的内容、把翻译 Tip 交给脚本写（现在是内置的本地词典 Tip + 独立的 `online` 行）。
- **没有脚本时不付代价**：`Scripts\` 不存在或里面没有真脚本时，按键路径上一次查表就返回，
  行为与「没有脚本功能」时逐字节一致。
