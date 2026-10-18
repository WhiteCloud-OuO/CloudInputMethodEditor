# docs

README.md 只介绍项目，所有技术内容放在这里，分四类：前三类是给开发者看的内部文档，`user/` 是给用户看的、官网渲染的文档。

| 目录 | 放什么 |
|---|---|
| [design/](design/) | 设计：现在是什么样、为什么这么定 |
| [plan/](plan/) | 计划：接下来做什么、按什么顺序 |
| [notes/](notes/) | 工程记录：性能优化史、事故复盘、踩坑，按时间顺序写，以后复习与面试用 |
| [user/](user/) | 用户文档：官网「文档」页的内容源，按目录结构渲染；维护约定见 [user/README.md](user/README.md) |

| 文件 | 内容 |
|---|---|
| [contributing.md](contributing.md) | 开发约定：架构约束短版、代码组织、版本号、提交信息、文档同步、提交前检查、发版与外部 PR 流程（CLAUDE.md 直接载入它） |
| [design/architecture.md](design/architecture.md) | Core 与平台层的划分、crate 结构、必须遵守的架构约束、`.qj` 数据容器 |
| [design/candidate-ui.md](design/candidate-ui.md) | 候选窗口与按键约定的设计 |
| [design/landscape.md](design/landscape.md) | 同类项目（水杉、Rime）、可用数据源及其许可 |
| [design/rendering.md](design/rendering.md) | UI 与自绘渲染器（2026-09-13）：显示面与控件面的区分、Flutter / Avalonia / WebView 的调研结论、主题的两条路、tiny-skia + cosmic-text 的 spike 结果与 Windows 接入 |
| [design/theme.md](design/theme.md) | 主题（2026-10-09）：三个窗口的 21 个颜色收进一份 JSON，两个目录的优先级、从改色到热加载这条链、设置页「主题」页，以及还没做的（字体 / 布局 / 注解色） |
| [design/script.md](design/script.md) | 用户脚本（2026-10-07）：脚本清单（manifest）与硬规则、没脚本就走老路、有脚本在固定四处跑、调用链路（一次按键走一遍）、能给脚本看到什么 / 改什么，以及出错、稳定性闸门与风险怎么兜 |
| [design/ui-layout.md](design/ui-layout.md) | 前台 UI 布局文件（2026-10-18）：JSON 控件格式、与 `set_content` 元素表的字段对应、动态控件 / 区域的 `events` 响应、`create{ layout = … }` 与 `load_layout` 接口 |
| [design/text-box.md](design/text-box.md) | 多行编辑框 `text_box`（2026-10-18 设计）：背景 / 文本区 / 内部水平与垂直滚动条的组成，自动换行、行距、两遍布局、复用 RichEdit 与文本 API / 事件的规则 |
| [design/edit-todo.md](design/edit-todo.md) | 自绘文本编辑器待办（暂缓）：去掉 Windowless RichEdit / `EDIT` 的可行性结论、目标架构、TSF composition 风险、分阶段任务和启动条件 |
| [design/online-translate.md](design/online-translate.md) | 在线翻译（2026-10-07）：输入法里**没有**这个功能 —— 它只给 `http_get` / `http_post` 与候选窗底部那一行（`online` 动作），翻什么、按哪个键、怎么签名全由脚本自己实现；随包一份小牛翻译的完整示例 |
| [design/input-schemes.md](design/input-schemes.md) | 输入方案（2026-10-10）：三种方案单选、**永不混输**；双键拼音的方案文件格式（INI/TOML 双格式与「同名认 toml」、目录与 `use_input_scheme.cfg`、Info / Auxiliary / Shengmu / Exchange / Remapping 五节、零声母写死在 `[Shengmu]` 里、按键变换流水线、校验与报错）与内置样例（首道 / 小鹤 / 自然码） |
| [design/windows.md](design/windows.md) | Server 侧自绘窗口的模型（2026-10-14）：两类窗口（独立窗口 / 辅助窗口）、所有者唯一且不转移、桥接（bridging）、焦点（focus，2026-10-17：一组里任何时刻恰好一个 / 默认不具备焦点 / 允许获取焦点 / 指定资产获取焦点 / 同一所有者的两件资产经桥接转移 / 聚焦与失焦 / 归还给所有者 / 克隆窃取来的不抢焦点）、画法：渲染（rendering）与取舍、资产克隆与资产窃取（暂不用，先定规则）、两个模板与实现落点 |
| [plan/roadmap.md](plan/roadmap.md) | 分阶段路线图、各阶段的依赖关系与已完成项 |
| [plan/todo.md](plan/todo.md) | 待办清单，按「从自用到能给别人用」排 |
| [notes/crate-notes.md](notes/crate-notes.md) | 各 crate / app / tool 的实现要点：入口类型、数据文件、常数、生成命令 |
| [notes/performance.md](notes/performance.md) | 历次性能优化：起因、定位方法、改法、数字前后对比与经验 |
| [notes/release.md](notes/release.md) | 发版流程：CHANGELOG、标签触发的 CI 打包、产品数据包、签名的 Secrets、官网用的 `releases.json` |
| [notes/phrase-layer.md](notes/phrase-layer.md) | 短语层（2026-09-12）：常用词表收不到的 我的 / 不知道 怎么从语料挖、怎么进语言模型而不伤整句、两把尺子的前后数字 |
| [notes/domain-words.md](notes/domain-words.md) | 领域词（2026-09-12）：从输入日志人工挑 48 条进基础词库的挑法、低频词当 token 统计为什么伤整句、两把尺子的前后数字 |
| [notes/constant-sweep.md](notes/constant-sweep.md) | 排序常数扫描（2026-09-12）：插值与敲错代价在冻结日志上扫网格，全在平台区不改；没命中的构成与复现步骤 |
| [notes/windows-win10.md](notes/windows-win10.md) | Windows 10 与设置程序（2026-09-13）：Reactor 早期绑定 Windows 11 才有的 AppModel API 导致加载期失败，改自包含部署 + 延迟加载 |

约定：文档写中文，代码标识符一律英文。实现与文档产生分歧时以代码为准，并同步更新文档。
