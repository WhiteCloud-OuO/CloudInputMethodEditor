-- 小牛英汉互译窗口：Ctrl + Alt + Y。
--
-- 流程：
--   1. 先记下剪贴板，模拟 Ctrl+C，再看剪贴板有没有变化；变了就用新复制的内容；
--   2. 在输入光标附近开一个独立窗口：两个多行编辑框（下面只读）、提示标签和关闭按钮；
--   3. 如果拿到了复制文本，自动填到上面的编辑框并开始翻译；
--   4. 没有配置小牛 API / KEY 时，窗口里直接提示先修改本脚本。
--
-- 窗口静态布局在：
--   Scripts/lib/niutrans-window.json
-- 未配置 API/KEY 时，脚本用 win:update_content("warning", …) 在布局里的空标签上显示提示。
--
-- 使用前：
--   * 把本文件放到安装目录的 Scripts\ 下（或从 Scripts\lib\ 复制过去），重启输入法服务；
--   * 把 APP_ID / API_KEY 换成小牛翻译控制台里的值。仓库里的这份保持占位符，别填真凭据。
--
-- 注意：Ctrl + Enter 是输入法 / 脚本窗口的常用组合；如果某些宿主把组句状态下的按键先吃掉，
-- 可以直接点上面的编辑框后再按，或重新打开窗口。

cloudime.script{
    name                  = "小牛英汉互译窗口",
    description           = "Ctrl+Alt+Y：复制当前选中文本，在光标旁打开小牛英汉互译窗口（初次使用点击编辑，然后填写user_id和user_key）",
    api                   = 2,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",
    keys                  = { "y" },
    timeout               = 10000,
    sync                  = false,
    handover              = "callback",
    on_error              = function(event, message)
        cloudime.log("小牛英汉互译窗口出错：" .. event .. " - " .. message)
    end,
}

-- ===== 配置 =====
local APP_ID = "user_id"       -- TODO: 小牛翻译控制台 -> API 应用里的 appId
local API_KEY = "user_key"     -- TODO: 小牛翻译控制台 -> API 应用里的 apikey
local ENDPOINT = "https://api.niutrans.com/v2/text/translate"
local TIMEOUT_MS = 6000
local MAX_SOURCE = 4000        -- 窗口里的编辑框最多放这么多字符

-- ===== 窗口尺寸 =====
local WIDTH, HEIGHT = 600, 450
local GAP = 24

local CONFIGURED = APP_ID ~= ""
    and APP_ID ~= "user_id"
    and API_KEY ~= ""
    and API_KEY ~= "user_key"

local md5 = dofile("Scripts/lib/md5.lua").hex

local WARNING = "请先在脚本文件中配置小牛翻译 API 和 KEY"
local WARNING_COLOR = "#FFCC3333"

local win = nil
local waiting = false
local pending_request = false

local function read_clipboard()
    local ok, text = pcall(cloudime.clipboard.gettext)
    if not ok then
        return nil
    end
    return text
end

--- 模拟 Ctrl+C，只有剪贴板内容确实变了才返回新文本。
local function copy_selection()
    if not cloudime.keyboard or not cloudime.keyboard.press then
        return nil
    end
    local before = read_clipboard()
    local ok = pcall(cloudime.keyboard.press, "ctrl+c")
    local after = read_clipboard()
    if not ok or after == nil or after == before then
        return nil
    end
    return after
end

local function clamp_source(text)
    text = text or ""
    if #text <= MAX_SOURCE then
        return text
    end
    return text:sub(1, MAX_SOURCE) .. "\n…（内容过长，已截断）"
end

local function layout_path()
    return cloudime.install_path("Scripts/lib/niutrans-window.json")
end

local function sign(params)
    params.apikey = API_KEY
    local names = {}
    for key in pairs(params) do
        names[#names + 1] = key
    end
    table.sort(names)
    local parts = {}
    for _, key in ipairs(names) do
        if params[key] ~= "" then
            parts[#parts + 1] = key .. "=" .. params[key]
        end
    end
    return md5(table.concat(parts, "&"))
end

local function url_encode(text)
    return (text:gsub("[^%w%-%._~]", function(character)
        return string.format("%%%02X", string.byte(character))
    end))
end

-- 英汉互译：有中文字符（UTF-8 的 0xE4–0xE9 起始字节）→ 中译英，否则英译中。
local function translate_languages(text)
    if text:find("[\228-\233]") then
        return "zh", "en"
    end
    return "en", "zh"
end

local function show_result(text)
    if win then
        win:set_text("result", text)
    end
end

local function request_translate(source)
    if pending_request then
        return
    end
    if not CONFIGURED then
        show_result(WARNING)
        return
    end
    source = clamp_source(source or (win and win:get_text("source")) or "")
    if source:match("^%s*$") then
        show_result("请输入要翻译的内容")
        return
    end

    pending_request = true
    show_result("翻译中…")

    local from, to = translate_languages(source)
    local params = {
        ["from"] = from,
        to = to,
        appId = APP_ID,
        timestamp = tostring(os.time() * 1000),
        srcText = source,
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
        pending_request = false
        if not win then
            return
        end
        if result.status == 200 then
            local translated = result.body:match('"tgtText"%s*:%s*"(.-)"')
            if translated and translated ~= "" then
                show_result(translated)
                return
            end
            local message = result.body:match('"errorMsg"%s*:%s*"(.-)"')
            show_result("翻译失败：" .. (message or "返回内容为空"))
            return
        end
        show_result("翻译失败：" .. tostring(result.error or result.status))
    end)
end

local function has_ctrl(modifier)
    modifier = modifier or 0
    return math.floor(modifier / 2) % 2 == 1
end

local function make_window(source)
    local path = layout_path()
    if not path then
        cloudime.log("小牛英汉互译：找不到窗口布局文件 Scripts/lib/niutrans-window.json")
        return
    end
    win = cloudime.window.create{
        name        = "小牛英汉互译",
        width       = WIDTH,
        height      = HEIGHT,
        layout      = path,
        interactive = true,
        background  = false,
        shadow      = true,
    }
    if source and source ~= "" then
        win:set_text("source", clamp_source(source))
    end
    win:update_content("warning", {
        { text = CONFIGURED and "" or WARNING, color = WARNING_COLOR },
    })
    win:show()
    win:focus(true)
    if CONFIGURED and source and source ~= "" then
        request_translate(source)
    end
end

local function place_at_caret(pos)
    local x = pos.left
    local y = pos.top - GAP - HEIGHT
    if y < pos.work_top then
        y = pos.bottom + GAP
    end
    x = math.max(pos.work_left, math.min(x, pos.work_right - WIDTH))
    y = math.max(pos.work_top, math.min(y, pos.work_bottom - HEIGHT))
    win:show{ x = x, y = y }
end

cloudime.on("key", function(event)
    if not (event.ctrl and event.alt and event.vk == 89) then -- Ctrl+Alt+Y
        return
    end
    if waiting then
        return {}
    end

    -- 不要在 key 处理里同步模拟 Ctrl+C：这时 DLL 还在等脚本返回，目标应用的输入
    -- 线程拿不到注入的按键。先请求 caret，等 key 处理结束、应用恢复输入后再复制。
    waiting = true
    cloudime.caret()
    return {}
end)

cloudime.on("caret", function(pos)
    if not waiting then
        return
    end
    waiting = false
    local source = copy_selection()
    if win then
        win:set_text("source", clamp_source(source))
        if source and source ~= "" and CONFIGURED then
            request_translate(source)
        else
            show_result("")
        end
        win:show()
        win:focus(true)
    else
        make_window(source)
    end
    if win then
        place_at_caret(pos)
    end
end)

cloudime.on("window", function(event)
    if not win or event.id ~= win.id then
        return
    end
    if event.handler == "close" then
        win:close()
        win = nil
        pending_request = false
        return
    end
    if event.handler == "translate_key"
        and event.action == "key_pressed"
        and event.key == 13
        and has_ctrl(event.modifier)
    then
        request_translate(win:get_text("source"))
    end
end)
