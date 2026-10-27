-- 列表盒行子页面脚本示例。
--
-- 路径写在子页面 JSON 的 page.script（或 subpage 节点的 script）：
--   "script": "test-list-box-row.lua"
-- 文件固定从 Scripts/shared/ 解析。
--
-- 每个实例会收到一次 created；控件事件先派给这里，返回 true 可吞掉主窗口脚本。
-- ctx 字段：window / subpage / instance / index / key / item / path(name)。

local page = {}

function page.created(ctx)
    -- 可以在这里按 item 初始化控件，例如：
    -- ctx.window:set_text_at(ctx.path("edit"), ctx.item.value)
end

function page.event(ctx, event)
    -- 例如：
    -- if event.action == "text_changed" and event.control == "edit" then
    --     ctx.window:set_text_at(ctx.path("title"), "编辑中 " .. ctx.index)
    -- end
    return false
end

function page.destroyed(ctx)
    -- 实例滚出虚拟窗口或数据删除时调用。
end

return page
