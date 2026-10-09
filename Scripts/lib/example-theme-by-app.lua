-- 示例：按前台程序换主题 —— Alt + Shift + K 触发，前台是记事本（notepad.exe）就换成 Panic 主题，
-- 别的程序就换回 Default。
--
-- 这份在 `Scripts\lib\` 里，**不会被加载**（加载器只认 `Scripts\` 根部的 *.lua）。想用就把它拷到
-- `Scripts\` 下、重启输入法服务（设置 → 脚本 → 「重启输入法服务」，或任务栏「中 / 英」图标右键）。
--
-- 用到的能力（完整说明见安装目录的 lua.md）：
--   * 清单的 trigger_condition = "combination_key" + combination_modifiers = "alt+shift"
--     —— 只把 Alt+Shift 那一套组合键送上来（组句与否都一样，不用等候选窗）。
--   * cloudime.foreground_app() / `key` 载荷里的 app：当前前台程序的 exe 名。
--   * cloudime.apply_theme(名字)：换主题 —— 会把它写进配置 `[theme] curr_theme` 并**重启输入法服务**
--     （和设置页「应用主题」同一套：重启才一定换上、选择也留了下来）；候选窗开着时会等它关掉那一拍才换/重启。
--     ⚠️ 每换一次都会重启一次输入法服务（正在打字的程序断一下再连上，约 1–3 秒）—— 本示例是「偶尔按一下」的用法。
--
-- 注意：`Alt+Shift` 本身是 Windows 切输入法的热键（单独按才切），带 `K` 一般不会被系统截走；
-- 在你的机器上按了没反应就把它改成别的组合（清单里换一个词，比如 "ctrl+shift" / "alt"）。

cloudime.script{
    name                  = "按前台程序换主题",
    description           = "Alt+Shift+K：记事本用 Panic 主题，别的程序用 Default",
    api                   = 1,
    trigger_condition     = "combination_key",
    combination_modifiers = "alt+shift",   -- 只收 Alt+Shift 的组合；`K` 在下面自己认
    timeout               = 200,           -- 纯本地判断，很快
    sync                  = true,          -- 同步：算完就返回
    handover              = "return",      -- 必须与 sync 一致
    on_error              = function(event, message)
        cloudime.log("按前台程序换主题出错：" .. event .. " → " .. message)
    end,
}

cloudime.on("key", function(event)
    -- 只认 K（虚拟键码 0x4B）。Alt+Shift 那一套已经由清单筛过，这里再保险一层：
    -- 顺便挡住 Caps Lock 或别的键混进来的情况。
    if event.vk ~= 0x4B then
        return
    end

    -- 这一拍的前台程序（载荷里就有；取不到时再问一次）
    local app = event.app or cloudime.foreground_app()
    if app and app:lower() == "notepad.exe" then
        cloudime.apply_theme("Panic")
    else
        cloudime.apply_theme("Default")
    end
end)
