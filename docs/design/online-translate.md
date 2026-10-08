# 在线翻译：脚本自己实现（2026-10-07）

## 结论：输入法里没有「在线翻译」这个功能

输入法**不认识任何翻译厂商**：没有 `translate_online` 这种接口、没有 `[translate.online]` 配置项、
设置页里也没有那一节。它只提供三样**通用**的东西，剩下的全在用户的 Lua 脚本里：

| 输入法给的 | 说明 |
|---|---|
| `cloudime.http_get(url, timeout_ms, callback)` | 异步 GET（早就有） |
| `cloudime.http_post(url, body, options, callback)` | 异步 POST；`options = { timeout_ms = …, headers = { … } }` |
| 动作 `online = "文本"` / `{ text = …, state = "waiting" \| "result" \| "error" }` / `false` | 候选窗**底部单独那一行**：脚本把结果给 Server，Server 只负责画（这一行还参与 `Ctrl + 反引号` 上屏，见下） |
| `key` 载荷里的 `highlight` | 这一刻**高亮候选的文本**（脚本要翻谁得知道翻谁） |

`online` 那一行归脚本：换候选、改拼音都不动它，只有组句结束才收 —— 想「换候选就刷新」就在 `candidates`
事件里重写。顶部的 `notice` 是另一处，两条互不影响。**它支持多行**：脚本体里写了换行（`\n`）就按几行画，
窗口高度跟着长（渲染器按 `\n` 分行；最高几行由脚本自己限）。

**随包的示例**：`Scripts\lib\example-niutrans.lua`（小牛翻译「通用文本-Flash」的完整实现：签名、表单编码、
抠 `tgtText`、写 `online` 都写好了）。它在 `lib\` 子目录里，**不会被加载** —— 拷到 `Scripts\` 根部、
填上自己的 appId / apikey、重启输入法服务就生效。另有 `Scripts\lib\md5.lua`（纯 Lua MD5）供签名用。

示例拿到译文后还会**按候选窗现在的宽度折行**：`cloudime.candidate.width()` 拿到窗口内容区宽度（点），
扣掉左右内边距（各 8 点）与「在线 」前缀，用 `cloudime.ui.truncate(译文, 行宽, { mode = "wrap",
max_lines = 5, font = "translate" })` 折成最多 5 行（超出的在末行补「…」）。行宽跟着窗口走，
不用在脚本里写死；拿不到窗口宽度（还没画过）时才退回一个兜底常量。长译文于是不再把窗口拉成一条长条，
也不会被截掉 —— 最多 5 行，行数由脚本里的 `MAX_LINES` 定；窗口宽度本身不再由脚本改。

## 上屏：`Ctrl + 反引号` 会带上那一行

写进 `online` 那一行的译文也当「一条译文」参与 `Ctrl + 反引号`（`Router::translate_action`）：

- 列表 = **在线那条排最前面** + 本地词典的释义；在线那条带 `pos = "在线"`，列表里渲染成 `(在线)`。
- 只有它一条 → 直接上屏；和本地释义一起 → 进释义选择（数字键 / 空格 / 鼠标选，与原来一样）。
- **上屏的是那一行的原文**（输入法画的「在线 」前缀不算），所以脚本别自己再加前缀。
- 上屏在线那条**不**记本地词典的「学会」次数（那个记的是本地释义的上屏次数）—— 靠 `ONLINE_POS` 标记区分。
- **陈旧的不认**：写这一行时输入法记下了「当时高亮候选是谁」（`Online::word`），
  `Ctrl + 反引号` 只在这一行还对应**当前**高亮候选时才用它；挪过候选就退回本地词典那套
  （避免把上一个候选的译文上屏）。换回原来那个候选，它又算数。

## 为什么这么放

- **厂商 / 端点 / 参数 / 语向 / 组合键 / 结果怎么用**，全是脚本里几行；换服务商只改脚本，输入法不动。
- 输入法侧不留厂商相关代码：没有 HTTP 之外的接口面，也没有随之而来的配置与设置页。
- 要签名的接口（比如小牛）需要 MD5：Lua 标准库没有，所以随包给一份**纯 Lua MD5**
  （`Scripts\lib\md5.lua`，只依赖 LuaJIT 的 `bit` 库）。它拿 RFC 1321 的标准向量与小牛文档里的例子
  当回归用例（`crates/cloudime-script` 的 `the_shipped_lua_md5_matches_the_known_vectors`）——
  这份文件写错，所有签名就都错了。

## `http_post` 的形状

```lua
cloudime.http_post(url, body, { timeout_ms = 6000, headers = { ["Content-Type"] = "application/x-www-form-urlencoded" } },
    function(result, list)
        -- result = { url, status, body } 成功；{ url, error } 连不上 / 超时
    end)
```

- `timeout_ms` **必给**（给 0 或不给直接报 Lua 错、请求不发），且不能超过清单里的 `timeout`；
  超过时限才回来的结果一律丢掉（与 `http_get` 同一条规矩）。
- `headers` 可选；名字 / 值不合法会**当场**报错（不给后台线程 panic 的机会）。
- 请求在自己的线程上跑，工人线程只收结果 —— 与 `http_get` 共用同一条管线
  （`Runtime::poll_requests`，`tick` 那一拍交付给回调）。

## 小牛翻译那个接口本身（示例里用的）

`POST https://api.niutrans.com/v2/text/translate`，`from` / `to` / `srcText` / `appId` / `timestamp` / `authStr`：

- 返回成功 `{"from":"zh","to":"en","tgtText":"Hello"}`；失败 `{"errorCode":"…","errorMsg":"…"}`
  （`13001` 没流量、`20001` 鉴权失败、`10001` 超 QPS…见它文档的错误码表）。
- **签名**：把 `apikey` 与其余参数一起按参数名 **ASCII 升序**拼成 `k=v&k=v…`（空值字段不参与），
  取 **MD5** 的小写十六进制当 `authStr` 随请求发出；`apikey` 本身不上网。
- **时间戳的坑**：它文档自相矛盾（参数表写「毫秒」、Java / C# 示例用毫秒、Python 示例用秒）。
  **实测毫秒是对的**（2026-10-07 真机联调：「翻译成功了」），示例就按毫秒发（`os.time() * 1000`）
  —— 别照抄它 Python 示例里的秒。

## 组合键：脚本能绑哪些

**带 `Ctrl`（不带 `Alt` / `Win`）的组合都能到脚本**（`com::service::key_sink::eats_key` 里那条规则）：
`Ctrl+字母` / `Ctrl+标点` / `Ctrl+Shift+…` / `Ctrl+功能键` —— 但**只在组句里**（候选窗显示着）：没在组句时这些键一律归应用，`Ctrl+A` / `Ctrl+C` 这类快捷键不能被输入法吃掉。
DLL 先把它们送进 Server 问一趟，脚本想接管就接管，没人接管就回 `Passthrough`、按键照旧交给应用。
`Alt` / `Win` 组合不碰：AltGr 就是 `Ctrl+Alt`（打字用）、`Win` 是系统键，而且 `Alt+Tab` / `Alt+F4` /
`Win+L` / `Ctrl+Alt+Del` 这类本来就在系统 / 外壳那一层被处理掉了，输入法拦不到。

代价（用户接受这份责任）：每个 `Ctrl` 组合都多一趟 IPC —— 没有脚本时那趟几乎为零（Server 见到没有
`key` 处理函数就直接放行），有脚本时按脚本的活算；脚本卡住时这些键会跟着等（与拼音字母同一条已知风险）。
逃生门留着：`Win+Space` / 任务栏右键这些不走这条路，输入法坏不了也锁不住人。

**输入法自己占用的组合键脚本抢不走**（谁先定义谁优先，`Router::handle_key` 里 `reserved_combo` 直接跳过
派发）：`Ctrl+数字`、`Ctrl+Enter`、`Ctrl+反引号`、`Shift+反引号`。别的 `Ctrl` 组合（含 `Ctrl+Shift+…`）
都能到脚本。

**不用 `Ctrl` 的键**也都能到脚本：组句里字母、数字、标点、功能键、方向键全送；不在组句时字母（中文模式下）、
数字、标点也送（那是为了标点转全角）。想用 `;` 之类触发也行 —— 只是组句里按字母会同时打进拼音，
才常用 `Ctrl+字母`。

## 还没做

- **随包就生效的翻译脚本**：现在只给 `lib\` 下的示例（要用户自己拷 + 填凭据）。
- **JSON 解析**：示例用最朴素的字符串匹配抠 `tgtText`（要更稳就自己加个解析）。
- **缓存 / 批量**：每次按都是一次新请求（脚本想缓存就自己在 Lua 里存表）。
