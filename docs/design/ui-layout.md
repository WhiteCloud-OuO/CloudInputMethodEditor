# 前台 UI 布局文件（页面树 v2）

2026-10-18 初版（v1：绝对坐标）；2026-10-21 页面树 v2 设计定稿，v2 解析、基础布局、
子页面 `repeat` / `key` 与 `{{item.xxx}}` 声明式绑定已经接入。当前发布版本仍默认按 v1 解析；
`lua.md` 以 v1 为准，v2 的脚本 API 待页面树稳定后统一补文档。

本页定义窗口内容的页面树、布局算法与 JSON 契约。元素的画法、事件、窗口模板见
`script.md` / `windows.md`；页面树与布局引擎放在 `cloud-ui-core`，`cloud-ui` 负责 Windows
窗口壳，`cloudime-render` 仍只负责把最终几何画成位图。

## 1. 目标与边界

- 一个窗口只有**一棵主 Page 树**；后续多页面 / Tab 用子页面嵌套与显隐表达。
- 窗口内容是一棵树，不是平铺元素表：节点负责结构、布局、尺寸与数据，渲染器只拿
  布局后的绝对几何。
- 布局是 `Page` / `Stem` / `ControlsContainer` / `SubpageContainer` 的**属性**，不是独立节点。
- JSON 是初始内容；运行时仍需支持按节点更新、增删和重新布局。
- 不引入第二套控件字段：布局文件里的控件字段与 `set_content` 元素表保持一致。

## 2. 节点模型

```text
Page（主页面，一窗口一棵）
└─ layout = flex(row)（缺省）
   ├─ Stem（无外观，只分组 / 布局）
   │  └─ layout = linear（强制 row）
   │     ├─ Control
   │     └─ ControlsContainer（panel）
   │        └─ layout = grid
   │           ├─ Control
   │           └─ SubpageContainer
   │              └─ Subpage
   └─ SubpageContainer
      └─ Subpage（可重复实例）
```

| 节点 | 职责 | 是否有外观 | 是否有 layout |
|---|---|---|---|
| `Page` | 主页面根，拥有窗口内容区 | 可有根背景 | 有，缺省 `flex` |
| `Stem` | 抽象分组，只负责结构和布局 | **没有**，不绘制、不参与视觉样式 | 有，缺省 `flex` |
| `ControlsContainer` | 控件容器，例如 `panel` | 有背景 / 边框 / 圆角 / padding | 有，缺省 `flex` |
| `SubpageContainer` | 子页面容器（后续实现），只放 `subpage` | 无自己的视觉外观 | 有，缺省 `flex` |
| `Control` | 叶子控件，例如 `label` / `flat_svg_button` / `text_edit` | 有 | 没有 |
| `Subpage` | 引用另一份 Page 布局，可重复实例化 | 由子页面根决定 | 子页面内部有；父级只看到一个外框 |

- `self_drawing_panel` **不是容器**，是带内部 draw list 的特殊控件；它的 `children` 是控件内部组合内容，不套用容器子节点矩阵。
- `Stem` 可以被命名（供脚本寻址），但没有 `fill` / `border` / `radius` 等视觉字段。
- 容器的 `children` 直接挂在它下面，不需要再套一层 `layout` 节点。
- 一个网格单元格建议只放一个节点；需要多个节点时用 `Stem` 或控件容器包起来。
- 页面树节点的绘制顺序与命中顺序按树顺序；绝对定位节点仍在树里，按树顺序参与叠放。
- 节点名称在同一个作用域内唯一；子页面实例有独立作用域，完整路径由实例路径拼接。

## 3. 布局

所有布局共用一组基础属性：

```json
{
  "layout": {
    "type": "flex",
    "direction": "row",
    "gap": 8
  },
  "padding": 12
}
```

- `layout.type` 缺省 `flex`；`direction` 缺省 `row`。
- `padding` 属于宿主 content box；`gap` 只作用于相邻子节点，不加在最外侧。
- `margin` 只允许在 `flex` 和绝对定位子节点上使用；`linear`、`grid` 不支持子节点 margin。
- 不做 margin collapse；flex 子节点的 margin 与 `gap` 直接相加。
- 子节点尺寸、绝对定位、溢出规则见第 4 节。

### 3.1 线性布局 `linear`

- 强制 `direction = row`，**没有 column 写法**；纵向排列用 `flex` 的 `direction = column`。
- 单方向、不换行、不拉伸、不做交叉轴对齐。
- 只支持统一 `gap`，不支持子节点单独 `margin` / `align_self`。
- 从宿主 content box 的左上角开始排列。
- 子节点保持自己的尺寸约束；超出宿主时按宿主的 overflow 规则处理。

### 3.2 弹性布局 `flex`（缺省）

- `direction`: `row` / `column`，缺省 `row`。
- `wrap`: `nowrap` / `wrap`，缺省 `nowrap`。
- `justify`: `start` / `center` / `end` / `space-between` / `space-around` / `space-evenly`。
- `align`: `start` / `center` / `end` / `stretch`。
- `gap`、子节点 `margin`、`grow` / `shrink` / `basis`、`align_self`。
- 交叉轴默认 `stretch` 还是不拉伸，由第 4 节的尺寸规则决定；实现时必须固定一个缺省。

### 3.3 网格布局 `grid`

- 仍是“轨道 + 单元格”模型，不改用流式。
- 列 / 行支持固定值、百分比、`fr`、`auto`、`minmax`。
- 子节点可指定 `row` / `column` / `row_span` / `column_span`；未指定时自动流。
- 支持 `gap` 和宿主 `padding`；**不允许子节点 margin**。
- 一个单元格建议只放一个节点；多个节点用 `Stem` / `Container` 包成一组。
- 自动行高、`auto` 轨道和内容测量的具体算法在实现时按第 4 节的两阶段测量确定。

### 3.4 响应式布局 `responsive`

采用可换行的流式方案：

- 单方向排列，空间不够自动换行。
- 缺省 `direction = row`；方向、换行和断点规则在实现时固定。
- 子节点支持固定宽度、百分比宽度、`auto`、`min_width` / `max_width`。
- 行高由该行最高子节点决定。
- 可配置断点；断点只改变布局参数（方向、列数 / 子项宽度），不改变页面树结构。
- 不自动缩放字体；字号仍由主题 / 控件属性决定。
- `gap` 和宿主 `padding` 可用；子节点 margin 是否开放待实现细化，默认不建议开放。

### 3.5 绝对定位

绝对定位是技术性定位模式，不是第五种页面布局：

```json
{
  "type": "flat_svg_button",
  "name": "close",
  "position": "absolute",
  "inset": { "top": 8, "right": 8 },
  "w": 24,
  "h": 24
}
```

- 绝对定位节点脱离正常布局流，不参与父布局的尺寸分配。
- 支持 `left` / `top` / `right` / `bottom` / `inset`，相对父节点 padding box。
- `position` 不是 `absolute` 时，`x` / `y` 在 v2 中不允许作为普通布局坐标使用。
- 绝对定位节点按树顺序叠放；后续如需显式层级再加 `z_index`。
- 用于关闭按钮、角标、遮罩、浮层等场景。

## 4. 尺寸与测量（方案 A）

### 4.1 盒子模型

- `box-sizing: border-box`：`w` / `h` 包含 padding 与 border。
- `padding` 只属于宿主；子节点用 `margin`（仅 flex / absolute）或父布局的 `gap`。
- `linear` / `grid` 不支持子节点 margin。

### 4.2 尺寸来源

节点的 `w` / `h` 支持：

- 固定值；
- 百分比：相对父节点 content box；
- `auto`：由内容 / 子节点测量；
- flex 的 `grow` / `shrink` / `basis`；
- grid 轨道的 `fr` / `minmax`；
- `min_width` / `max_width` / `min_height` / `max_height`；
- `aspect_ratio`。

百分比只在父轴尺寸确定时生效；父轴是 `auto` 时按 `auto` 处理并记日志，避免循环依赖。

### 4.3 两阶段

1. **measure**：自底向上量节点固有尺寸；文字 / 图片 / 子页面都参与。
2. **arrange**：自顶向下分配最终位置和尺寸。

需要换行的布局（`responsive` / `flex wrap`）允许在 arrange 阶段再次测量行高。

### 4.4 根页面与窗口

- `page.w` / `page.h` 可省：
  - `create{ layout = ... }` 使用创建参数；
  - `win:load_layout` 使用句柄当前尺寸；
  - 文件里写了就覆盖。
- 根页面可以固定尺寸，也可以一个轴 `auto` 由内容撑开；窗口最小 / 最大尺寸由顶层约束限制。
- 窗口内容尺寸变化时重新 measure / arrange；不自动改字号。
- 子页面根节点的外框由父布局分配；内部再按子页面自己的 root layout 排。

### 4.5 溢出与裁剪

- 每个宿主需要明确 `overflow`：默认裁切还是允许溢出，实现时统一。
- 需要滚动的容器使用独立滚动属性，不复用 `text_box` 的内部滚动条。
- 绝对定位节点不改变父级的 overflow 判断，但仍受父级裁切影响。

## 5. v2 JSON 契约

### 5.1 顶层

```json
{
  "version": 2,
  "page": {
    "w": 500,
    "h": 360,
    "layout": { "type": "flex", "direction": "row", "gap": 8 },
    "padding": 12,
    "children": []
  },
  "data": {}
}
```

- `version: 2` 表示页面树格式。
- `page` 是主 Page；一个窗口一份。
- `data` 是子页面 `repeat` / `{{...}}` 的数据上下文，可由 Lua 创建时或运行时提供。
- 顶层直接写控件数组、或写 `controls` / `elements`，属于 v1 兼容语法，见第 7 节。

### 5.2 节点字段

| 字段 | 说明 |
|---|---|
| `type` | `stem` / `subpage` / `subpage_container` / 控件类型（`panel`、`label` 等） |
| `name` | 控件 / 分组 / 容器名；同一作用域唯一 |
| `layout` | 仅 Page / Stem / ControlsContainer / SubpageContainer；布局对象 |
| `padding` | 宿主内容区内边距 |
| `position` | `absolute` 时脱离布局流 |
| `w` / `h` | 尺寸；沿用现有 v1 字段名 |
| `children` | Stem / ControlsContainer / SubpageContainer 的子节点 |
| `visible` | `false` 时不占位（类似 `display: none`） |
| `events` / `regions` | 沿用现有控件 / 区域语义 |

子页面节点：

```json
{
  "type": "subpage",
  "name": "rows",
  "src": "list-row.json",
  "repeat": "items",
  "key": "id",
  "max_instances": 100
}
```

- `subpage` 必须放在 `SubpageContainer.children` 里；容器负责外框、排列和裁剪。
- `src` 相对当前布局文件解析，再按现有布局路径搜索链查找。
- `repeat` 指向数据上下文中的数组；省略时只创建一个实例。
- `key` 是稳定标识；省略时用下标，但列表增删建议必须写。
- 子页面文件也按自己的 `version` 解析；未知版本整份布局失败。

### 5.3 必填与可空

- **必填字段缺失**：整份布局失效；记 `error` 日志，窗口保持原样。
- **可空字段缺失**：按缺省值 / 空串处理；记 `warn` 日志（同一字段每次加载只记一次，避免刷屏）。
- 必填示例：节点 `type`、控件 `name`、子页面 `src`、`layout.type`（写了 `layout` 时）。
- `subpage.required` 是数据项必填字段数组（相对 `item`）；任一项缺失整份布局失败。
- 可空示例：`padding`、`gap`、`visible`、`repeat`、`key`、`w` / `h`。
- 子页面缺字段同样遵守：必填失败整份布局失效，可空按缺省并记日志。

## 6. 子页面实例化（方案 B + B2）

子页面是**模板 + 可重复实例 + 数据上下文**，不是简单 include。

### 6.1 数据上下文

- 窗口有一份页面数据，例如 `data.items`。
- `repeat` 指向其中的数组；每个 item 创建实例。
- 实例内可用：
  - `item`：当前数组项；
  - `index`：从 0 开始的下标；
  - `key`：稳定标识；
  - `params`：实例级参数（后续可加）。
- 数据更新通过 `win:set_data(table)` 写回，按 key 做增删改，模板只解析一次。
- `create{ layout = path, data = table }` 和 `win:load_layout(path, data)` 也可以在首次加载时带数据；
  后续 `win:set_data` 只重跑最近一次布局，不重新读文件。

### 6.2 声明式绑定 `{{...}}`

只允许用在展示字段：

```json
{ "type": "label", "name": "title", "text": "{{item.title}}" }
{ "type": "static_image", "name": "icon", "img": { "path": "{{item.icon}}" } }
```

- 支持简单的属性路径，不支持函数调用、算术、条件表达式；状态选择用 6.2.1 的状态值表。
- 不允许用 `{{...}}` 改 `type`、`layout`、`children`、`position` 等结构字段。
- 不支持嵌套 `{{}}`。
- 必填绑定值缺失：整份布局失效并记 `error`；可选字段缺失按空串并记 `warn`。
- 路径字段（图片 / SVG）仍受现有路径搜索和安全限制。

### 6.2.1 状态值 `state`

展示字段的值可以写成状态表，由当前实例状态选择：

```json
"color": {
  "state": {
    "0": "#FF222222",
    "1": "#FFFF0000",
    "2": "#FF0000FF",
    "default": "#FF222222"
  }
}
```

- 键是状态值（`0` / `1` / `2`），`default` 是找不到对应键时的兜底；两者都没有时为空。
- `list_box` 行模板提供 `item_state`、`selected` / `focused`、`hovered`；其他实例缺省按 `0`。
- 状态表只用于展示字段；展开后各状态共用同一份测量结果，不能靠它改变行高、行宽或结构。
- 状态表可以嵌在数组 / 对象里，例如 `runs[].color` 或 `img` 的展示属性。

### 6.3 实例路径与事件

- 实例路径形如 `rows[0]`、`rows[1]`；控件完整路径为 `rows[0].title`。
- 路径 API 同时接受实例下标与实例 key：`rows[1].title`、`rows[abc].title`、`rows["abc"].title`
  指向同一个节点；数字 key 与下标无法区分，只保留下标写法。
- 控件名在实例内局部唯一，跨实例允许重名。
- 运行时保留页面树与节点路径索引（`PageLayout`），改可见性 / 数据后重新测量、展开，不重读文件。
- 事件载荷：
  - `subpage`：子页面名；
  - `instance`：实例路径；
  - `item_index`；
  - `item_key`；
  - `control`：局部控件名；
  - `control_path`：完整路径。
- 窗口事件载荷已带上 `subpage` / `instance` / `item_index` / `item_key` / `control_path`；
  控件内部仍用实例前缀名（例如 `rows_0_title`）保证布局内唯一，但 `event.control` 给**局部名**
  （例如 `title`），完整路径用 `event.control_path`。
- 窗口本身仍由一个主 Lua 脚本控制；子页面可通过 `script` 使用 `Scripts/shared/` 下的实例脚本
  （见 6.7），实例事件先派给子页面脚本，再交给主窗口脚本。
- 已提供按路径寻址的 `win:set_visible(path, visible)`、`win:get_text_at` / `win:set_text_at` /
  `win:update_content_at`；旧的按控件名 API 保留给 v1 和非子页面控件。
- 状态条与状态切换提示的几何已改为页面树 → `Document` → `render_document`；候选窗已改为
  `CandidateView` → `PageTree`（拼音行 / 候选行或格 / 底部信息）→ `Document`。协议层 `Frame` 仍用于
  Core → Server 的 IPC，但不再进入候选渲染器。候选行同时生成 `Rendered.regions`（`candidate_<index>`），
  鼠标命中已优先走区域。旧 `Frame` 渲染只保留给设置页预览和逐像素对照。
- 节点级 `insert` / `remove` 和列表 keyed diff 尚未接入；当前用 `set_data` 整份重建实例。

### 6.4 文字测量

- `auto` 文本通过 Server 装入的 `LayoutMeasureHook` 量：`text` / 字号 / 行距 / 字体槽 → `(宽, 高)`。
- 量尺与候选窗共用同一套渲染器、字库和整形管线（`SharedMeasurer`），不是按字符数估算；
  量尺不可用时才退回旧的字符宽度估算。
- 多行高度 = 行高 × 行数 + 行距 × (行数 − 1)。
- 测量单位是点；窗口渲染仍按 DPI / 缩放整体换算。

### 6.5 虚拟化

```json
{
  "type": "subpage", "name": "rows", "src": "row.json",
  "repeat": "items", "key": "id",
  "virtualize": {
    "axis": "y",
    "viewport": 300,
    "item_height": "auto",
    "overscan": 1
  }
}
```

- `axis`：`y`（默认，纵向）/ `x`（横向）。
- `viewport`：可见区高度 / 宽度（点）；省略时用模板 `page` 的固定 `h` / `w`。
- `item_height`：数字 = 固定步长；省略或 `"auto"` = 用 `LayoutMeasureHook` 逐项量实际高度（变高行）。
- `overscan`：可见区外额外创建几项，缺省 1。
- 只把可见窗口的实例挂到页面上；前面的项用一个固定高度的 spacer 占位，窗口边界由窗口自身裁剪。
- 滚动由脚本 `win:set_scroll(path, offset)` 驱动；`set_data` 保留当前偏移。
- 虚拟化时忽略 `max_instances`。
- keyed identity：有字符串 `key` 的实例内部控件名前缀用 key（如 `rows_a_title`），重排后名字不变；
  `cloud-ui` 更新内容时按控件名复用同名编辑框宿主，焦点 / 选区 / 输入中的文本不重建。
- `set_data` 数据完全相同时直接跳过；变化时重建当前可见窗口。自动触底加载仍需脚本在 `wheel`
  事件里判断后 `set_data`。
- `list_box` 复用这套虚拟化：页树只挂可见行 + overscan，滚动时通过 `SetListWindow` 原地替换
  `ListBox.rows`，不再重建整份 `Document`。固定行高直接算偏移；`auto` 行高用按行缓存的高度，
  未测量行先按估算高度定位，滚动到附近时再测量并修正总高度。

### 6.6 子页面实例脚本

子页面模板的 `page`（或调用方的 `subpage` 节点）可以写：

```json
{ "script": "list-row.lua" }
```

脚本固定在安装目录的 `Scripts/shared/` 下解析。文件返回一个表：

```lua
local page = {}
function page.created(ctx) end          -- 每个实例创建时一次
function page.event(ctx, event) end     -- 实例内控件事件
function page.destroyed(ctx) end        -- 实例滚出窗口 / 数据删除
return page
```

`ctx` 提供 `window`（窗口句柄，可用 `win:set_text_at` / `update_content_at` 等按路径 API）、
`subpage`、`instance`、`index`、`key`、`item` 和 `path(name)`。同一窗口、同一实例只调用一次
`created`；列表虚拟化滚出窗口的实例调用 `destroyed`。`event` 先于主窗口脚本执行，返回 `true`
时吞掉该事件；返回其它值继续交给主窗口脚本。脚本出错只记日志，不中断窗口。

### 6.7 生命周期与限制

- 子页面模板按文件路径缓存；文件变化不自动监听，`load_layout` 整份重载。
- 循环引用、总实例数、总节点数、最大深度都要有限制；沿用并扩展现有 4 MB / 512 元素 / 深度 8 的预算。
- 子页面不可见（`visible: false`）时不占位，父布局按不存在处理。
- 未开 `virtualize` 时列表项仍受 `max_instances` 限制；开了虚拟化后只挂可见窗口，但 `set_data`
  仍整份重建可见实例，keyed diff 与自动触底加载待接入。

## 7. v1 兼容

- 没有 `version` 或 `version: 1`：按现有绝对坐标格式解析，行为不变。
- 顶层 `controls` / `elements` / 控件数组、`panel.children`、`x` / `y` / `w` / `h` 均保持原语义。
- v1 内部适配成 `Page(layout = absolute)`，让上层统一走页面树。
- `version: 2`：
  - 普通节点不能用 `x` / `y` 做布局坐标；
  - 绝对定位必须显式 `position: "absolute"`；
  - 未声明 `version` 的文件不会因为出现 `layout` 字段自动升级，避免误判。
- 未知 `version`、v2 必填字段缺失、布局类型未知：整份布局失效并记 `error`。
- 旧文件不需要迁移；后续可提供迁移工具，把 v1 转成 v2 的绝对定位页面树。

## 8. 引擎边界

- 页面树、布局算法、子页面实例化放在 `cloud-ui-core`。
- `cloud-ui` 只负责 Windows 窗口壳、分层贴图、命中测试和文本宿主。
- `cloudime-script` 负责读取 JSON、校验字段、把 Lua 数据变成页面树的数据上下文。
- `cloudime-render` 只接收布局后的几何和 `Document`，不实现 flex / grid。
- 布局在工人线程完成后生成新的 `Document`，UI 线程只负责贴图、命中测试和文本宿主。
- 绝对定位、子页面、数据变化都必须按树顺序更新绘制顺序和命中顺序。

## 9. 待实现时固定

以下不是设计分歧，但实现前必须落成常数 / 规则：

- `flex` 交叉轴缺省是否 `stretch`。
- `responsive` 的断点字段与换行后的列数规则。
- `grid` 轨道语法的具体字符串 / 对象写法。
- `overflow` 的缺省值。
- 子页面路径搜索顺序与 v1 的兼容细节。
- 布局失败时的日志字段与错误码。

## 附录 A：v1 当前契约（仍生效）

v2 实现之前，`layout_file.rs` 仍按这一节解析。

- 顶层是对象（也可以直接是控件数组）；`controls` 与 `elements` 等价，根数组等价于只有控件、
  尺寸用窗口缺省。
- 每个控件都必须写 `name`：英文字母开头，后续只允许英文字母 / 数字 / 下划线，最长 255；
  同一份布局里区分大小写唯一；不能是 Rust 关键字。
- `width` / `height` 可省：`win:load_layout` 用句柄上的尺寸，`create{ layout = … }` 用创建参数；
  文件里写了就覆盖。
- `regions` 与 `set_content` 的 `regions` 同义；`drag` 缺省 `true`。
- 路径字段（`svg_path` / `img.path`）先按布局文件所在目录找，再逐级向上找；放在 `Scripts/layouts/`
  的布局可以直接引用安装根目录的 `Images/...`。
- 文件上限 4 MB；控件数量、文字长度、图片像素等沿用 `script.md` 的内容限额。
- 动态控件（`flat_svg_button` / `text_edit` / `text_box` / `hscroll_bar` / `vscroll_bar`）和区域条目
  可以带 `events`；键是窗口动作名，值是由脚本分发的处理器名。未知动作让整份布局失败。
- 静态元素（`rect` / `label` 等）不能直接写 `events`；要接事件请加 `regions`。
- 脚本入口：

  ```lua
  local win = cloudime.window.create{ name = "面板", layout = cloudime.layout_path("main.json") }
  win:load_layout(cloudime.layout_path("main.json"))
  ```

- `load_layout` 与 `set_content` 一样只排队一个请求；失败时本次调用报 Lua 错误，窗口保持原样。
- 布局文件不监听变化；改文件后重新调用或重启 Server。
- 辅助窗口同样支持 `create_auxiliary{ layout = … }` / `aux:load_layout(...)`。
- 布局文件中的窗口级选项（`interactive` / `shadow` / `background`）仍在 Lua 创建时给。
- v1 不做 `include` / 变量替换 / 自动布局约束。
