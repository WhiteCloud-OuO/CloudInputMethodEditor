-- 示例：把当前输入框的**整篇文本**复制到剪贴板（`cloudime.text.all` + `cloudime.clipboard` 的测试样例）。
--
-- 用法：把这份脚本拷到 `Scripts\` **根部**（`lib\` 里的不会被加载），重启输入法服务，
-- 然后在任何输入框里**打字组句**时按 `Ctrl+A`：整篇文本进剪贴板，候选窗上提示复制了多少字。
--
-- 两个要知道的：
--   1. `Ctrl+A` 只在**组句里**（候选窗显示着）到得了脚本 —— 这是「带 Ctrl 的组合只在组句里问脚本」
--      那条规则的结果；不在组句时它照旧是应用的「全选」。
--   2. 整篇可能一次拿不到（刚启动、或换了输入框之后的第一次调用是 `nil`，Server 已经在请 DLL 读了）：
--      这时按提示**再按一次** `Ctrl+A` 即可。
--
-- 剪贴板走的是 `cloudime.clipboard.settext` —— Lua 标准库里碰不到剪贴板，这接口由 Server 用 Windows
-- 剪贴板 API 实现（不用借外部程序绕一圈）。

cloudime.script{
    name        = "复制全文到剪贴板",
    description = "组句里按 Ctrl+A：把输入框整篇文本复制到剪贴板（测试 cloudime.text.all）",
    api         = 1,
    timeout     = 500,
    sync        = true,
    handover    = "return",
    on_error    = function(event, message)
        cloudime.log("复制全文出错：" .. event .. " → " .. message)
    end,
}

-- 数一下有多少个字符（Lua 5.1 没有 utf8 库，用模式逐字抓）。
local function char_count(text)
    local count = 0
    for _ in text:gmatch("[\1-\127\194-\244][\128-\191]*") do
        count = count + 1
    end
    return count
end

cloudime.on("key", function(event)
    if not (event.ctrl and event.vk == 65) then
        return                                    -- 只认 Ctrl+A（A = 0x41）
    end
    local box = cloudime.text.all()
    if not box then
        return { notice = "整篇还没读到，再按一次 Ctrl+A" }
    end

    -- 剪贴板可能被别的程序占着（`OpenClipboard` 打不开）：接口会报错，这里用 pcall 接住
    local ok, error = pcall(cloudime.clipboard.settext, box.text)
    if not ok then
        return { notice = "写剪贴板失败：" .. tostring(error) }
    end
    cloudime.log(string.format("复制全文：%d 字%s", char_count(box.text), box.truncated and "（被截断）" or ""))

    return {
        notice = string.format(
            "已复制 %d 字到剪贴板%s",
            char_count(box.text),
            box.truncated and "（整篇被截断）" or ""
        ),
    }
end)
