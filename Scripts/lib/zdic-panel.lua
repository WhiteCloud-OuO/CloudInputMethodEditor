-- 汉典查询面板：Ctrl + Alt + Q。
--
-- 有候选窗口时：
--   * 查询词 = 当前高亮候选；
--   * 面板摆在候选窗口旁边（右边放不下就放左边，再不行放候选窗下方 / 上方）；
--   * 候选是中文词语 / 汉字时自动查询。
-- 没有候选窗口时：直接在光标附近打开空面板，输入中文后按回车或点搜索。
--
-- 布局在 Scripts/lib/zdic-panel.json；本文件放到 Scripts\ 下后重启输入法服务即可。
-- 英文单词查询暂未实现：只做中文词语 / 汉字的识别与查询。

cloudime.script{
    name                  = "汉典查询",
    description           = "Ctrl+Alt+Q：查询高亮候选词或输入中文词语，在独立面板里显示汉典释义",
    api                   = 2,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",
    keys                  = { "q" },
    timeout               = 12000,
    sync                  = false,
    handover              = "callback",
    on_error              = function(event, message)
        cloudime.log("汉典查询出错：" .. event .. " - " .. message)
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

    local english = first_html(html, '<span class="xxjs%-english__text">(.-)</span>')
    if english then
        lines[#lines + 1] = "英文：" .. english
    end

    local translations = collect_html(html, '<dd class="word%-trans__text">(.-)</dd>')
    if #translations > 0 then
        lines[#lines + 1] = "翻译：" .. table.concat(translations, "；")
    end

    if #definitions == 0 and #guoyu == 0 and #translations == 0 then
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
    local extra = theme and theme.candidate and theme.candidate.extra
    if not extra or not win.set_button_background then
        return
    end
    local ok, error = pcall(win.set_button_background, win, "search", extra)
    if not ok then
        cloudime.log("汉典查询：设置搜索按钮主题色失败：" .. tostring(error))
    end
end

local function ensure_window()
    if win then
        return win
    end
    local layout = cloudime.install_path("Scripts/lib/zdic-panel.json")
    if not layout then
        cloudime.log("汉典查询：找不到 Scripts/lib/zdic-panel.json")
        return nil
    end
    win = cloudime.window.create{
        name        = "汉典查询",
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
        win:set_text("result", "输入中文词语后按回车，或点搜索按钮。")
        set_status("请输入中文词语 / 汉字", "#FFCC3333")
        return
    end
    if not is_chinese_word(text) then
        pending_query_text = nil
        win:set_text("result", "英文单词查询暂未支持。\n请先输入中文词语或汉字。")
        set_status("目前只支持中文词语 / 汉字", "#FFCC3333")
        return
    end

    if last_result_word == text and last_result_text then
        win:set_text("result", last_result_text)
        set_status("来自汉典 zdic.net", "#FF888888")
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
    set_status("正在连接汉典…", "#FF888888")

    local url = "https://zdic.net/hans/" .. url_encode(text)
    cloudime.http_get(url, HTTP_TIMEOUT_MS, function(result)
        if sequence == request_seq then
            pending_query_text = nil
        end
        if sequence ~= request_seq or not win then
            return
        end
        if result.error then
            win:set_text("result", "网络请求失败：" .. tostring(result.error))
            set_status("查询失败", "#FFCC3333")
            return
        end
        if result.status ~= 200 then
            win:set_text("result", "汉典返回 HTTP " .. tostring(result.status))
            set_status("没有找到词条", "#FFCC3333")
            return
        end

        local parsed = parse_zdic(result.body, text)
        if not parsed then
            win:set_text("result", "页面里没有找到可显示的释义。\n" .. url)
            set_status("未找到释义", "#FFCC3333")
            return
        end
        win:set_text("result", parsed)
        last_result_word = text
        last_result_text = parsed
        set_status("来自汉典 zdic.net", "#FF888888")
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

    if word ~= "" and is_chinese_word(word) then
        request_query(word)
    elseif word ~= "" then
        win:set_text("result", "当前候选不是中文词语 / 汉字。\n英文单词查询暂未支持。")
        set_status("目前只支持中文词语 / 汉字", "#FFCC3333")
    elseif not was_open then
        win:set_text("result", "输入中文词语后按回车，或点搜索按钮。")
        set_status("请输入中文词语 / 汉字", "#FF888888")
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
