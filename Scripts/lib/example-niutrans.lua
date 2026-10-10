-- 小牛翻译：Ctrl + Alt + T 翻**高亮候选**，译文写在候选窗底部那一行。
--
-- 这份是**活跃**的那个脚本（放在 Scripts\ 根部就会被加载）。`Scripts\lib\example-niutrans.lua`
-- 是随包发出去的同一份参考（放 lib\ 里所以不会被加载）。
--
-- 两个坑先说：
--   1. 凭据写在下面这两行。**真 key 填在安装目录那一份**（比如 C:\Program Files\CloudIME\Scripts\niutrans.lua），
--      别填进仓库里这一份 —— 这份是跟着 git 走的。仓库里放的是占位。
--   2. 改完要**重启输入法服务**（任务栏「中 / 英」图标右键 → 重启输入法服务）才生效。
--
-- 占位 key 按 Ctrl + Alt + T 会看到「在线翻译失败：…（鉴权失败 / 参数错误之类）」—— 那是服务端回的，
-- 说明请求这条路是通的，填上真凭据就能出译文。

cloudime.script{
    name        = "小牛翻译",
    description = "Ctrl + Alt + T 翻高亮候选，译文写在候选窗底部那一行（初次使用点击编辑，然后填写user_id和user_key）",
    api         = 1,
    trigger_condition = "combination_key",   -- Ctrl + Alt + T 触发（key 事件）
    combination_modifiers = "ctrl+alt",      -- 要哪一套修饰键（10 选一）
    timeout     = 8000,      -- 单次调用墙钟上限（毫秒）：要 ≥ 下面 http_post 的时限
    sync        = false,     -- 只用异步回调
    handover    = "callback",
    on_error    = function(event, message)
        cloudime.log("小牛翻译脚本出错：" .. event .. " → " .. message)
    end,
}

-- 纯 Lua 的 MD5（随包放在 Scripts\lib\ 下；Server 的工作目录就是安装目录，所以这个相对路径能找到）
local md5 = dofile("Scripts/lib/md5.lua").hex

local APP_ID = "user_id"      -- TODO: 换成自己的 appId（小牛翻译云平台 → 控制台 → API应用）
local API_KEY = "user_key"    -- TODO: 换成自己的 apikey
local ENDPOINT = "https://api.niutrans.com/v2/text/translate"
local FROM, TO = "auto", "en"  -- 源语自动识别、翻成英文（见它的「支持语言列表」）
local TIMEOUT_MS = 6000        -- 等候上限；不能超过清单里的 timeout
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
    -- 组合键自己定：清单里声明的那一套（这里是 Ctrl+Alt）命中就会先到脚本一趟，组句与否都一样；
    -- 系统 / 外壳先一步处理掉的组合（Alt+Tab、Win+字母 之类）到不了脚本 —— 那种得换一套；
    -- 例外：输入法自己的四个组合（Ctrl+数字 / Ctrl+回车 / Ctrl+反引号 / Shift+反引号）脚本抢不走，绑了也不生效。
    if not (event.ctrl and event.alt and event.vk == 84) then
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
