# 多行编辑框 `text_box`

2026-10-18 设计。目标是在已有单行编辑框 `text_edit` 的基础上，复用同一套 Windowless RichEdit、
脚本文本缓存和字体 / 颜色模型，增加多行显示、自动换行和内部滚动条。

本页先定结构和行为；实现落在 `cloudime-render` 的控件、`cloudime-script` 的解析 / 文本 API，
以及 Server `script_window` 的宿主窗和交互层。

## 组成与几何

```text
┌──────────────────────────────────────────────┐
│  背景形状（圆角 / 填充 / 边框，区域也是它）  │
│  ┌────────────────────────────────────────┐  │
│  │  RichEdit 文本区                       │█ │  ← 垂直滚动条（需要时）
│  │                                        │  │
│  │                                        │  │
│  └────────────────────────────────────────┘  │
│  ██████████████████████████████████████████  │  ← 水平滚动条（需要时）
└──────────────────────────────────────────────┘
```

- 控件尺寸是 `x` / `y` / `w` / `h`；背景形状覆盖整个控件，自动区域形状也由背景决定
  （圆角矩形 `radius`，缺省 `h/2` 的胶囊不再适用，`text_box` 使用普通圆角；缺省 6）。
- 文本区从背景内边距（`edge_width`）开始；有垂直滚动条时右侧扣掉滚动条厚度和小间隙，
  有水平滚动条时底部扣掉同样高度。
- 内部滚动条厚度固定为 **12 点**，与文本区之间留 **4 点**间隙；不写入脚本 API，
  后续如果需要再提升为属性。
- 文本超出文本区才显示对应滚动条；没有超出时滚动条不占用文本区，也不参与命中。
  这意味着尺寸计算需要一次 **两遍布局**：
  1. 先按“没有滚动条”的文本区让 RichEdit 排版，查询 `TxGetHScroll` / `TxGetVScroll`；
  2. 根据 `enabled` / `page < max - min` 决定哪条滚动条出现，缩文本区并重排；
  3. 滚动条显隐变化只重排一次；结果缓存到 `TextBox` 的运行时状态，下一帧不再反复抖动。

## 属性

与 `text_edit` 相同的属性：`x` / `y` / `w` / `h`、`read_only`、`back_color`、`edge_color`、
`edge_width`、`text_color`、`cursor_color`、`sel_back_color`、`sel_text_color`、
`font_name` / `font_size` / `font_flag`、`text`、`font`、`name`、`events`。

不同 / 新增：

| 属性 | 类型 | 缺省 | 说明 |
|---|---|---|---|
| `auto_line_wrap` | bool | `true` | 自动换行；启用时隐藏水平滚动条，RichEdit 打开 `TXTBIT_WORDWRAP` |
| `line_spacing` | f64 | `0` | 两行之间的额外间距（点）；映射到 RichEdit `PARAFORMAT` 的行距，0 = 用字体默认行高 |

**不提供**：

- `passkey`：多行编辑框不支持密码显示。
- 所有 SVG 属性（`svg` / `svg_path` / `svg_align` / `fill_color`）及由 SVG 带来的文本区宽度公式。
- `radius` 仍可写，但它是背景圆角，不参与文本区宽度公式。

颜色、字体标志、只读等语义与 `text_edit` 完全一致；旧字段名兼容规则也沿用。

## 方法、事件与文本 API

方法沿用单行编辑框，按控件名调用：

```lua
win:get_text("editor")
win:set_text("editor", "内容")
win:select_all("editor"[, false])
win:replace("editor", "旧", "新")
win:clear("editor")
```

事件沿用 `text_edit`：

- `text_changed`：`event.control` / `event.text`；
- `key_pressed`：`event.key` / `event.modifier`；
- `focused` / `unfocused`；
- 布局事件名继续接受 `onKeyPressed` / `on_key_pressed` 等别名。

内部水平 / 垂直滚动条**不暴露为脚本控件**，不派发 `value_changed`；滚动由用户鼠标操作，
脚本只看到文本变化 / 焦点 / 按键事件。

## RichEdit 模式

`text_box` 复用 `TextHostWindow`，增加一个多行模式：

- `TxGetPropertyBits` 增加 `TXTBIT_MULTILINE`；`auto_line_wrap` 为真时再加 `TXTBIT_WORDWRAP`。
- `text_edit` 的自绘单行 GDI 路径保持不变；`text_box` 使用 RichEdit 自己的多行排版 / 绘制
  （`TxDraw`），否则要重新实现换行、行高、滚动、软换行和选区布局，风险大于收益。
- 选中 / 普通文本颜色由 `TxGetSysColor` 提供（`COLOR_HIGHLIGHT` / `COLOR_HIGHLIGHTTEXT` /
  `COLOR_WINDOWTEXT` / `COLOR_WINDOW`）；宿主窗仍负责背景填充、边框和内部滚动条。
- 字号 / 字体 / 字体标志通过 `CHARFORMAT` 设置；`line_spacing` 通过 `PARAFORMAT2` 的
  `PFM_LINESPACING` / `dyLineSpacing` 设置（点 → twips）。
- 不创建 RichEdit 自己的滚动条：`TxGetScrollBars` 返回 0；滚动条由 `text_box` 绘制和命中。
- `TxGetHScroll` / `TxGetVScroll` 用于读取 `min/max/pos/page/enabled`，这是内部滚动条的
  唯一数据源；滚动位置通过 `EM_SETSCROLLPOS`（或 `EM_LINESCROLL`）回写 RichEdit。

## 鼠标交互

- 点击文本区：焦点交给 RichEdit，开始正常编辑 / 选区 / IME。
- 点滚动条滑块：拖动时按滚动范围的 `pos` 更新 RichEdit 滚动位置；滚动条滑块长度按
  `page / (max - min + page)` 计算，比单行滚动条的固定 1/4 更准确。
- 点滚动条空白轨道：按轨道位置翻一页（不是按单行滚动条的 `step`，因为内部滚动条没有
  `step` 属性）；滚轮在文本区 / 滚动条上都滚动文本。
- `auto_line_wrap = true` 时没有水平滚动条，文本区宽度固定；`false` 时长行产生水平滚动。
- 滚动条只在 `enabled` 或 `page < max - min` 时显示，显示后重新计算文本区尺寸。

## 宿主与数据流

- `cloudime-render` 新增 `TextBox` 控件和 `Element::TextBox`；它保存背景、文本、字体、颜色、
  `auto_line_wrap`、`line_spacing` 和运行时滚动条状态（`h_scroll` / `v_scroll`）。
- `cloudime-script` 的 `parse_text_box` 复用 `text_edit` 的属性解析，去掉 SVG / passkey，
  加 `auto_line_wrap` / `line_spacing`；文本缓存仍用 `(窗口编号, 控件名)`，`get_text` /
  `set_text` / `replace` / `clear` 共用现有实现。
- Server `ScriptWindow` 新增 `TextBoxWindow`，包含 `TextHostWindow`（多行模式）和滚动条命中 /
  拖动状态；文本区尺寸、滚动条显隐由宿主布局结果驱动。
- 滚动条由渲染器画在 `TextBox` 内，不进入 `Document.regions`；命中在 `ScriptWindow` 的
  `text_box` 区域里按几何判断，优先于文本区点击。
- `text_changed` / `key_pressed` / `focused` / `unfocused` 继续走现有宿主 → 自定义窗口消息 →
  `ScriptWindowEvent` → 运行时缓存的链路；`event.control` 是 text_box 的 `name`。

## 限额与失败处理

- 沿用内容限额：最多 4096 字符、512 个元素、8–2048 点尺寸。
- `line_spacing` 夹到 `0–64` 点；`auto_line_wrap` 缺省 true。
- 布局错误（非法尺寸 / 颜色 / 事件名）整份内容失败；RichEdit 初始化失败时退化为只显示背景，
  并写日志，不影响其他控件。
- 文本滚动位置在 `set_text` / `set_content` 后回到顶部；自动换行开关变化时重新排版并夹取滚动位置。

## 实现落点

| 位置 | 内容 |
|---|---|
| `crates/cloudime-render/src/control.rs` | `TextBox` 控件、内部滚动条几何、绘制 / 命中形状 |
| `crates/cloudime-render/src/document.rs` | `Element::TextBox` 与运行时滚动条状态 |
| `crates/cloudime-render/src/renderer/mod.rs` | 多行背景和滚动条绘制 |
| `crates/cloudime-script/src/window.rs` | `parse_text_box`、`Element::TextBox`、文本 API 缓存 |
| `apps/windows/server/src/ui/script_window/text_host.rs` | 多行 RichEdit 模式、`TxDraw`、滚动查询 / 回写、行距 |
| `apps/windows/server/src/ui/script_window/mod.rs` | `TextBoxWindow`、文本区 / 滚动条交互、焦点 / 事件 |
| `lua.md` / `docs/design/script.md` / `docs/notes/crate-notes.md` | 对外属性、方法、事件和实现说明 |

## 验收

1. 单行 `text_edit` 的所有行为逐字节不变。
2. `auto_line_wrap = true` 时长文本自动折行、无水平滚动条；关闭后出现水平滚动条。
3. 内容未超过可视区时不显示滚动条；超过后对应滚动条出现，文本区尺寸随之缩小。
4. `line_spacing` 改变后行距变化，滚动条范围 / 位置同步。
5. 选中、复制、粘贴、IME 组合、`text_changed`、`key_pressed`、`focused` / `unfocused` 与单行一致。
6. `get_text` / `set_text` / `select_all` / `replace` / `clear` 对 text_box 生效。
7. 鼠标拖动滑块、点轨道、滚轮都能滚动，且滚动位置不会和文本区点击错位。
