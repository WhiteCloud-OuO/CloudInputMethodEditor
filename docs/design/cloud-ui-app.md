# cloud-ui 独立应用宿主（2026-10-28）

目标：把 `cloud-ui` 从「Server 进程内的 IME 悬浮窗壳」扩展成 **Windows 专用的应用 UI 库**，
让独立 exe 也能复用它，方向类似 egui / gpui，但**不做跨平台**。

## 分层

```text
cloud-ui-core      页面树、布局、尺寸（无 Windows / 脚本依赖）
cloudime-render    Document -> 位图（字体、主题、SVG、图片）
cloud-ui           窗口壳、消息循环、Painter、输入、命中、文本宿主
  ├─ app/          普通应用窗口宿主（本次新增）
  ├─ view/         无 Lua 的控件树 + 主题
  ├─ text_edit/    windowless RichEdit 宿主（脚本窗口 / 独立应用共用）
  ├─ layered/      分层窗口贴图（IME 悬浮窗继续用）
  ├─ auxiliary/    辅助窗口 / 资产 / 焦点（IME 悬浮窗继续用）
  └─ script_window/ Lua 脚本窗口（feature = "script"）
```

`cloud-ui` 的默认 feature 改为空；只有 Server 显式开 `script`。独立 exe 默认不拖进
LuaJIT / 脚本窗口。

## 本次落地：普通窗口宿主

`crates/cloud-ui/src/app/`：

- `WindowOptions`：标题、初始客户区尺寸（逻辑点）、最小尺寸、`decorations`（`System` / `None`）、
  `top_most`（缺省 false）、`shadow` / `corner_radius`（无边框时用）、`caption_height`（自绘标题栏拖动区）。
- `Application`：`update(&mut Context)` + `view(&Context) -> Document`。
- `Context`：事件列表、客户区尺寸、DPI、`request_redraw()`、`close()`。
- `AppEvent`：尺寸变化、鼠标移动 / 离开、按键、滚轮、键盘、字符输入、焦点，
  `RegionClicked(name)`（左键在 `View::button` / 区域里按下并松开），以及
  `TextChanged` / `TextFocusChanged` / `TextKey`（`View::text_edit`）。
- `run_native(options, app)`：`Decorations::System` 注册普通 `WS_OVERLAPPEDWINDOW | WS_EX_APPWINDOW` 窗口；
  `Decorations::None` 改成分层弹出窗（`WS_POPUP | WS_EX_LAYERED`），四周留阴影白边、内容完全自绘。
  两条路径都跑 `GetMessage` / `DispatchMessage` 消息循环，`WM_PAINT` 时把 `Document` 渲染成位图；
  系统边框走 `StretchDIBits`，无边框走 `UpdateLayeredWindow`。
- DPI：进程设 PerMonitorV2，处理 `WM_DPICHANGED` / `WM_GETMINMAXINFO`。
- 线程：和现有 `cloud-ui` 一样只在 UI 线程使用，`Application` 不要求 `Send`。

示例程序：

```text
apps/cloud-ui-demo/     # 无边框窗口 + 自绘标题栏 + 主题切换 + 滚动日志列表 + 列表盒 + 单行/多行编辑 + 多窗口 + 托盘 + 窗口蒙版
```

运行：

```text
cargo run -p cloud-ui-demo
```

示例有自己的独立安装包（`apps/cloud-ui-demo/installer/`），但**不混进云朵输入法安装包**；
输入法安装包仍只装 `apps/windows` 下的 Server / TSF / 设置程序。

## 轻依赖拆分

新增 `crates/cloud-ui-types`：

- `LayoutMode`、`ItemNumberStyle`；
- `ThemeFile` / `ThemeColor` / `CandidateTheme` / `BarTheme` / `TipTheme` 与读写方法。

`cloudime-platform` 依赖它并 re-export，原有配置代码不用改。
`cloud-ui` 默认只依赖 `cloud-ui-types` + `cloudime-render`；
`auxiliary` / `auxiliary_window_template` / `independent_window_template` 收进 `ime` 特性，
`cloudime-platform` 变成可选依赖；`script = ["ime", "dep:cloudime-script"]`。
Server 继续通过 `features = ["script"]` 打开全部 IME 能力。

## 无 Lua 的 view 层

新增 `crates/cloud-ui/src/view/`：

- `View::new(width, height)`：根页面铺满窗口；
- 容器分两类：
  - **控件容器**（`Container`）：`panel`、`scrollable_page`；支持 `gap` / `padding`，
    `scrollable_page` 额外带固定视口、裁剪与滚动条；
  - **子页面容器**：`list_box`（`NodeKind::ListBox`，已实现）；`SubpageContainer` 预留给通用子页面，后续实现。
- 布局分组：`column` / `row` 用 `Stem`，只负责结构与布局，不算上面两类容器；
- 控件：`label`、`button`、`spacer`、`text_edit`、`text_box`，外加整窗 `background`；
- 布局：`cloud_ui_core` 的 measure-arrange，文字尺寸由 `Painter::measure_text` 提供；
- 输出：`View::build(measure)` 生成 `Document`，按钮 / 列表行同时写入 `Document.regions`；
- 事件：`Context::measure_text` 给 view 用，`AppEvent::RegionClicked(name)` 让应用知道
  左键点中了哪个按钮 / 行；`AppEvent::TextChanged` / `TextFocusChanged` / `TextKey` 交回文本编辑状态。

示例已经改成用 `View` 搭页面，不再手写 `Element`。

## 主题

`view::Theme` 是一套独立应用的颜色 / 字号 / 圆角，不读 IME 的 `ThemeFile`：

- 颜色：`background` / `surface` / `surface_alt` / `border` / `text` / `muted` / `accent` / `accent_text`；
- 尺寸：`radius` / `font_size` / `title_size` / `muted_size` / `button_height` / `row_height`；
- `Theme::dark()` / `Theme::light()` 两套缺省值，`Default` 是深色；
- 派生样式：`title()` / `label()` / `muted()` / `button()` / `panel()` / `list_box(height)` / `text_edit()` /
  `text_box(height)` / `scrollable_page(height)`。
  换主题 = 换一本 `Theme` 再重建一帧，控件样式跟着走。

## scrollable_page（可滚动页面，控件容器）

`scrollable_page` 的中文名是**可滚动页面**，类型是**控件容器**（与子页面容器并列的两类容器之一），
不是临时演示控件。它和 `panel` 同属控件容器，只是额外带视口裁剪、滚动偏移与滚动条；
当前已实现的子页面容器是 `list_box`（列表盒），通用 `SubpageContainer` 仍预留。

`View::scrollable_page(name, style, state, |ui| { ... })` 是固定视口高度的垂直滚动容器：

- 生成一块 `Element::SelfDrawingPanel`（`clip = true`），子控件摆在面板自己的图层里，
  超出视口的部分由渲染器的图层裁剪自然切掉，滚动条（轨道 + 滑块）画在子控件之后；
- `ScrollablePageState { offset }` 是应用保存的滚动偏移，`View` 按内容高度夹紧后使用；
- `View::build` 会记录 `ScrollablePageMetrics { rect, content_height, viewport_height }`，
  之后用 `View::scrollable_page_metrics(name)` 读取；`max_scroll()` / `clamp()` 给滚轮用；
- 应用按 `metrics.rect.contains(pointer)` 决定滚轮落在哪个容器上，自己改 `offset` 再重建；
- 面板里的按钮 / 列表行区域会平移到文档坐标并按面板矩形裁剪，滚出视口的行不再可点；
- `ScrollablePageStyle` 管视口高度、内边距、间隙、底板、圆角与滚动条颜色；
  `Theme::scrollable_page(h)` 给一套缺省值。

它承载**任意内容**；行列表用 `list_box`（整行显示、间隙平均、选中 / 悬停）。
嵌套的 `scrollable_page` 也支持：子面板坐标是相对上一层面板的，
`Document::text_edits()` / `text_boxes()` 会把 `clip` 合并成最内层可见区域。

## 列表盒（子页面容器）

`View::list_box(name, items, style, state)` 是已实现的**子页面容器**（`NodeKind::ListBox`），
生成 `Element::ListBox`，行由子页面式的行模板展开：

- 行由 `ListItem` 描述：`title`、可选右侧 `detail`、可选点击名 `action`；
- `ListBoxState` 保存 `selected` 与 `scroll_offset`。滚轮 / 选中状态由应用自己维护，下一帧传回来；
- 可点击行按 `action` 写 `Document.regions`，应用收到 `RegionClicked(action)`；
- 悬停取自 `View::pointer(x, y)`，渲染时按指针位置算行号；
- 行高 / 间隙 / 圆角 / 选中色 / 悬停色都在 `ListBoxStyle` 里，`Theme::list_box(h)` 给一套缺省值；
- 可见行不够填满视口时，`auto_item_gap_averaging`（缺省 true）把剩余高度平均进行间隙：
  行永远完整显示，不会在顶部 / 底部露出半行；需要保留固定 gap 时把它设为 false。
  行内容由渲染器的 `paint_scroll` 吸附到行首，滚到底时则按从下往上的整行窗口显示。
- `scrollable_page` 是给任意内容的控件容器；**行列表**请用 `list_box`（子页面容器），
  才会得到上面这套整行 / 均匀间隙规则。

## 文本编辑

`View::text_edit(name, text, style)` 生成 `Element::TextEdit`，`View::text_box(name, text, style)`
生成多行 `Element::TextBox`；真正的输入由 `crates/cloud-ui/src/text_edit/` 的 windowless RichEdit
宿主负责（与脚本窗口同一份实现）：

- 宿主的 `owner` 是应用窗口，`WS_EX_TOPMOST` 跟随 `WindowOptions::top_most`；
  不置顶时 `raise()` 用 `HWND_TOP`，不会浮到别的应用上面；
- 每帧 `AppWindow::sync_text_edits` 同时处理 `Document::text_edits()` 与 `text_boxes()`：
  坐标 = 内容区坐标 × DPI + 阴影白边，再 `ClientToScreen`；
- 多行宿主建好后先 `ensure_layout()` 并回写 `TextBox.h_scroll` / `v_scroll`，渲染器才画内部滚动条；
  `WM_TEXT_SCROLL` / `WM_TEXT_WHEEL` 只触发重画，偏移仍由 RichEdit 自己维护；
- 滚动容器里的编辑框按 placement 的 `clip` 收窄宿主矩形，完全滚出视口就隐藏；
- `AppEvent::TextChanged { name, text }` 把新文本交给应用（文本仍以应用状态为准），
  `TextFocusChanged` / `TextKey` 分别报焦点与按键；
- 主题切换导致宿主颜色变化时，按颜色重建宿主（`EditColors` 比较）；
- 文本编辑框仍要求应用每帧用自身状态重建 `View`，宿主只做投影，不持有应用数据。

## 无边框窗口与置顶

`WindowOptions` 加了三组开关：

- `decorations: Decorations`：`System`（缺省，系统边框 / 标题栏 / 任务栏项）或 `None`（无边框分层窗口）。
- `top_most: bool`（缺省 false）：直接加 `WS_EX_TOPMOST`；IME 侧两类窗口模板另有 `set_top_most(bool)`，
  内部用 `SetWindowPos(HWND_TOPMOST / HWND_NOTOPMOST)`，建好之后也能改。
- `shadow` / `corner_radius`：只在无边框时生效。阴影复用 `cloudime-render` 的 `Shadow::panel()`，
  在内容四周留出 `Shadow::panel().margin()` 的透明边；圆角同时写进主题（`Painter::set_corner_radius`），
  可见底板用 `View::background_rounded(color, radius)` 画。

无边框路径的细节：窗口尺寸 = 内容尺寸 + 两侧阴影白边；`UpdateLayeredWindow` 的目标点是
**窗口左上角**（位图已经包含白边，传内容区坐标会让窗口每帧往右下漂一个白边宽度）；
阴影白边内的 `WM_NCHITTEST` 回 `HTTRANSPARENT`（点到阴影等于点穿）；标题栏拖动 / 双击靠
`WindowOptions::caption_height`，不设时整块客户区都算普通内容。其余输入坐标统一减掉白边再除 DPI 缩放。
`decorations: System` 时这些逻辑全部不参与。

## 自定义标题栏

`View::title_bar(title, style, maximized)` 是一行普通控件，不依赖窗口模板：

- 标题 + 右侧三个图标按钮；默认最小化 / 最大化 / 还原使用设备像素字形，关闭按钮使用 SVG；
- 图标资产：`Images/SVG/减号.svg`、`最大化.svg`、`还原窗口.svg`、`打叉.svg` 仍在编译期用
  `include_str!` 读进 `view/title_bar.rs`；关闭始终用 SVG，其余三个在 `pixel_icons = false` 时作为 fallback；
- 默认 `TitleBarStyle::pixel_icons = true`：最小化 / 最大化 / 还原生成 `Element::WindowIcon`，
  渲染器按当前 DPI 把图标盒取整到设备像素，线宽统一为 `round(1.2 × DPI/96)` 个像素；
  三个字形共用同一线宽，不经过 SVG 等比缩放，因此不会出现「最小化 2px、最大化 1px」；
- 关闭图标和 `pixel_icons = false` 的自定义 SVG 走 `Element::Svg`：形状用 `fill="currentColor"`，
  生成时把 `IconButtonStyle::icon` / `hover_icon` 写进根节点 `color`，`tint` 留空，所以 SVG 自己的
  填充色（包括多色）会参与显示，只有 `currentColor` 部分跟随主题变（深色白、浅色深）；
- 按钮动作名：`MINIMIZE_ACTION` / `MAXIMIZE_ACTION` / `CLOSE_ACTION`，应用在
  `RegionClicked` 里分别调 `Context::minimize()` / `toggle_maximize()` / `close()`；
- `WindowOptions::caption_height: Some(h)` 告诉窗口层标题栏高度：这一段里**没有命中区域**
  的地方 `WM_NCHITTEST` 回 `HTCAPTION`（可拖动、双击最大化 / 还原）；落在任意
  `Document.regions` 上的地方回 `HTCLIENT`，按钮才能收到点击；标题栏以下照旧 `HTCLIENT`；
- 无边框窗口带标题栏时窗口样式补 `WS_MINIMIZEBOX | WS_MAXIMIZEBOX`；不加 `WS_THICKFRAME`，
  避免系统边框和系统命中区；
- 最大化：`WM_GETMINMAXINFO` 把 `ptMaxPosition` / `ptMaxSize` 设成当前显示器工作区；
  宿主检测 `IsZoomed` 后把阴影白边当 0、`render_document` 不再画阴影，内容贴满窗口；还原后恢复；
- 窗口命令（最小化 / 最大化 / 关闭）在 `Context` 里只排队，`WM_PAINT` 的借用结束后才执行
  `ShowWindow` / `DestroyWindow`；否则 `ShowWindow` 同步发 `WM_SIZE` 会重入 `APP_WINDOW` 的 `RefCell`；
- `Theme::title_bar()` 给深浅两套：关闭按钮悬停红底白叉，其余按钮悬停换底色；标题栏背景留空时透出窗口背景；
- SVG 路径（关闭 / 自定义 fallback）保持抗锯齿；最小化 / 最大化 / 还原的 fallback SVG 用
  `IconButtonStyle::anti_alias = false`，在根节点注入 `shape-rendering="crispEdges"`（resvg 关闭形状 AA）；
  默认的像素字形本身就不走 AA。

## 多窗口

`Context::open_window(options, app)` 在同一个 UI 线程上再开一块窗口：

- 所有窗口共用一个 `run_native` 消息循环；线程本地表是 `WINDOWS`（HWND → `AppWindow`），
  窗口过程按 HWND 找自己那块，不再假设线程上只有一个窗口；
- 每块窗口有自己的 `Application`、`Painter`、`Document`、RichEdit 宿主、标题栏和窗口命令；
- `open_window` 返回 `WindowId`，可用 `is_open` / `close` / `focus` / `is_maximized`；
  新窗口先登记，等调用它的那一帧 `WM_PAINT` 借用结束再统一 `ShowWindow`（`PENDING_SHOW`），
  避免同步焦点消息重入父窗口的 `RefCell`；
- 新窗口按父窗口所在显示器的工作区居中，DPI 跟父窗口；不设 owner，任务栏各自一项；
- `WM_NCDESTROY` 把自己的表项摘掉，最后一个窗口销毁时才 `PostQuitMessage`；
- 窗口销毁时先把 `edits` / `retired` 宿主摘出来，离开借用后再析构；`text_edit` 的宿主注册表
  改成 `Weak`，不会自己把宿主续命到线程退出（原先会在进程退出时触发 TLS AccessError）。

## 窗口蒙版

窗口蒙版是独立窗口的一项属性，不是控件，也不进 `Document`：

- `WindowOptions::mask: Option<WindowMask>` 是初始值；`Context::set_mask(Option<WindowMask>)` 在运行时开关；
- `WindowMask { color }`：RGB 是蒙版颜色，`a` 是整体不透明度；`WindowMask::dim(0.35)` 是 35% 黑色；
- 实现是一块独立的顶层分层窗口（owner = 应用窗口），盖在客户区上方、所有 RichEdit 宿主上方；
  它不属于 `WINDOWS` 表，不影响布局、区域点击和焦点；
- 跟随客户区：`WM_SIZE` / `WM_MOVE` / `WM_DPICHANGED` 重新摆放；无边框窗口扣掉阴影白边，
  圆角用和窗口背景同一套 `tiny-skia` 几何生成预乘 RGBA 位图（`rounded_rect_pixmap`），
  经 `UpdateLayeredWindow` 逐像素贴上（最大化时按 0 圆角）；不用 `SetWindowRgn` 的硬区域，
  所以圆角抗锯齿边缘和背景对齐，不会漏出未覆盖的像素；
- 不参与输入：`WS_EX_TRANSPARENT | WS_EX_NOACTIVATE`，`WM_NCHITTEST` 回 `HTTRANSPARENT`，
  `WM_MOUSEACTIVATE` 回 `MA_NOACTIVATE`；鼠标落到下面的按钮 / 编辑框，键盘焦点不变；
- 蒙版在上面时不再抬 RichEdit 宿主（`set_bounds` 用 `SWP_NOZORDER`、跳过 `raise`），避免每帧
  z-order 来回切导致编辑框背景闪；连续重复的 `WM_MOUSEMOVE` 按坐标去重，避免贴图 / 重绘互相触发；
- 隐藏 / 最小化时蒙版和 RichEdit 宿主一起收起（`hide_owned`），恢复后下一帧再同步；
- 系统边框窗口只盖客户区，不盖系统标题栏；要连标题栏一起变暗就用 `Decorations::None` 自绘标题栏；
- 示例：demo 的「窗口蒙版」按钮；蒙版开着时仍能点击按钮、输入文字。

## 模态辅助窗口与蒙版的配合

辅助窗口的**模态**机制见 `docs/design/windows.md` §4.1：`Asset::set_modal(true/false)`，
同一所有者同时只允许一件模态辅助窗口。独立应用按下面的顺序配蒙版：

1. 先 `Context::set_mask(Some(WindowMask::dim(0.35)))` 给独立窗口蒙版；
2. 再创建 / 显示辅助窗口；
3. 辅助窗口出来后 `set_modal(true)`；
4. 辅助窗口销毁后 `set_modal(false)`（销毁会自动取消）并 `Context::set_mask(None)`。

蒙版管**视觉变暗**，模态管**挡住所有者输入**，两者不自动联动；只蒙版不模态、或者模态但自绘遮罩都可以。

Cloud UI 内置 **dialog**（`AuxiliaryWindowTemplate::dialog` / 脚本 `create_dialog`）走另一条路：
它创建即模态、自动压模态栈顶，`DialogOptions::mask = true` / `mask = true` 时自动向所有者要蒙版，
销毁前自动恢复原蒙版并让上一件模态恢复；调用方不需要手动 `set_modal` / 关蒙版。
详见 `docs/design/windows.md` §4.1.1。

对话框都是类：`MessageDialog::new(title, text, icon, buttons)` / `TextInputDialog::new(title, text, allow_multiline, default_text)`
建实例，先改 `style` 里的字体 / 颜色等属性，再 `show()`。`show` 在 `Application::update` / `view` 里同步阻塞、
自动禁用当前窗口：消息对话框返回按钮下标，文本输入对话框返回字符串。消息对话框的 `buttons` 1–4 个，
没有标题栏关闭按钮。

打开对话框时：

- 所有者名下的 RichEdit 宿主是独立顶层窗口，`EnableWindow` 管不到；宿主列表由窗口进入
  `current` 上下文时携带（`CurrentWindow.edit_hosts`），`dialog::BlockingPanel` 逐个
  `set_input_blocked(true)` 并放掉键盘焦点，关闭时恢复屏蔽前的焦点。
- 对话框面板可以拖动：对话框与所有者按相同位移移动，所有者保持模态（仍不响应鼠标 / 键盘）；
  编辑框宿主和窗口蒙版作为独立顶层窗口由对话框带着一起走（`TextHostWindow::move_by`）。
- 辅助窗口类在 `WM_SETCURSOR` 里恢复箭头光标，编辑框宿主的 I 型光标不会残留到面板上。
- `dialog` 的完整契约见 `docs/design/windows.md` §4.1.1。


## 通知区域（托盘）图标

`WindowOptions::tray: Option<TrayOptions>` 在窗口创建时注册图标，窗口销毁时摘掉：

- 图标用 `LoadImageW` 按 `SM_CXSMICON` 取 exe 内嵌的应用图标（`winresource` 写入的资源 ID 1），
  取不到退回 `IDI_APPLICATION`；`TrayOptions::tooltip` 为空时用窗口标题；
- `TrayOptions::new(tooltip).menu(vec![TrayMenuItem::new(id, label), …])` 配右键菜单，
  菜单项只有文字，没有分隔线 / 勾选 / 禁用状态；
- 左键单击 / 双击 / 右键 / 菜单项都以 `AppEvent::Tray(TrayEvent)` 交回应用，
  `TrayEvent::Menu(id)` 里的 `id` 就是 `TrayMenuItem::id`；
- 显示 / 隐藏窗口由应用自己决定：`Context::show()` / `Context::hide()`，和最小化 / 最大化一样
  先排队，窗口借用结束后再 `ShowWindow`；显示时会顺带还原最小化的窗口；
- 窗口隐藏或最小化时没有 `WM_PAINT`，托盘消息里会当场跑一遍 `Application::update`（只跑 update，
  不渲染），否则菜单里的「显示窗口 / 退出」永远不会生效；窗口可见时仍走正常的一帧；
- 主窗口隐藏 / 最小化 / 不在前台时它不能当前台窗口（或会被强行提到前面），`TrackPopupMenuEx` 会一弹就收；
  这时菜单改用一个离屏（-32000, -32000）的 1×1 `WS_POPUP | WS_EX_TOOLWINDOW` 窗口做 owner，
  弹完把前台还给原窗口；主窗口本来就是前台时仍用它做 owner；
- 监听 `TaskbarCreated` 广播，Explorer 重启后自动重新 `NIM_ADD`；
- 示例：`apps/cloud-ui-demo` 的托盘菜单「显示窗口 / 隐藏到托盘 / 退出」，左键 / 双击显示窗口。

## 独立应用打包（示例）

参考实现在 `apps/cloud-ui-demo/installer/`，与 `apps/windows/installer/`（输入法安装包）完全分开：

- `apps/cloud-ui-demo/build.rs`：`embed_manifest` 嵌 PerMonitorV2 DPI 感知 manifest，
  `winresource` 把 `resources/cloud-ui-demo.ico`（16–256 多尺寸）嵌进 exe；
- `installer/cloud-ui-demo.iss`：Inno Setup 7，`PrivilegesRequired=lowest`，
  按当前用户装到 `{autopf}\Cloud UI Demo`；开始菜单 + 可选桌面快捷方式；卸载删安装目录；
- `installer/build.ps1`：release 构建 → 铺 `target\installer\cloud-ui-demo\stage` → 调 ISCC；
  版本取 workspace 的 `cloud-ui-demo` 包版本，产物
  `target\installer\cloud-ui-demo\cloud-ui-demo-<版本>-windows-x86_64-setup.exe`；
- 程序不写注册表、不写用户数据，装完直接运行；示例包不随输入法安装包发布。

产品化时复制这一套，至少换 `AppId`（新 GUID，避免与示例互相覆盖）、`AppName` / exe 名 / 目录、
manifest 名、图标与数据文件清单；需要装 Program Files 或高权限时把 `PrivilegesRequired` 改成 `admin`。

## 还没做的

1. **更完整的控件层**：label / button / spacer / 列表盒 / 单行与多行文本编辑 / `scrollable_page` 与主题已进 `View`；
   通用 `SubpageContainer`、图片、列表虚拟化、滚动条拖动、按钮 hover / pressed、焦点链还没封装。
2. **普通窗口的完整语义**：系统边框与无边框自绘、自绘标题栏（最小化 / 最大化 / 还原 / 关闭）、
   多窗口、托盘图标与菜单、窗口蒙版、内置对话框（消息 / 文本输入）已有（蒙版只支持纯色半透明；
   模糊 / 渐变 / 输入拦截不做）；应用菜单、拖拽缩放与 Aero Snap 还没封装。
3. **文本编辑的边界**：单行 / 多行编辑已可输入（含 IME / 选区 / 剪贴板 / 内部滚动条），
   但显式 `focus_text` API、`Tab` 焦点切换还没接进 `View`。
4. **输入事件细节**：目前是「输入 -> 失效 -> 下一帧 update + view」，还没有事件队列预算、
   指针捕获（拖拽滚动条要用）。
5. **发布**：`apps/cloud-ui-demo` 已有独立安装包（`apps/cloud-ui-demo/installer/`，不混入输入法安装包）；
   真正产品化时再按模板换品牌、图标、数据清单与签名流程。

## 阶段

- 阶段 1（本次）：普通窗口宿主 + demo，证明 `cloud-ui` 能脱离 Server 跑。
- 阶段 2（本次）：拆出 `cloud-ui-types`；`cloud-ui` 默认不再依赖 `cloudime-platform`，
  IME 专用模块收到 `ime` 特性，`script` 连带打开 `ime`。
- 阶段 3（本次）：无 Lua 的 `View` / label / button / 行列 / 面板 / 区域点击；无边框窗口（阴影 + 圆角）
  与 `top_most` 属性。
- 阶段 4（本次）：`view::Theme`、列表盒、单行文本编辑（RichEdit 宿主从 `script_window` 抽成
  `text_edit` 模块，独立应用与脚本窗口共用）；补了布局核心「带 padding 容器给子节点减内边距」。
- 阶段 5（本次）：`scrollable_page`（可滚动页面，控件容器：`SelfDrawingPanel` 裁剪 + 区域裁剪 + 滚动条）、多行 `text_box`
  （`ensure_layout` + 内部滚动条回写）；渲染器的编辑框 placement 会合并裁剪面板的 clip。
- 阶段 6（本次）：自绘标题栏（`View::title_bar` + 四个 SVG 图标：减号 / 最大化 / 还原窗口 / 打叉）、`caption_height`
  拖动区、最小化 / 最大化 / 还原 / 关闭窗口命令；顺手修了 `UpdateLayeredWindow` 位置漂移。
- 阶段 7（本次）：多窗口——线程本地 `WINDOWS` 表、`Context::open_window` / `WindowId`、
  最后一个窗口关闭才退出消息循环；修了 `text_edit` 宿主表强引用导致的 TLS 退出崩溃。
- 阶段 8（本次）：独立应用打包——示例 manifest / 图标、Inno Setup 按用户安装与卸载、
  `build.ps1` 一键 release + stage + 出 setup；不混入输入法安装包。
- 阶段 9（本次）：通知区域图标——`WindowOptions::tray` / `TrayOptions` / `TrayMenuItem`，
  左键 / 双击 / 右键 / 菜单项经 `AppEvent::Tray` 回应用，`Context::show` / `hide`，
  隐藏 / 最小化时的 `pump` 与离屏菜单 owner，`TaskbarCreated` 重新注册。
- 阶段 10（本次）：窗口蒙版——`WindowMask` / `WindowOptions::mask` / `Context::set_mask`，
  独立顶层分层窗口覆盖客户区，点击穿透，跟随移动 / 缩放 / DPI，隐藏与最小化一起收起。

## 边界

- 只做 Windows；不引入 macOS / Linux 后端。
- 现有 IME 悬浮窗路径不变：Server 继续开 `script`，继续走 `IndependentWindowTemplate` /
  `ScriptWindows`。
- `cloud-ui` 不反向依赖 `apps/windows/server`。
