-- 例子：Ctrl + T 调小牛翻译的「通用文本-Flash」接口翻**高亮候选**，译文写在候选窗底部那一行。
--
-- 这份文件放在 Scripts\lib\ 里，**不会被加载**（加载器只认 Scripts\ 根部的 *.lua）。
-- 想用它：拷到 Scripts\ 下（名字随意，比如 my-translate.lua），填上自己的 appId / apikey，
-- 再重启输入法服务（任务栏「中 / 英」图标右键 → 重启输入法服务）就生效了。
--
-- 要改的东西都在这份文件里：接口地址、语向、组合键、结果怎么写。输入法不掺和这些 ——
-- 它只做两件事：把 `{ online = … }` 画在候选窗底部那一行；把脚本给的请求发出去（`http_post`）。

cloudime.script{
    name        = "小牛翻译（示例）",
    description = "Ctrl+T 翻高亮候选，译文写在候选窗底部那一行",
    api         = 1,
    timeout     = 8000,      -- 单次调用墙钟上限（毫秒）：要 ≥ 下面 http_post 的时限
    sync        = false,     -- 只用异步回调
    handover    = "callback",
    on_error    = function(event, message)
        cloudime.log("小牛翻译示例出错：" .. event .. " → " .. message)
    end,
}

-- 纯 Lua 的 MD5：跟着安装包一起装在 Scripts\lib\ 下（Server 的工作目录就是安装目录）
local md5 = dofile("Scripts/lib/md5.lua").hex

local APP_ID = "填你自己的 appId"   -- 小牛翻译云平台 → 控制台 → API应用
local API_KEY = "填你自己的 apikey"
local ENDPOINT = "https://api.niutrans.com/v2/text/translate"
local FROM, TO = "auto", "en"       -- 源语自动识别、翻成英文（见它的「支持语言列表」）
local TIMEOUT_MS = 6000             -- 等候上限；不能超过清单里的 timeout
local MAX_LINES = 5            -- 最多显示几行；再长的最后一行补「…」
local FALLBACK_WIDTH = 460     -- 拿不到候选窗宽度时（还没画过窗口）按这个行宽折
-- 签名：apikey 与所有参数按参数名 ASCII 升序拼成 k=v&k=v…（空值字段不参与），再取 MD5 的小写十六进制
local function sign(params)
    params.apikey = API_KEY
    local names = {}
    for name in pairs(params) do
        names[#names + 1] = name
    end
    table.sort(names)
    local parts = {}
    for _, name in ipairs(names) do
        local value = params[name]
        if value ~= "" then
            parts[#parts + 1] = name .. "=" .. value
        end
    end
    return md5(table.concat(parts, "&"))
end

-- 表单编码（这个接口也收 application/x-www-form-urlencoded）
local function url_encode(text)
    return (text:gsub("[^%w%-%._~]", function(c)
        return string.format("%%%02X", string.byte(c))
    end))
end

local pending = false

cloudime.on("key", function(event)
    -- 组合键自己定：组句内外，带 Ctrl（不带 Alt / Win）的组合都会先到脚本一趟（Ctrl+字母 / Ctrl+Shift+… 都行）；
-- Alt / Win 组合到不了脚本 —— 那些在系统 / 外壳那一层，输入法也拦不到。
-- 例外：输入法自己的四个组合（Ctrl+数字 / Ctrl+回车 / Ctrl+反引号 / Shift+反引号）脚本抢不走，绑了也不生效。
    if not (event.ctrl and event.vk == 84) then
        return
    end
    if pending then
        return { online = false }        -- 再按一次收起
    end
    local text = event.highlight
    if text == "" then
        return                           -- 这一格没有候选
    end

    local params = {
        ["from"] = FROM,
        to = TO,
        appId = APP_ID,
        -- 毫秒（文档的参数表这么写；2026-10-07 真机验证过是对的，别照抄它 Python 示例里的秒）
        timestamp = tostring(os.time() * 1000),
        srcText = text,
    }
    local auth = sign(params)
    local body = table.concat({
        "from=" .. url_encode(params["from"]),
        "to=" .. url_encode(params.to),
        "appId=" .. url_encode(params.appId),
        "timestamp=" .. params.timestamp,
        "srcText=" .. url_encode(params.srcText),
        "authStr=" .. auth,
    }, "&")

    cloudime.http_post(ENDPOINT, body, {
        timeout_ms = TIMEOUT_MS,
        headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    }, function(result)
        pending = false
        if result.status == 200 then
            -- 最朴素的抠字段，够用就好（要更稳就自己加个 JSON 解析）
            local translated = result.body:match('"tgtText"%s*:%s*"(.-)"')
            if translated and translated ~= "" then
                -- 按候选窗**现在的宽度**折行（拿不到窗口就退回 FALLBACK_WIDTH）：不把窗口拉成一条长条，也不写死行宽
                -- 内容区左右各有 8 点内边距、「在线 」前缀只有第一行有，都扣掉；再窄也留几个字免得算成 0
                local prefix = cloudime.ui.measure_tip("在线 ").width
                local width = cloudime.candidate.width() or FALLBACK_WIDTH
                local box = cloudime.ui.truncate(translated, math.max(width - 16 - prefix, 32), {
                    mode = "wrap", max_lines = MAX_LINES, font = "translate",
                })
                -- 只放译文本身：候选窗那一行的「在线 」/「在线翻译失败：」前缀由输入法画
                return { online = { text = box.text, state = "result" } }
            end
            local message = result.body:match('"errorMsg"%s*:%s*"(.-)"')
            if message then
                return { online = { text = message, state = "error" } }
            end
        end
        return {
            online = {
                text = tostring(result.error or result.status),
                state = "error",
            },
        }
    end)

    pending = true
    return { online = { text = "在线翻译中…", state = "waiting" } }
end)
