-- 测试：光标位置 + 独立窗口 + SVG 文件 + 区域点击关闭。
--
-- 用法：把这个文件从 `Scripts\lib\` 拷到 `Scripts\` 下（或直接放在 `Scripts\` 里），重启输入法服务。
-- 在任意文本编辑器里按 **Ctrl + Alt + F3**：
--   * 光标上方 30 点摆一块 180 × 140 的独立窗口；
--   * 上方放不下（会超出显示器工作区）就改到光标下方 30 点；
--   * 右上角 20 × 20、圆角 3 的区域里画 `Images\SVG\打叉.svg`，点它关闭并销毁窗口。
-- F3 只负责创建：窗口已经开着时再按不会关它。
--
-- 用到的接口：
--   * `cloudime.caret()`：请 DLL 量一次插入点；结果以 `caret` 事件送来（屏幕坐标 / 点）；
--   * `cloudime.install_path(...)`：安装目录里的文件绝对路径（`Images\` 与 `Scripts\` 同级）；
--   * `svg` 元素的 `path` 字段：直接从 SVG 文件读文本（不再要求脚本内嵌 source）。

cloudime.script{
    name                  = "光标窗口测试",
    description           = "Ctrl+Alt+F3：光标上方 30 点开一块 180×140 的测试窗口（右上角 × 关闭）",
    api                   = 2,
    trigger_condition     = "combination_key",
    combination_modifiers = "ctrl+alt",
    keys                  = { "f3" },
    timeout               = 200,
    sync                  = true,
    handover              = "return",
    on_error              = function(event, message)
        cloudime.log("光标窗口测试出错：" .. event .. " → " .. message)
    end,
}

local WIDTH, HEIGHT, GAP = 180, 140, 30
local BUTTON = 20
local MARGIN = 8
local BUTTON_X = WIDTH - BUTTON - MARGIN
local BUTTON_Y = MARGIN

local win = nil
local waiting = false
local close_svg = nil

local function svg_path()
    if close_svg == nil then
        close_svg = cloudime.install_path("Images/SVG/打叉.svg")
    end
    return close_svg
end

local function draw()
    win:set_content{
        width = WIDTH,
        height = HEIGHT,
        regions = {
            -- 右上角的关闭区域：圆角矩形，按住它不拖窗口（点一下才是关闭）
            { name = "close", x = BUTTON_X, y = BUTTON_Y,
              w = BUTTON, h = BUTTON, radius = 3, drag = false },
        },
        { type = "text", x = MARGIN + 8, y = 18, text = "光标窗口", size = 16, color = "#FF4A90E2" },
        { type = "text", x = MARGIN + 8, y = 44, text = "180 × 140", size = 12, color = "#FF888888" },
        { type = "text", x = MARGIN + 8, y = 66, text = "Ctrl+Alt+F3 只创建", size = 12,
          color = "#FF888888" },
        { type = "svg", x = BUTTON_X, y = BUTTON_Y, size = BUTTON, path = svg_path() },
    }
end

-- 按光标矩形摆窗口：优先上方 30，放不下就下方 30；再夹在当前显示器工作区里。
local function place(pos)
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
    if event.vk ~= 0x72 then               -- F3
        return
    end
    -- 已经开着就什么都别做：这个键只负责「创建」，不负责关闭。
    if win then
        return
    end
    waiting = true
    cloudime.caret()
    return {}
end)

cloudime.on("caret", function(pos)
    if not waiting then
        return
    end
    waiting = false
    if win then
        return
    end
    local path = svg_path()
    if not path then
        cloudime.log("光标窗口测试：找不到 Images/SVG/打叉.svg")
        return
    end
    win = cloudime.window.create{
        name        = "光标窗口测试",
        width       = WIDTH,
        height      = HEIGHT,
        interactive = true,
    }
    draw()
    place(pos)
end)

cloudime.on("window", function(event)
    if event.action ~= "click" then
        return
    end
    if win and event.id == win.id and event.region == "close" then
        win:close()
        win = nil
    end
end)
