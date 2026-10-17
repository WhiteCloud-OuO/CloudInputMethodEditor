-- 示例：脚本窗口 —— Ctrl+Alt+W 开关一块可点的面板，点面板里的方块计数 +1。
--
-- 这份在 `Scripts\lib\` 里，**不会被加载**（加载器只认 `Scripts\` 根部的 *.lua）。想用就把它拷到
-- `Scripts\` 下、重启输入法服务（设置 → 脚本 → 「重启输入法服务」，或任务栏「中 / 英」图标右键）。
--
-- 用到的能力（完整说明见安装目录的 lua.md）：
--   * `cloudime.window.create{…}`：建一块独立窗口（`interactive = true` 才接鼠标）；
--   * `win:set_content{…}`：换一整份内容（矩形 / 文字 / SVG / 图片……），坐标与尺寸都是点；
--   * `cloudime.on("window", …)`：窗口上的点击 / 悬停事件（`event.action` / `event.element`）；
--   * `win:show{ x, y }` / `win:hide()`：显示与收起（窗口不会因为收起而销毁）。
--
-- 窗口内容变了就整份重送（`draw()`），不需要自己记「哪一块脏了」。

cloudime.script{
    name                  = "脚本窗口示例",
    description           = "Ctrl+Alt+W：开关一块可点的面板（点了多少下）",
    api                   = 1,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",    -- 只收 Ctrl+Alt 的组合；W 在下面自己认
    timeout               = 200,
    sync                  = true,
    handover              = "return",
    on_error              = function(event, message)
        cloudime.log("脚本窗口示例出错：" .. event .. " → " .. message)
    end,
}

local count = 0
local shown = false
local win

-- 整份重画：面板背景 / 圆角 / 阴影由输入法按主题画，这里只摆内容
local function draw()
    win:set_content{
        width = 220,
        height = 96,
        { type = "text", x = 12, y = 10, text = "点下面的方块", size = 14, color = "#FF666666" },
        { type = "rect", x = 12, y = 36, w = 196, h = 44, radius = 10, color = "#FF4A90E2" },
        { type = "text", x = 12, y = 46, text = "点了 " .. count .. " 下", size = 18,
          color = "#FFFFFFFF" },
    }
end

cloudime.on("startup", function()
    win = cloudime.window.create{
        name = "脚本窗口示例",
        width = 220,
        height = 96,
        interactive = true,               -- 接鼠标（点击在下面的 window 事件里）
    }
    draw()                                -- 先备好内容；显示等用户按键
end)

cloudime.on("key", function(event)
    if event.vk ~= 0x57 then             -- 只认 W（虚拟键码 0x57）
        return
    end
    if shown then
        win:hide()
    else
        -- 屏幕坐标（点）：内容区左上角
        win:show{ x = 200, y = 200 }
    end
    shown = not shown
end)

cloudime.on("window", function(event)
    if event.action == "click" and event.element ~= nil then
        count = count + 1
        draw()                            -- 改完立刻看得见
    end
end)
