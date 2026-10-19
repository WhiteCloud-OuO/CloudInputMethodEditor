-- 在线查询面板：Ctrl + Alt + Q。
--
-- 有候选窗口时：
--   * 查询词 = 当前高亮候选；
--   * 面板摆在候选窗口旁边（右边放不下就放左边，再不行放候选窗下方 / 上方）；
--   * 候选是中文词语 / 汉字或英文单词时自动查询。
-- 没有候选窗口时：直接在光标附近打开空面板，输入中文或英文后按回车或点搜索。
-- 窗口显示后焦点会自动落到单行输入框。
--
-- 布局在 Scripts/lib/fast-searching-online.json；本文件放到 Scripts\ 下后重启输入法服务即可。
-- 查询中文词语 / 汉字走汉典，英文单词走有道。

cloudime.script{
    name                  = "在线查询",
    description           = "Ctrl+Alt+Q：查询高亮候选词或输入词语，中文走汉典、英文走有道，在独立面板里显示释义",
    api                   = 2,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",
    keys                  = { "q" },
    timeout               = 12000,
    sync                  = false,
    handover              = "callback",
    on_error              = function(event, message)
        cloudime.log("在线查询出错：" .. event .. " - " .. message)
    end,
}

local WIDTH, HEIGHT = 440, 360
local EDGE_GAP = 8
local HTTP_TIMEOUT_MS = 6000

local win = nil
local waiting_caret = false
local pending_word = ""
local pending_has_candidate = false
local request_seq = 0
local pending_query_text = nil
local last_query_text = nil
local last_query_at = 0
-- 同词查询结果缓存：热键 / Enter 自动重复时不再把已经显示的结果打回“正在查询”。
local last_result_word = nil
local last_result_source = nil
local last_result_text = nil

local function trim(text)
    return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function utf8_char(cp)
    if cp < 0x80 then
        return string.char(cp)
    elseif cp < 0x800 then
        return string.char(
            0xC0 + math.floor(cp / 0x40),
            0x80 + (cp % 0x40)
        )
    elseif cp < 0x10000 then
        return string.char(
            0xE0 + math.floor(cp / 0x1000),
            0x80 + (math.floor(cp / 0x40) % 0x40),
            0x80 + (cp % 0x40)
        )
    else
        return string.char(
            0xF0 + math.floor(cp / 0x40000),
            0x80 + (math.floor(cp / 0x1000) % 0x40),
            0x80 + (math.floor(cp / 0x40) % 0x40),
            0x80 + (cp % 0x40)
        )
    end
end

local function decode_utf8(text, index)
    local b1 = text:byte(index)
    if not b1 then
        return nil, index
    end
    if b1 < 0x80 then
        return b1, index + 1
    end
    if b1 < 0xE0 then
        local b2 = text:byte(index + 1)
        if not b2 or b2 < 0x80 or b2 > 0xBF then
            return nil, index + 1
        end
        return (b1 - 0xC0) * 0x40 + (b2 - 0x80), index + 2
    end
    if b1 < 0xF0 then
        local b2 = text:byte(index + 1)
        local b3 = text:byte(index + 2)
        if not b2 or not b3 or b2 < 0x80 or b2 > 0xBF or b3 < 0x80 or b3 > 0xBF then
            return nil, index + 1
        end
        return (b1 - 0xE0) * 0x1000
            + (b2 - 0x80) * 0x40
            + (b3 - 0x80), index + 3
    end
    if b1 < 0xF8 then
        local b2 = text:byte(index + 1)
        local b3 = text:byte(index + 2)
        local b4 = text:byte(index + 3)
        if not b2 or not b3 or not b4
            or b2 < 0x80 or b2 > 0xBF
            or b3 < 0x80 or b3 > 0xBF
            or b4 < 0x80 or b4 > 0xBF then
            return nil, index + 1
        end
        return (b1 - 0xF0) * 0x40000
            + (b2 - 0x80) * 0x1000
            + (b3 - 0x80) * 0x40
            + (b4 - 0x80), index + 4
    end
    return nil, index + 1
end

local function is_chinese_word(text)
    if text == "" then
        return false
    end
    local index = 1
    while index <= #text do
        local cp, next_index = decode_utf8(text, index)
        if not cp or next_index <= index then
            return false
        end
        local is_cjk = (cp >= 0x3400 and cp <= 0x4DBF)   -- 扩展 A
            or (cp >= 0x4E00 and cp <= 0x9FFF)           -- 常用汉字
            or (cp >= 0xF900 and cp <= 0xFAFF)            -- 兼容汉字
            or cp == 0x3007                               -- 〇
        if not is_cjk then
            return false
        end
        index = next_index
    end
    return true
end

local function is_english_word(text)
    if text == "" then
        return false
    end
    for index = 1, #text do
        local byte = text:byte(index)
        if byte >= 0x80 then
            return false
        end
        if not string.char(byte):match("[%w%-' ]") then
            return false
        end
    end
    return true
end

local function url_encode(text)
    return (text:gsub(".", function(char)
        local byte = string.byte(char)
        if (byte >= 0x30 and byte <= 0x39)
            or (byte >= 0x41 and byte <= 0x5A)
            or (byte >= 0x61 and byte <= 0x7A)
            or byte == 0x2D or byte == 0x2E or byte == 0x5F or byte == 0x7E then
            return char
        end
        return string.format("%%%02X", byte)
    end))
end

local function html_unescape(text)
    text = text:gsub("&#x(%x+);", function(hex)
        local cp = tonumber(hex, 16)
        return cp and utf8_char(cp) or ""
    end)
    text = text:gsub("&#(%d+);", function(decimal)
        local cp = tonumber(decimal, 10)
        return cp and utf8_char(cp) or ""
    end)
    local entities = {
        amp = "&",
        lt = "<",
        gt = ">",
        quot = "\"",
        apos = "'",
        nbsp = " ",
    }
    return (text:gsub("&(%a+);", function(name)
        return entities[name] or ""
    end))
end

local function clean_html(fragment)
    fragment = fragment:gsub("<br%s*/?>", "\n")
    fragment = fragment:gsub("<[^>]->", " ")
    fragment = html_unescape(fragment)
    fragment = fragment:gsub("%s+", " ")
    return trim(fragment)
end

local function collect_html(html, pattern)
    local values = {}
    for fragment in html:gmatch(pattern) do
        local value = clean_html(fragment)
        if value ~= "" then
            values[#values + 1] = value
        end
    end
    return values
end

local function first_html(html, pattern)
    local values = collect_html(html, pattern)
    return values[1]
end

local function append_items(lines, title, values)
    if #values == 0 then
        return
    end
    lines[#lines + 1] = title
    for index, value in ipairs(values) do
        lines[#lines + 1] = tostring(index) .. ". " .. value
    end
end

local function parse_zdic(html, word)
    local lines = { "词语：" .. word }

    local pinyin = first_html(html, '<span class="meta%-pinyin">(.-)</span>')
    if pinyin then
        lines[#lines + 1] = "拼音：" .. pinyin
    end

    local definitions = collect_html(html, '<div class="xxjs%-item__def">(.-)</div>')
    append_items(lines, "【词语解释】", definitions)

    local guoyu = collect_html(html, '<p class="gy%-sense__def">(.-)</p>')
    append_items(lines, "【国语辞典】", guoyu)

    if #definitions == 0 and #guoyu == 0 then
        return nil
    end
    return table.concat(lines, "\n")
end

-- 最小的 JSON 解码器：有道返回的是 JSON，但脚本运行时不带 JSON 库。
-- 支持对象 / 数组 / 字符串 / 数字 / true / false / null，够解析 jsonapi 的响应。
local function json_decode(text)
    local pos, length = 1, #text

    local function fail(message)
        error(message, 0)
    end

    local function skip()
        while pos <= length do
            local byte = text:byte(pos)
            if byte == 0x20 or byte == 0x09 or byte == 0x0A or byte == 0x0D then
                pos = pos + 1
            else
                break
            end
        end
    end

    local parse_value

    local function parse_string()
        if text:sub(pos, pos) ~= '"' then
            fail("JSON 字符串缺少引号")
        end
        pos = pos + 1
        local out = {}
        local escapes = {
            ['"'] = '"',
            ['\\'] = '\\',
            ['/'] = '/',
            b = '\b',
            f = '\f',
            n = '\n',
            r = '\r',
            t = '\t',
        }
        while pos <= length do
            local byte = text:byte(pos)
            if byte == 0x22 then
                pos = pos + 1
                return table.concat(out)
            elseif byte == 0x5C then
                pos = pos + 1
                local escape = text:sub(pos, pos)
                pos = pos + 1
                if escapes[escape] then
                    out[#out + 1] = escapes[escape]
                elseif escape == "u" then
                    local code = tonumber(text:sub(pos, pos + 3), 16)
                    pos = pos + 4
                    if not code then
                        fail("JSON Unicode 转义不完整")
                    end
                    if code >= 0xD800 and code <= 0xDBFF
                        and text:sub(pos, pos + 1) == "\\u" then
                        local low = tonumber(text:sub(pos + 2, pos + 5), 16)
                        if low and low >= 0xDC00 and low <= 0xDFFF then
                            pos = pos + 6
                            code = 0x10000
                                + (code - 0xD800) * 0x400
                                + (low - 0xDC00)
                        end
                    end
                    out[#out + 1] = utf8_char(code)
                else
                    fail("JSON 转义不认识：" .. escape)
                end
            else
                out[#out + 1] = string.char(byte)
                pos = pos + 1
            end
        end
        fail("JSON 字符串没有结束")
    end

    parse_value = function()
        skip()
        local first = text:sub(pos, pos)
        if first == "{" then
            pos = pos + 1
            local value = {}
            skip()
            if text:sub(pos, pos) == "}" then
                pos = pos + 1
                return value
            end
            while true do
                skip()
                local key = parse_string()
                skip()
                if text:sub(pos, pos) ~= ":" then
                    fail("JSON 对象缺冒号")
                end
                pos = pos + 1
                value[key] = parse_value()
                skip()
                local separator = text:sub(pos, pos)
                if separator == "}" then
                    pos = pos + 1
                    return value
                elseif separator ~= "," then
                    fail("JSON 对象缺逗号")
                end
                pos = pos + 1
            end
        elseif first == "[" then
            pos = pos + 1
            local value = {}
            skip()
            if text:sub(pos, pos) == "]" then
                pos = pos + 1
                return value
            end
            while true do
                value[#value + 1] = parse_value()
                skip()
                local separator = text:sub(pos, pos)
                if separator == "]" then
                    pos = pos + 1
                    return value
                elseif separator ~= "," then
                    fail("JSON 数组缺逗号")
                end
                pos = pos + 1
            end
        elseif first == '"' then
            return parse_string()
        elseif text:sub(pos, pos + 3) == "true" then
            pos = pos + 4
            return true
        elseif text:sub(pos, pos + 4) == "false" then
            pos = pos + 5
            return false
        elseif text:sub(pos, pos + 3) == "null" then
            pos = pos + 4
            return nil
        elseif first == "-" or first:match("%d") then
            local start = pos
            while pos <= length and text:sub(pos, pos):match("[-+0-9%.eE]") do
                pos = pos + 1
            end
            local number = tonumber(text:sub(start, pos - 1))
            if not number then
                fail("JSON 数字不合法")
            end
            return number
        end
        fail("JSON 值不认识")
    end

    local value = parse_value()
    skip()
    if pos <= length then
        fail("JSON 末尾还有内容")
    end
    return value
end

local function text_value(value)
    if type(value) == "string" then
        return value
    end
    if type(value) == "table" then
        local parts = {}
        for _, item in ipairs(value) do
            local text = text_value(item)
            if text and text ~= "" then
                parts[#parts + 1] = text
            end
        end
        return table.concat(parts, " ")
    end
    return nil
end

local function parse_youdao(body, word)
    local ok, data = pcall(json_decode, body)
    if not ok or type(data) ~= "table" then
        return nil
    end

    local lines = { "单词：" .. word }
    local simple = data.simple
    local simple_word = simple and simple.word and simple.word[1]
    if type(simple_word) == "table" then
        if simple_word.usphone and simple_word.usphone ~= "" then
            lines[#lines + 1] = "美音：" .. simple_word.usphone
        end
        if simple_word.ukphone and simple_word.ukphone ~= "" then
            lines[#lines + 1] = "英音：" .. simple_word.ukphone
        end
    end

    local definitions, seen = {}, {}
    local function add(value)
        value = trim(text_value(value) or "")
        if value ~= "" and not seen[value] then
            seen[value] = true
            definitions[#definitions + 1] = value
        end
    end
    local function add_with_pos(pos, value)
        value = trim(text_value(value) or "")
        if value == "" then
            return
        end
        pos = trim(pos or "")
        if pos ~= "" and not value:match("^%a+%.") then
            value = pos .. " " .. value
        end
        add(value)
    end

    local ec_words = data.ec and data.ec.word
    if type(ec_words) == "table" then
        for _, entry in ipairs(ec_words) do
            for _, group in ipairs(entry.trs or {}) do
                for _, translation in ipairs(group.tr or {}) do
                    local line = translation.l
                    add_with_pos(translation.pos, line and line.i)
                end
            end
        end
    end

    if data.individual and type(data.individual.trs) == "table" then
        for _, item in ipairs(data.individual.trs) do
            add_with_pos(item.pos, item.tran)
        end
    end

    if #definitions == 0 then
        local english = data.ee and data.ee.word
        if type(english) == "table" then
            for _, group in ipairs(english.trs or {}) do
                add_with_pos(group.pos, group.tr and group.tr[1] and group.tr[1].l and group.tr[1].l.i)
            end
        end
    end

    local web_definitions = {}
    local web = data.web_trans and data.web_trans["web-translation"]
    if type(web) == "table" then
        for _, entry in ipairs(web) do
            for _, translation in ipairs(entry.trans or {}) do
                local value = trim(text_value(translation.value) or "")
                if value ~= "" then
                    web_definitions[#web_definitions + 1] = value
                end
            end
        end
    end

    if #definitions > 0 then
        lines[#lines + 1] = "【释义】"
        for index, value in ipairs(definitions) do
            lines[#lines + 1] = tostring(index) .. ". " .. value
        end
    elseif #web_definitions > 0 then
        lines[#lines + 1] = "【网络释义】"
        for index, value in ipairs(web_definitions) do
            lines[#lines + 1] = tostring(index) .. ". " .. value
        end
    else
        return nil
    end
    return table.concat(lines, "\n")
end

local function collect_texts(value, output)
    if type(value) == "string" then
        local text = trim(value)
        if text ~= "" then
            output[#output + 1] = text
        end
    elseif type(value) == "table" then
        if type(value["#text"]) == "string" then
            local text = trim(value["#text"])
            if text ~= "" then
                output[#output + 1] = text
            end
        end
        for _, item in ipairs(value) do
            collect_texts(item, output)
        end
    end
end

-- 中文词条额外查一次有道，补英文 / 翻译；返回给汉典结果拼在末尾。
local function parse_youdao_cn(body, word)
    local ok, data = pcall(json_decode, body)
    if not ok or type(data) ~= "table" then
        return nil
    end

    local english, translations = {}, {}
    local seen = {}
    local function add(output, value)
        value = trim(value or "")
        if value ~= "" and not seen[value] then
            seen[value] = true
            table.insert(output, value)
        end
    end

    local ce_words = data.ce and data.ce.word
    if type(ce_words) == "table" then
        for _, entry in ipairs(ce_words) do
            for _, group in ipairs(entry.trs or {}) do
                for _, translation in ipairs(group.tr or {}) do
                    local line = translation.l
                    if line then
                        local values = {}
                        collect_texts(line.i, values)
                        for _, value in ipairs(values) do
                            add(english, value)
                        end
                    end
                end
            end
        end
    end

    local web = data.web_trans and data.web_trans["web-translation"]
    if type(web) == "table" then
        for _, entry in ipairs(web) do
            if entry.key == word or entry["@same"] == "true" then
                for _, translation in ipairs(entry.trans or {}) do
                    add(translations, text_value(translation.value))
                end
            end
        end
    end

    local lines = {}
    if #english > 0 then
        lines[#lines + 1] = "英文：" .. table.concat(english, "；")
    end
    if #translations > 0 then
        lines[#lines + 1] = "翻译：" .. table.concat(translations, "；")
    end
    if #lines == 0 then
        return nil
    end
    return table.concat(lines, "\n")
end

local function set_status(text, color)
    if not win then
        return
    end
    win:update_content("status", {
        { text = text or "", color = color or "#FF888888" },
    })
end

local function apply_theme()
    if not win then
        return
    end
    local theme = cloudime.get_theme()
    if not theme or not theme.candidate then
        return
    end
    local extra = theme.candidate.extra
    if extra and win.set_button_background then
        local ok, error = pcall(win.set_button_background, win, "search", extra)
        if not ok then
            cloudime.log("在线查询：设置搜索按钮主题色失败：" .. tostring(error))
        end
    end
    local highlight = theme.candidate.highlight
    if highlight and win.set_selection_background then
        local query_ok, query_error = pcall(
            win.set_selection_background, win, "query", highlight
        )
        local result_ok, result_error = pcall(
            win.set_selection_background, win, "result", highlight
        )
        if not query_ok then
            cloudime.log("在线查询：设置输入框选区颜色失败：" .. tostring(query_error))
        end
        if not result_ok then
            cloudime.log("在线查询：设置结果框选区颜色失败：" .. tostring(result_error))
        end
    end
end

local function ensure_window()
    if win then
        return win
    end
    local layout = cloudime.install_path("Scripts/lib/fast-searching-online.json")
    if not layout then
        cloudime.log("在线查询：找不到 Scripts/lib/fast-searching-online.json")
        return nil
    end
    win = cloudime.window.create{
        name        = "在线查询",
        width       = WIDTH,
        height      = HEIGHT,
        layout      = layout,
        interactive = true,
        background  = false,
        shadow      = true,
    }
    apply_theme()
    return win
end

local function clamp(value, minimum, maximum)
    if maximum < minimum then
        return minimum
    end
    return math.max(minimum, math.min(value, maximum))
end

local function place_at_caret(pos)
    local x = clamp(pos.left, pos.work_left, pos.work_right - WIDTH)
    local below = pos.bottom + EDGE_GAP
    local above = pos.top - EDGE_GAP - HEIGHT
    local y
    if below + HEIGHT <= pos.work_bottom then
        y = below
    elseif above >= pos.work_top then
        y = above
    else
        y = clamp(pos.work_bottom - HEIGHT, pos.work_top, pos.work_bottom)
    end
    return x, y
end

-- 按候选窗的最终内容尺寸推算它的屏幕矩形；候选窗未显示时返回 nil。
local function candidate_rect(pos)
    local width = cloudime.candidate.width()
    local height = cloudime.candidate.height()
    if not width or not height or width <= 0 or height <= 0 then
        return nil
    end
    local gap = 2
    local x = clamp(pos.left, pos.work_left, pos.work_right - width)
    local below = pos.bottom + gap
    local above = pos.top - gap - height
    local y
    if below + height <= pos.work_bottom then
        y = below
    elseif above >= pos.work_top then
        y = above
    else
        y = clamp(pos.work_bottom - height, pos.work_top, pos.work_bottom)
    end
    return x, y, width, height
end

local function place_beside_candidate(pos, has_candidate)
    if not has_candidate then
        return place_at_caret(pos)
    end
    local candidate_x, candidate_y, candidate_width, candidate_height = candidate_rect(pos)
    if not candidate_x then
        return place_at_caret(pos)
    end

    local x = candidate_x + candidate_width + EDGE_GAP
    if x + WIDTH > pos.work_right then
        x = candidate_x - WIDTH - EDGE_GAP
    end
    if x >= pos.work_left and x + WIDTH <= pos.work_right then
        local y = clamp(candidate_y, pos.work_top, pos.work_bottom - HEIGHT)
        return x, y
    end

    -- 左右都放不下：退回候选窗下方 / 上方。
    local below = candidate_y + candidate_height + EDGE_GAP
    local above = candidate_y - EDGE_GAP - HEIGHT
    local y
    if below + HEIGHT <= pos.work_bottom then
        y = below
    elseif above >= pos.work_top then
        y = above
    else
        y = clamp(pos.work_bottom - HEIGHT, pos.work_top, pos.work_bottom)
    end
    return clamp(candidate_x, pos.work_left, pos.work_right - WIDTH), y
end

local function request_query(text)
    if not win then
        return
    end
    text = trim(text or "")
    if text == "" then
        pending_query_text = nil
        win:set_text("result", "输入中文词语、汉字或英文单词后按回车，或点搜索按钮。")
        set_status("请输入查询内容", "#FFCC3333")
        return
    end

    local source, url
    if is_chinese_word(text) then
        source = "汉典"
        url = "https://zdic.net/hans/" .. url_encode(text)
    elseif is_english_word(text) then
        source = "有道"
        url = "https://dict.youdao.com/jsonapi?q=" .. url_encode(text)
    else
        pending_query_text = nil
        win:set_text("result", "目前支持中文词语 / 汉字和英文单词。")
        set_status("暂不支持这种文本", "#FFCC3333")
        return
    end

    if last_result_word == text and last_result_text then
        win:set_text("result", last_result_text)
        set_status("来自" .. (last_result_source or source) .. "词典", "#FF888888")
        return
    end

    if pending_query_text == text then
        return
    end
    local now = os.time()
    if last_query_text == text and now - last_query_at < 2 then
        return
    end
    last_query_text = text
    last_query_at = now
    pending_query_text = text
    request_seq = request_seq + 1
    local sequence = request_seq
    win:set_text("result", "正在查询「" .. text .. "」…")
    set_status("正在连接" .. source .. "…", "#FF888888")

    local function finish(parsed, result_source)
        if sequence == request_seq then
            pending_query_text = nil
        end
        if sequence ~= request_seq or not win then
            return
        end
        win:set_text("result", parsed)
        last_result_word = text
        last_result_source = result_source
        last_result_text = parsed
        set_status("来自" .. result_source .. "词典", "#FF888888")
    end

    cloudime.http_get(url, HTTP_TIMEOUT_MS, function(result)
        if sequence ~= request_seq or not win then
            return
        end
        if result.error then
            pending_query_text = nil
            win:set_text("result", "网络请求失败：" .. tostring(result.error))
            set_status("查询失败", "#FFCC3333")
            return
        end
        if result.status ~= 200 then
            pending_query_text = nil
            win:set_text("result", source .. "返回 HTTP " .. tostring(result.status))
            set_status("没有找到词条", "#FFCC3333")
            return
        end

        if source == "汉典" then
            local parsed = parse_zdic(result.body, text)
            if not parsed then
                pending_query_text = nil
                win:set_text("result", "页面里没有找到可显示的释义。\n" .. url)
                set_status("未找到释义", "#FFCC3333")
                return
            end
            set_status("正在补充有道…", "#FF888888")
            local youdao_url = "https://dict.youdao.com/jsonapi?q=" .. url_encode(text)
            cloudime.http_get(youdao_url, HTTP_TIMEOUT_MS, function(youdao)
                if sequence ~= request_seq or not win then
                    return
                end
                local extra = nil
                if not youdao.error and youdao.status == 200 then
                    extra = parse_youdao_cn(youdao.body, text)
                end
                if extra then
                    finish(parsed .. "\n\n" .. extra, "汉典 + 有道")
                else
                    finish(parsed, "汉典")
                end
            end)
            return
        end

        local parsed = parse_youdao(result.body, text)
        if not parsed then
            pending_query_text = nil
            win:set_text("result", "页面里没有找到可显示的释义。\n" .. url)
            set_status("未找到释义", "#FFCC3333")
            return
        end
        finish(parsed, "有道")
    end)
end

local function open_panel(pos)
    local was_open = win ~= nil
    if not ensure_window() then
        return
    end

    local word = trim(pending_word)
    local same_query = word ~= ""
        and word == last_query_text
        and os.time() - last_query_at < 2
    if not same_query then
        request_seq = request_seq + 1
        pending_query_text = nil
    end

    if word ~= "" then
        win:set_text("query", word)
    elseif not was_open then
        win:set_text("query", "")
    end

    local x, y = place_beside_candidate(pos, pending_has_candidate)
    win:show{ x = x, y = y }
    win:focus(true)
    win:focus_text("query")

    if word ~= "" and (is_chinese_word(word) or is_english_word(word)) then
        request_query(word)
    elseif word ~= "" then
        win:set_text("result", "目前支持中文词语 / 汉字和英文单词。")
        set_status("暂不支持这种文本", "#FFCC3333")
    elseif not was_open then
        win:set_text("result", "输入中文词语、汉字或英文单词后按回车，或点搜索按钮。")
        set_status("请输入查询内容", "#FF888888")
    end
end

cloudime.on("key", function(event)
    if event.vk ~= 0x51 then -- Q
        return {}
    end
    if waiting_caret then
        return {}
    end
    pending_word = event.highlight or ""
    pending_has_candidate = pending_word ~= ""
    waiting_caret = true
    cloudime.caret()
    return {}
end)

cloudime.on("caret", function(pos)
    if not waiting_caret then
        return
    end
    waiting_caret = false
    open_panel(pos)
end)

cloudime.on("window", function(event)
    if not win or event.id ~= win.id then
        return
    end
    if event.handler == "close" then
        request_seq = request_seq + 1
        pending_query_text = nil
        last_query_text = nil
        last_query_at = 0
        last_result_word = nil
        last_result_source = nil
        last_result_text = nil
        win:close()
        win = nil
        return
    end
    if event.handler == "search" then
        request_query(win:get_text("query"))
        return
    end
    if event.handler == "query_key"
        and event.action == "key_pressed"
        and event.key == 13 then
        request_query(win:get_text("query"))
    end
end)
