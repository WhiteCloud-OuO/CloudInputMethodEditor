-- 测试：光标位置 + 独立窗口 + JSON 布局解析 + SVG 文件 + 区域点击关闭。
--
-- 用法：把这个文件从 `Scripts\lib\` 拷到 `Scripts\` 下（或直接放在 `Scripts\` 里），重启输入法服务。
-- 在任意文本编辑器里按 `i` 进 i 模式，选中「光标窗口测试」运行：
--   * 光标上方 30 点摆一块 760 × 640 的独立窗口；
--   * 上方放不下（会超出显示器工作区）就改到光标下方 30 点；
--   * 窗口内容来自 `Scripts/lib/test-caret-window.json`：静态控件、两个单行编辑框、
--     两个多行编辑框（自动换行 / 不换行）、独立滚动条和右上角扁平 SVG 关闭按钮；
--   * 关闭按钮用 `events.mouse_clicked = "close"` 声明事件，脚本在 `window` 事件里按
--     `event.handler` 分发；
--   * 窗口背景 / 圆角由布局文件里的 `rect` 画（`create` 用 `background = false`），
--     阴影与候选窗同一套（`shadow` 缺省开启）。
-- 运行只负责创建：窗口已经开着时再运行不会关它。
--
-- 用到的接口：
--   * `cloudime.caret()`：请 DLL 量一次插入点；结果以 `caret` 事件送来（屏幕坐标 / 点）；
--   * `cloudime.install_path(...)`：安装目录里的文件绝对路径；
--   * `cloudime.window.create{ layout = ... }`：读 JSON 布局并作为初始内容。

cloudime.script{
    name                  = "光标窗口测试",
    description           = "i 模式运行：光标上方 30 点开一块 760×640 的 JSON 布局测试窗口（含多行编辑框，右上角 × 关闭）",
    api                   = 2,
    trigger_condition     = "manual",
    timeout               = 200,
    sync                  = true,
    handover              = "return",
    on_error              = function(event, message)
        cloudime.log("光标窗口测试出错：" .. event .. " → " .. message)
    end,
}

local WIDTH, HEIGHT, GAP = 760, 640, 30

local win = nil
local waiting = false

local function layout_path()
    return cloudime.install_path("Scripts/lib/test-caret-window.json")
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

cloudime.on("manual", function(event)
    -- 已经开着就什么都别做：这个入口只负责「创建」，不负责关闭。
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
    local path = layout_path()
    if not path then
        cloudime.log("光标窗口测试：找不到 Scripts/lib/test-caret-window.json")
        return
    end
    win = cloudime.window.create{
        name        = "光标窗口测试",
        width       = WIDTH,
        height      = HEIGHT,
        layout      = path,
        interactive = true,
        background  = false,   -- 背景由 JSON 里的 rect 自己画
        shadow      = true,    -- 与候选窗同一套阴影（缺省就是 true）
    }
    place(pos)
end)

cloudime.on("window", function(event)
    if not win or event.id ~= win.id then
        return
    end
    if event.handler == "close" and event.control == "close" then
        win:close()
        win = nil
    end
end)
