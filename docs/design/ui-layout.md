# 前台 UI 布局文件（页面树 v2）

2026-10-18 初版（v1：绝对坐标）；2026-10-21 页面树 v2 设计定稿，**待实现**。
当前发布版本仍按 v1 解析；v2 落地前，`lua.md` 与实现以 v1 为准。

本页定义窗口内容的页面树、布局算法与 JSON 契约。元素的画法、事件、窗口模板见
`script.md` / `windows.md`；布局引擎放在 `cloud-ui`，`cloudime-render` 仍只负责把
最终几何画成位图。

## 1. 目标与边界

- 一个窗口只有**一棵主 Page 树**；后续多页面 / Tab 用子页面嵌套与显隐表达。
- 窗口内容是一棵树，不是平铺元素表：节点负责结构、布局、尺寸与数据，渲染器只拿
  布局后的绝对几何。
- 布局是 `Page` / `Stem` / `Container` 的**属性**，不是独立节点。
- JSON 是初始内容；运行时仍需支持按节点更新、增删和重新布局。
- 不引入第二套控件字段：布局文件里的控件字段与 `set_content` 元素表保持一致。

## 2. 节点模型

```text
Page（主页面，一窗口一棵）
└─ layout = flex(row)（缺省）
   ├─ Stem（无外观，只分组 / 布局）
   │  └─ layout = linear（强制 row）
   │     ├─ Control
   │     └─ Container（panel）
   │        └─ layout = grid
   │           ├─ Control
   │           └─ Subpage
   └─ Subpage（可重复实例）
```

| 节点 | 职责 | 是否有外观 | 是否有 layout |
|---|---|---|---|
| `Page` | 主页面根，拥有窗口内容区 | 可有根背景 | 有，缺省 `flex` |
| `Stem` | 抽象分组，只负责结构和布局 | **没有**，不绘制、不参与视觉样式 | 有，缺省 `flex` |
| `Container` | 具体控件容器，例如 `panel` | 有背景 / 边框 / 圆角 / padding | 有，缺省 `flex` |
| `Control` | 叶子控件，例如 `label` / `flat_svg_button` / `text_edit` | 有 | 没有 |
| `Subpage` | 引用另一份 Page 布局，可重复实例化 | 由子页面根决定 | 子页面内部有；父级只看到一个外框 |

- `Stem` 可以被命名（供脚本寻址），但没有 `fill` / `border` / `radius` 等视觉字段。
- `Container` 的 `children` 直接挂在它下面，不需要再套一层 `layout` 节点。
- 一个网格单元格建议只放一个节点；需要多个节点时用 `Stem` 或 `Container` 包起来。
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
| `type` | `stem` / `subpage` / 控件类型（`panel`、`label` 等） |
| `name` | 控件 / 分组名；同一作用域唯一 |
| `layout` | 仅 Page / Stem / Container；布局对象 |
| `padding` | 宿主内容区内边距 |
| `position` | `absolute` 时脱离布局流 |
| `w` / `h` | 尺寸；沿用现有 v1 字段名 |
| `children` | Stem / Container 的子节点 |
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

- `src` 相对当前布局文件解析，再按现有布局路径搜索链查找。
- `repeat` 指向数据上下文中的数组；省略时只创建一个实例。
- `key` 是稳定标识；省略时用下标，但列表增删建议必须写。
- 子页面文件也按自己的 `version` 解析；未知版本整份布局失败。

### 5.3 必填与可空

- **必填字段缺失**：整份布局失效；记 `error` 日志，窗口保持原样。
- **可空字段缺失**：按缺省值 / 空串处理；记 `warn` 日志（同一字段每次加载只记一次，避免刷屏）。
- 必填示例：节点 `type`、控件 `name`、子页面 `src`、`layout.type`（写了 `layout` 时）。
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
- 数据更新通过窗口 API 写回，按 key 做增删改，模板只解析一次。

### 6.2 声明式绑定 `{{...}}`

只允许用在展示字段：

```json
{ "type": "label", "name": "title", "text": "{{item.title}}" }
{ "type": "static_image", "name": "icon", "img": { "path": "{{item.icon}}" } }
```

- 支持简单的属性路径，不支持函数调用、算术、条件表达式。
- 不允许用 `{{...}}` 改 `type`、`layout`、`children`、`position` 等结构字段。
- 不支持嵌套 `{{}}`。
- 必填绑定值缺失：整份布局失效并记 `error`；可选字段缺失按空串并记 `warn`。
- 路径字段（图片 / SVG）仍受现有路径搜索和安全限制。

### 6.3 实例路径与事件

- 实例路径形如 `rows[0]`、`rows[1]`；控件完整路径为 `rows[0].title`。
- 控件名在实例内局部唯一，跨实例允许重名。
- 事件载荷增加：
  - `subpage`：子页面名；
  - `instance`：实例路径；
  - `item_index`；
  - `item_key`；
  - `control`：局部控件名；
  - `control_path`：完整路径。
- 一个窗口仍然只有一份 Lua 脚本；子页面不单独运行脚本，事件冒泡到窗口脚本。
- `win:update_content` 等按控件名寻址的 API 需要支持完整路径或“实例 + 局部名”。

### 6.4 生命周期与限制

- 子页面模板按文件路径缓存；文件变化不自动监听，`load_layout` 整份重载。
- 循环引用、总实例数、总节点数、最大深度都要有限制；沿用并扩展现有 4 MB / 512 元素 / 深度 8 的预算。
- 子页面不可见（`visible: false`）时不占位，父布局按不存在处理。
- 列表项过多时的虚拟化 / 截断策略另行定义；第一版先给 `max_instances`。

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

- 页面树、布局算法、子页面实例化放在 `cloud-ui`。
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
- 路径字段（`svg_path` / `img.path`）先按布局文件所在目录找，再逐级向上找；放在 `Scripts/lib/`
  的布局可以直接引用安装根目录的 `Images/...`。
- 文件上限 4 MB；控件数量、文字长度、图片像素等沿用 `script.md` 的内容限额。
- 动态控件（`flat_svg_button` / `text_edit` / `text_box` / `hscroll_bar` / `vscroll_bar`）和区域条目
  可以带 `events`；键是窗口动作名，值是由脚本分发的处理器名。未知动作让整份布局失败。
- 静态元素（`rect` / `label` 等）不能直接写 `events`；要接事件请加 `regions`。
- 脚本入口：

  ```lua
  local win = cloudime.window.create{ name = "面板", layout = "Scripts/lib/main.json" }
  win:load_layout("Scripts/lib/main.json")
  ```

- `load_layout` 与 `set_content` 一样只排队一个请求；失败时本次调用报 Lua 错误，窗口保持原样。
- 布局文件不监听变化；改文件后重新调用或重启 Server。
- 辅助窗口同样支持 `create_auxiliary{ layout = … }` / `aux:load_layout(...)`。
- 布局文件中的窗口级选项（`interactive` / `shadow` / `background`）仍在 Lua 创建时给。
- v1 不做 `include` / 变量替换 / 自动布局约束。
