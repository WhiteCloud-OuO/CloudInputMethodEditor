-- 云朵输入法 脚本模板
--
-- 用法：「设置 → 脚本 → 新建脚本」会用这份模板在安装目录的 Scripts\ 下建一个新文件、并打开记事本，
-- 改完按 Ctrl + S 保存。**改脚本要重启输入法服务**才生效（「设置 → 脚本」页里那个
-- 「重启输入法服务」按钮，或任务栏「中 / 英」图标右键的那一项）。
--
-- 完整说明（清单每个字段、事件与载荷、能返回的动作、cloudime 表的全部方法、闸门与例子）见
-- 安装目录下的 **lua.md**（仓库根目录同名文件）。下面只留清单，写法照它抄。
--
-- 规则（违反的脚本一律当无效、不加载）：
--   1. 必须在文件**最前面**声明一次清单 cloudime.script{…}（下面那块）；缺或写错 → 无效。
--   2. cloudime.on 只能在加载期调，运行期再登记会被拒绝。
--   3. 清单里 sync / handover / timeout / on_error 必写；budget 只能比全局上限小。
--   4. 不许改 cloudime 表；脚本自己的持久状态写别处（别写安装目录 —— 升级 / 卸载会删）。
--   5. 唯一允许的「等」是 cloudime.http_get 与 cloudime.http_post（异步；时限必给，且不能超过清单里的 timeout）。

cloudime.script{
    name        = "我的脚本",           -- 日志里点名用；不写就用文件名
    description = "一句话介绍",         -- 设置页列表里显示；不写就显示文件名
    api         = 2,                    -- 面向的脚本 API 版本（现在只有 2）
    -- 由什么触发（必写，五选一）：
    --   combination_key   按下组合键（不论候选窗口开没开）→ 还要写 combination_modifiers + keys
    --   time_range        某个时间段里触发**一次** → 还要写 time_range = { from = "20:00", to = "06:00" }
    --   candidate_context 候选窗口出现、拼音串等于指定内容 → 还要写 candidate_pinyin = "nihao"
    --   focus_app         编辑光标位于指定程序 → 还要写 apps = { "Code.exe" }
    --   manual            手动触发（i 模式，预留：现在不会自动跑）
    trigger_condition = "combination_key",
    combination_modifiers = "ctrl+alt",  -- 必写：none（不按修饰键）/ ctrl / ctrl+alt / ctrl+shift / alt+shift / alt / win+ctrl / win+alt / win+shift / win+alt+ctrl / win+ctrl+shift
    keys = { "j" },                       -- 必写：要哪几个键（单个字母 / 数字 / 标点，或 enter / tab / space / backspace / delete / esc / left up right down / home / end / pageup / pagedown / f1..f12）
    -- time_range = { from = "20:00", to = "06:00" },   -- time_range 时必写（跨夜也行，每个时间段只触发一次）
    -- candidate_pinyin = "nihao",                      -- candidate_context 时必写（比较时忽略 ' 与大小写）
    -- apps = { "Code.exe" },                           -- focus_app 时必写
    budget       = 200000,               -- 单次调用的指令数上限（不写 = 全局上限；只能更小）
    timeout      = 200,                  -- 单次调用的墙钟上限（毫秒，1–60000）
    apps         = { },                  -- 可选：只在哪些应用里跑（exe 文件名，大小写不敏感）；空 = 所有应用
    needs_context = false,               -- 可选：true = 每次派发前刷新 cloudime.context（按键前多读一次光标前文）
    sync         = true,                 -- 同步：处理函数算完就返回
    handover     = "return",             -- 必须与 sync 一致：true 配 "return"、false 配 "callback"
    on_error     = function(event, message)  -- 处理函数出错时调它；写 false = 不处理（直接中止这一次）
        cloudime.log("出错：" .. event .. " → " .. message)
    end,
    priority     = 0,                    -- 多个脚本都要改同一项时，大的盖小的（同值按文件名）
}
