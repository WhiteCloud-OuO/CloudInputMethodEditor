-- 测试：光标位置 + 独立窗口 + SVG 文件 + 区域点击关闭。
--
-- 用法：把这个文件从 `Scripts\lib\` 拷到 `Scripts\` 下（或直接放在 `Scripts\` 里），重启输入法服务。
-- 在任意文本编辑器里按 **Ctrl + Alt + F3**：
--   * 光标上方 30 点摆一块 500 × 360 的独立窗口；
--   * 上方放不下（会超出显示器工作区）就改到光标下方 30 点；
--   * 右上角 20 × 20、圆角 3 的 `flat_svg_button` 画 `Images\SVG\打叉.svg`，点它的自动区域关闭并销毁窗口；
--   * 窗口内容用静态控件搭：`panel` / `label`（每段颜色不同）/ `text_block`（多行 + 行距）/
--     `static_shape`（分隔线 + 圆角色块）/ `static_image`（测试机上的 `Images\cloud.png`，可选）；
--   * 两个单行编辑框：一个带 SVG 图标、一个不带（系统 EDIT，点进去可以输入）；
--   * 扁平按钮的 normal / hovered / pressed 底色与叉色由控件自动切换（悬停橙红底 + 白叉）。
--   * 窗口背景 / 圆角由脚本画（`background = false`），颜色取自 `cloudime.get_theme()` 的当前主题；
--     阴影与候选窗同一套（`shadow` 缺省开启）。
-- F3 只负责创建：窗口已经开着时再按不会关它。
--
-- 用到的接口：
--   * `cloudime.caret()`：请 DLL 量一次插入点；结果以 `caret` 事件送来（屏幕坐标 / 点）；
--   * `cloudime.install_path(...)`：安装目录里的文件绝对路径（`Images\` 与 `Scripts\` 同级）；
--   * `svg` 元素的 `path` 字段：直接从 SVG 文件读文本（不再要求脚本内嵌 source）。

cloudime.script{
    name                  = "光标窗口测试",
    description           = "Ctrl+Alt+F3：光标上方 30 点开一块 500×360 的测试窗口（右上角 × 关闭）",
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

local WIDTH, HEIGHT, GAP = 500, 360, 30
local PANEL_RADIUS = 12
local BUTTON = 20
local MARGIN = 8
local BUTTON_X = WIDTH - BUTTON - MARGIN
local BUTTON_Y = MARGIN

local win = nil
local waiting = false
local close_svg = nil
local cloud_image = nil

local function svg_path()
    if close_svg == nil then
        close_svg = cloudime.install_path("Images/SVG/打叉.svg")
    end
    return close_svg
end

-- 安装目录里的云朵图：拿一次句柄（`Arc<Pixmap>`）反复用。
local function cloud_icon()
    if cloud_image == nil then
        local path = cloudime.install_path("Images/cloud.png")
        if path then
            cloud_image = cloudime.image.load(path)
        end
    end
    return cloud_image
end

-- 当前主题的候选窗配色（`cloudime.get_theme()`）；没接上主题时用浅色兜底。
local function palette()
    local theme = cloudime.get_theme()
    local candidate = theme and theme.candidate or {}
    return {
        background = candidate.background or "#FFFFFFFF",
        text       = candidate.text or "#FF222222",
        muted      = candidate.page_number or "#FF888888",
        accent     = candidate.extra or "#FF4A90E2",
        highlight  = candidate.highlight or "#FFF0C8FF",
    }
end

local function draw()
    local color = palette()
    -- 面板的子控件：云朵图是测试机上的可选素材（仓库里没有），有就放进来。
    local children = {
        { type = "label", x = 88, y = 18, size = 20,
          runs = { { text = "云朵", color = color.accent },
                   { text = "输入法", color = color.text } } },
        { type = "text_block", x = 88, y = 54, size = 14, line_spacing = 5,
          text = "500 × 360\nCtrl+Alt+F3 只创建", color = color.muted },
        { type = "static_shape", shape = "line", from = { x = 16, y = 110 },
          to = { x = WIDTH - MARGIN * 2 - 16, y = 110 }, thickness = 1, color = color.muted },
        { type = "static_shape", shape = "round_rect", x = 16, y = 122, w = WIDTH - MARGIN * 2 - 32, h = 28,
          radius = 8, color = color.highlight },
        { type = "label", x = 28, y = 127, size = 14,
          runs = { { text = "静态控件演示", color = color.text } } },
    }
    local icon = cloud_icon()
    if icon then
        table.insert(children, 1, { type = "static_image", x = 16, y = 16, w = 56, h = 56,
                                    image = icon })
    end
    local content = {
        width = WIDTH,
        height = HEIGHT,
        -- 窗口背景由脚本画（`create` 用了 background = false）；颜色跟随当前主题。
        { type = "rect", x = 0, y = 0, w = WIDTH, h = HEIGHT, radius = PANEL_RADIUS,
          color = color.background },
        -- 面板：把下面这些静态控件分组；缺省无填充，只画 #FFE0E0E0 的 1 点边框。
        { type = "panel", x = MARGIN, y = MARGIN, w = WIDTH - MARGIN * 2, h = HEIGHT - MARGIN * 2,
          children = children },
        -- 有 SVG 图标的单行编辑框（图标按背景高度缩放，放在左侧）
        { type = "text_edit", name = "search", x = 24, y = 172, w = 320, h = 32,
          svg_path = cloudime.install_path("Images/SVG/打勾.svg"), svg_align = "left",
          text = "在这里输入…" },
        -- 无 SVG 的单行编辑框（文本区居中）
        { type = "text_edit", name = "plain", x = 24, y = 216, w = 320, h = 32,
          text = "没有图标的输入框" },
        -- 扁平 SVG 按钮：区域 = 背景形状（20 × 20 取自 SVG 画板），三态颜色自动切换。
        { type = "flat_svg_button", name = "close", x = BUTTON_X, y = BUTTON_Y,
          svg_path = svg_path(), shape = "round_rect", radius = 3,
          normal  = { background = "#00000000", svg = "#FF000000" },
          hovered = { background = "#FFFF3300", svg = "#FFFFFFFF" },
          pressed = { background = "#FFCC2900", svg = "#FFFFFFFF" } },
    }
    win:set_content(content)
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
        background  = false,   -- 面板底色 / 圆角由上面的 draw() 自己画
        shadow      = true,    -- 与候选窗同一套阴影（缺省就是 true）
    }
    draw()
    place(pos)
end)

cloudime.on("window", function(event)
    if not win or event.id ~= win.id then
        return
    end
    if event.action == "mouse_clicked" and event.region == "close" then
        win:close()
        win = nil
    end
end)
