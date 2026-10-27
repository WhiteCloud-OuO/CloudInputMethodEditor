-- 测试：光标位置 + 独立窗口 + JSON 布局解析 + SVG 文件 + 区域点击关闭。
--
-- 用法：脚本放在安装目录 `Scripts\` 根部（设置页才看得见），布局放在 `Scripts\layouts\`；
-- 重启输入法服务后，在任意文本编辑器里按 `i` 进 i 模式，选中「光标窗口测试」运行：
--   * 光标上方 30 点摆一块 760 × 640 的独立窗口；
--   * 上方放不下（会超出显示器工作区）就改到光标下方 30 点；
--   * 窗口内容来自 `Scripts/layouts/test-caret-window.json`：静态控件、两个单行编辑框、
--     两个多行编辑框（自动换行 / 不换行）、文本按钮、独立滚动条和右上角扁平 SVG 关闭按钮；
--   * 文本按钮 `ok` 绑定 `default_button = "enter"`，点它或按 Enter 都会走 `clicked` 的
--     `confirm`；多行文本、字重、四态颜色、圆角背景和按钮阴影都在它身上看效果；
--   * `disabled_demo` 设 `enabled = false`，鼠标点它不应有任何反应；
--   * 取消按钮用 `events.clicked`，右上角扁平关闭按钮用 `events.mouse_clicked`；
--     脚本都在 `window` 事件里按 `event.handler` 分发；
--   * 右上角「列表盒」按钮会另开一个列表盒测试窗口：1000 行、可滚动、可选中、
--     每行带一个可编辑输入框；初始状态来自 `test-list-box.json` / `test-list-box-row.json`；
--   * 窗口背景 / 圆角由布局文件里的 `rect` 画（`create` 用 `background = false`），
--     阴影与候选窗同一套（`shadow` 缺省开启）。
-- 运行只负责创建：窗口已经开着时再运行不会关它。
--
-- 用到的接口：
--   * `cloudime.caret()`：请 DLL 量一次插入点；结果以 `caret` 事件送来（屏幕坐标 / 点）；
--   * `cloudime.layout_path(...)`：安装目录 `Scripts\layouts\` 里的布局绝对路径；
--   * `cloudime.window.create{ layout = ... }`：读 JSON 布局并作为初始内容。

cloudime.script{
    name                  = "光标窗口测试",
    description           = "i 模式运行：Scripts/layouts 里的 JSON 测试窗口（多行编辑框、文本按钮四态、右上角 × 关闭）",
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
local list_win = nil
local waiting = false

local function layout_path()
    return cloudime.layout_path("test-caret-window.json")
end

local function list_layout_path()
    return cloudime.layout_path("test-list-box.json")
end

local function close_list()
    if list_win then
        list_win:close()
        list_win = nil
    end
end

local function open_list()
    if list_win then
        return
    end
    local path = list_layout_path()
    if not path then
        cloudime.log("光标窗口测试：找不到 Scripts/layouts/test-list-box.json")
        return
    end
    local items = {}
    for i = 1, 1000 do
        items[i] = {
            id = i,
            label = "列表项 " .. i,
            value = "可编辑 " .. i,
        }
    end
    list_win = cloudime.window.create{
        name = "列表盒测试",
        width = 320,
        height = 480,
        layout = path,
        data = { items = items },
        interactive = true,
        background = false,
        shadow = true,
    }
    list_win:show{ x = 80, y = 80 }
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
        cloudime.log("光标窗口测试：找不到 Scripts/layouts/test-caret-window.json")
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
    if event.handler == "open_list" and event.control == "list_demo" then
        open_list()
        return
    end
    if list_win and event.id == list_win.id then
        if event.handler == "close" and event.control == "close" then
            close_list()
        end
        return
    end
    if not win or event.id ~= win.id then
        return
    end
    if event.handler == "confirm" and event.control == "ok" then
        close_list()
        win:close()
        win = nil
    elseif event.handler == "close" and event.control == "cancel" then
        close_list()
        win:close()
        win = nil
    elseif event.handler == "close" and event.control == "close" then
        close_list()
        win:close()
        win = nil
    elseif event.handler == "disabled_click" then
        cloudime.log("禁用按钮不应触发点击")
    end
end)
