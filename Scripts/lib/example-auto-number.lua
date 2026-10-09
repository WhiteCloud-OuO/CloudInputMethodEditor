-- 示例：自动序号 —— 在 `1.` 或 `一、` 这样的序号后面敲回车时，自动接着写下一条的序号（像 Word 那样）。
--
-- 这份在 `Scripts\lib\` 里，**不会被加载**（加载器只认 `Scripts\` 根部的 *.lua）。想用就把它拷到
-- `Scripts\` 下、重启输入法服务（设置 → 脚本 → 「重启输入法服务」，或任务栏「中 / 英」图标右键）。
--
-- 用到的能力（完整说明见安装目录的 lua.md）：
--   * 清单 `trigger_condition = "key"` + `keys = { "enter" }`：没组句也收得到回车（脚本不吃就重放回去）。
--   * `cloudime.context`：光标前文 —— 这类脚本每次按键前都会现刷一份，所以按回车时它已经是
--     刚敲完那一行的样子。
--   * 动作 `commit`：吃掉这一键、直接上屏这段文本 —— 回车必须由脚本自己吃，把「换行 + 新序号」一次写出去。
--
-- 只对多行编辑框有意义（单行输入框里换行会被忽略或变成空格）；序号只认「一整行就是一个序号」那种。

cloudime.script{
    name              = "自动序号",
    description       = "在 1. / 一、 后面敲回车时，自动写出下一条的序号",
    api               = 1,
    trigger_condition = "key",
    keys              = { "enter" },
    timeout           = 200,        -- 纯本地判断，很快
    sync              = true,       -- 同步：算完就返回
    handover          = "return",   -- 必须与 sync 一致
    on_error          = function(event, message)
        cloudime.log("自动序号出错：" .. event .. " → " .. message)
    end,
}

local COUNTER = { "一", "二", "三", "四", "五", "六", "七", "八", "九" }
local DIGIT = {
    ["一"] = 1,
    ["二"] = 2,
    ["三"] = 3,
    ["四"] = 4,
    ["五"] = 5,
    ["六"] = 6,
    ["七"] = 7,
    ["八"] = 8,
    ["九"] = 9,
}

-- 光标前那一行（不含换行符）。`cloudime.context` 是这类脚本每次按键前现刷的光标前文。
local function current_line()
    local text = cloudime.context
    if not text or text == "" then
        return nil
    end
    return text:match("[^\r\n]*$")
end

-- 中文数字 → 数字（"一"…"九十九"）
local function cn_value(text)
    local tens, ones = text:match("^(.*)十(.*)$")
    if not tens then
        return DIGIT[text]
    end
    local t = (tens == "") and 1 or DIGIT[tens]
    local o = (ones == "") and 0 or DIGIT[ones]
    if not t or not o then
        return nil
    end
    return t * 10 + o
end

-- 数字 → 中文数字（1…99）
local function cn_text(n)
    if n < 1 or n > 99 then
        return nil
    end
    if n < 10 then
        return COUNTER[n]
    end
    local tens = math.floor(n / 10)
    local ones = n % 10
    local head = (tens == 1) and "十" or (COUNTER[tens] .. "十")
    return head .. ((ones > 0) and COUNTER[ones] or "")
end

-- 上一次自己写出去的序号（如 "2."）：上屏后马上读回来时，有的宿主会少给最后一个字符（`2.` 读成 `2`），
-- 靠它把行补全，序号才接得下去。
local last_committed = nil

-- "1." / "1. " → "2." / "2. "；"一、" → "二、"（保持原来那个点后面的空格）
local function next_prefix(line)
    if last_committed and line ~= "" and last_committed:sub(1, #line) == line then
        line = last_committed
    end
    local digits, tail = line:match("^(%d+)%.(%s*)$")
    if digits then
        return tostring(tonumber(digits) + 1) .. "." .. tail
    end
    local chinese, tail = line:match("^([一二三四五六七八九十]+)、(%s*)$")
    if chinese then
        local n = cn_value(chinese)
        if n then
            local next = cn_text(n + 1)
            if next then
                return next .. "、" .. tail
            end
        end
    end
    return nil
end

cloudime.on("key", function(event)
    if event.vk ~= 0x0D then    -- 只认 Enter
        return
    end
    local line = current_line()
    if not line then
        return
    end
    local prefix = next_prefix(line)
    if prefix then
        last_committed = prefix
        -- Enter 由脚本吃：换行 + 新序号一起上屏（`\r\n` 是 TSF 文档里的换行）
        return { commit = "\r\n" .. prefix }
    end
end)
