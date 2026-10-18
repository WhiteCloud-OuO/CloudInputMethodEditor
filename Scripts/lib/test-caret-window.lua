-- 测试：光标位置 + 独立窗口 + SVG 文件 + 区域点击关闭。
--
-- 用法：把这个文件从 `Scripts\lib\` 拷到 `Scripts\` 下（或直接放在 `Scripts\` 里），重启输入法服务。
-- 在任意文本编辑器里按 **Ctrl + Alt + F3**：
--   * 光标上方 30 点摆一块 180 × 140 的独立窗口；
--   * 上方放不下（会超出显示器工作区）就改到光标下方 30 点；
--   * 右上角 20 × 20、圆角 3 的区域里画 `Images\SVG\打叉.svg`，点它关闭并销毁窗口；
--   * 窗口内容用静态控件搭：`panel` / `label`（每段颜色不同）/ `text_block`（多行 + 行距）/
--     `static_shape`（分隔线 + 圆角色块）/ `static_image`（测试机上的 `Images\cloud.png`，可选）；
--   * 鼠标不在这个区域里：叉是黑色、没有底色；移进去：底色变成 `#FFFF3300`、叉变成白色（模拟关闭按钮）；
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
local PANEL_RADIUS = 8
local BUTTON = 20
local MARGIN = 8
local BUTTON_X = WIDTH - BUTTON - MARGIN
local BUTTON_Y = MARGIN

local win = nil
local waiting = false
local hovering = false
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
        { type = "label", x = 56, y = 10, size = 16,
          runs = { { text = "云朵", color = color.accent },
                   { text = "输入法", color = color.text } } },
        { type = "text_block", x = 56, y = 36, size = 12, line_spacing = 4,
          text = "180 × 140\nCtrl+Alt+F3 只创建", color = color.muted },
        { type = "static_shape", shape = "line", from = { x = 8, y = 78 },
          to = { x = WIDTH - MARGIN * 2 - 8, y = 78 }, thickness = 1, color = color.muted },
        { type = "static_shape", shape = "round_rect", x = 8, y = 86, w = 148, h = 26,
          radius = 6, color = color.highlight },
        { type = "label", x = 16, y = 91, size = 12,
          runs = { { text = "静态控件演示", color = color.text } } },
    }
    local icon = cloud_icon()
    if icon then
        table.insert(children, 1, { type = "static_image", x = 8, y = 8, w = 40, h = 40,
                                    image = icon })
    end
    local content = {
        width = WIDTH,
        height = HEIGHT,
        regions = {
            -- 铺满整个内容区：鼠标从关闭区域挪到窗口别处时也能收到 hover（好把按钮底色收起来）。
            -- 后写的 close 盖住它。
            { name = "body", x = 0, y = 0, w = WIDTH, h = HEIGHT },
            -- 右上角的关闭区域：圆角矩形，按住它不拖窗口（点一下才是关闭）
            { name = "close", x = BUTTON_X, y = BUTTON_Y,
              w = BUTTON, h = BUTTON, radius = 3, drag = false },
        },
        -- 窗口背景由脚本画（`create` 用了 background = false）；颜色跟随当前主题。
        { type = "rect", x = 0, y = 0, w = WIDTH, h = HEIGHT, radius = PANEL_RADIUS,
          color = color.background },
        -- 面板：把下面这些静态控件分组；缺省无填充，只画 #FFE0E0E0 的 1 点边框。
        { type = "panel", x = MARGIN, y = MARGIN, w = WIDTH - MARGIN * 2, h = HEIGHT - MARGIN * 2,
          children = children },
    }
    -- 鼠标在关闭区域里才画按钮底色；先画它、再画叉（叉在上面）。
    if hovering then
        content[#content + 1] = {
            type = "rect", x = BUTTON_X, y = BUTTON_Y, w = BUTTON, h = BUTTON,
            radius = 3, color = "#FFFF3300",
        }
    end
    content[#content + 1] = {
        type = "svg", x = BUTTON_X, y = BUTTON_Y, size = BUTTON, path = svg_path(),
        tint = hovering and "#FFFFFFFF" or "#FF000000",
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
    hovering = false
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
    if event.action == "hover" then
        -- body 区域铺满窗口，鼠标离开 close 时这里也会收到一次 hover
        local inside = event.region == "close"
        if inside ~= hovering then
            hovering = inside
            draw()
        end
    elseif event.action == "leave" then
        if hovering then
            hovering = false
            draw()
        end
    elseif event.action == "click" and event.region == "close" then
        win:close()
        win = nil
        hovering = false
    end
end)
