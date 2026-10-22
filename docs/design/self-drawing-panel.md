# 自绘面板（self_drawing_panel）

状态：设计定稿，待实现。自绘面板是独立控件类型，不是 `panel` 的扩展。

## 1. 定位

- `panel`：静态分组容器，只画背景 / 边框，子控件按页面树布局。
- `self_drawing_panel`：容器 + 内部 draw list + 局部命中区域。
  - `draw` 用 Render 的基础元素直接画界面，不参与布局；
  - `children` 是普通动态控件，参与页面树布局，画在 `draw` 上面；
  - `regions` 给 draw 内容声明局部点击 / 拖拽区域；
  - 默认裁剪在面板范围内。

它同时可用于：

- 页面树 v2 的 JSON 控件；
- 候选窗口这类由 Rust 直接构造 `Document` 的窗口；
- 后续作为候选窗口行、状态条等自绘界面的基础。

## 2. JSON 契约

```json
{
  "type": "self_drawing_panel",
  "name": "candidate",
  "w": 320,
  "h": 240,
  "auto_size": "draw",
  "clip": true,
  "draw": [
    { "name": "bg", "type": "rect", "x": 0, "y": 0, "w": 320, "h": 240,
      "radius": 8, "color": "#FFFFFFFF" },
    { "name": "preedit", "type": "text", "x": 12, "y": 8,
      "text": "ni'hao", "size": 14, "font": "pinyin", "color": "#FF222222" },
    { "name": "icon", "type": "svg", "x": 12, "y": 34, "size": 18,
      "path": "Images/SVG/打勾.svg", "tint": "#FF4A90E2" }
  ],
  "regions": [
    { "name": "row_0", "shape": "rect", "x": 0, "y": 30, "w": 320, "h": 28,
      "drag": false, "events": { "mouse_clicked": "choose" } }
  ],
  "children": []
}
```

### 2.1 draw

`draw` 只接受以下纯绘制元素，拒绝 `label` / `text_block` / 动态控件：

- `rect`
- `line`
- `circle`
- `text`
- `svg`
- `img`
- `path`

每项**必须**写非空 `name`，同一面板内唯一，且**大小写敏感**（`bg` 和 `BG` 是两个不同的元素）；
缺少 `name`、空 `name` 或完全相同的重名都会让整份布局失败。`name` 只作为 draw 元素标识，
不参与控件事件寻址；事件仍由 `regions` / 面板 `name` 提供。

坐标是面板左上角局部坐标：

- `(0, 0)` = 面板外框左上角；
- **不扣 padding**；padding 只影响 `children` 的 content box；
- `draw` 不产生 `event.element`，不单独绑定事件；命中由 `regions` 提供。

### 2.2 children

- 普通动态控件 / 容器，参与页面树布局；
- 位置由页面树与面板的 `layout` / `padding` 决定；
- 绘制顺序：`draw` 在下，`children` 在上；
- 第一版限制嵌套自绘面板深度，建议最多 4 层。

### 2.3 regions

`regions` 是面板局部命中区域，复用现有 `regions` 的字段：

- `name`：区域名；同一面板内唯一；
- `shape`：`rect` / `round_rect` / `circle` / `path`；
- `x` / `y` / `w` / `h` / `radius` / `cx` / `cy` / `commands` 等按 shape；
- `drag`：是否可拖动（缺省 `false`）；
- `events`：与动态控件相同的事件表。

事件上下文：

- `event.control = 面板的 name`；
- `event.region = 局部区域 name`；
- `event.control_path = 面板的页面树路径`；
- 多实例面板再叠加 `event.instance` / `item_index` / `item_key`。

面板本身也可以写 `events`：等价于覆盖整个面板的一个自动区域，区域名用面板名。

### 2.4 clip

- `clip` 缺省 `true`；`draw` 和 `children` 都裁在面板矩形内。
- `clip: false` 仅用于确实需要溢出的特殊场景。
- 裁剪只影响绘制与命中，不改变布局。

## 3. 尺寸

`w` / `h` 和 `auto_size` 的优先级：

1. 显式 `w` / `h` 优先；
2. 该轴没有显式值时，`auto_size = "draw"` 才按 `draw` 包围盒测量；
3. 没有显式值且没有 `auto_size`：整份布局失败并报错，不静默塌成 0。

`auto_size` 目前只接受 `"draw"`。

### 3.1 draw 包围盒

用 `LayoutMeasureHook` 量 `text` 的宽高；其他元素用几何范围：

- `rect`：`x..x+w` / `y..y+h`；
- `line`：两端点 + `thickness / 2`；
- `circle`：`cx ± radius` / `cy ± radius`；
- `text`：`x` / `y` 为左上角，宽高由量尺给；
- `svg`：`x..x+size` / `y..y+size`；
- `img`：`x..x+w` / `y..y+h`；`auto_size` 下 `img` 必须显式写 `w/h`，否则报错；
- `path`：按命令的几何包围盒。

面板尺寸 = draw 包围盒覆盖的最小外框；padding 不额外加在 draw 外框上（draw 是外框坐标）。没有量尺时退回现有字符估算，并记一次 warn。

## 4. 更新接口

`draw` 元素通过必需的 `name` 更新，name 比较大小写敏感；整份替换和单元素更新都保留：

| 接口 | 作用 |
|---|---|
| `win:set_draw_at(path, draw)` | 替换整份 draw list；路径为 v2 页面树路径时用这个 |
| `win:update_draw_at(path, name, element)` | 按 name 替换一个 draw 元素 |
| `win:insert_draw_at(path, index, element)` | 在指定下标插入一个 draw 元素 |
| `win:remove_draw_at(path, name)` | 删除一个 draw 元素 |
| `win:update_region_at(path, name, region)` | 更新一个局部区域 |
| `win:remove_region_at(path, name)` | 删除一个局部区域 |

v1 / 非页面树窗口使用对应的控件名版本（`win:update_draw(name, ...)` 等），内部仍解析到同一个自绘面板 payload。

单元素更新只重建 draw/region 数据和 Document，不重新测量布局；`auto_size` 面板改 draw 内容后必须重排一次。

## 5. 渲染模型

`cloudime-render` 增加 `Element::SelfDrawingPanel`：

```text
SelfDrawingPanel {
  x, y, width, height,
  clip: bool,
  draw: Vec<Element>,     // 只含纯绘制元素
  children: Vec<Element>, // 动态控件
}
```

- `draw_element` 在面板矩形处建立裁剪，先画 `draw`，再按原点偏移画 `children`，最后恢复裁剪。
- local regions 在 `Document.regions` 里展开成绝对区域；事件仍走现有 region → handler 链路。
- 自绘面板本身没有内置背景 / 边框；需要背景就用 draw 里的 `rect` 画。

## 6. 与页面树 / 候选窗口

- v2 页面树新增节点类型 `self_drawing_panel`，按 Container 参与布局；`draw` 不占布局空间。
- `auto_size` 在 `cloudime-script` 的页面树测量阶段调用 `LayoutMeasureHook`。
- 候选窗口迁移时：
  - 根节点用一个自绘面板；
  - 拼音行、候选行、高亮、页码、译文都进 draw；
  - 每一行的点击区域放 `regions`；
  - 动画每帧生成 draw 或单元素更新，不重建整个窗口；
  - 键位映射等资产仍按现有辅助窗口处理。

## 7. 实现顺序

1. `cloudime-render`：元素模型、draw/children 绘制、矩形裁剪、局部 regions 展开。
2. `cloudime-script`：JSON 解析、`auto_size` 测量、v2 节点、单元素更新接口。
3. `cloud-ui`：事件上下文 `event.region`、更新后的 Document 替换。
4. 测试：固定尺寸 / auto_size、裁剪、region 命中、单元素更新、候选行示例。
5. 候选窗口迁移。

## 8. 兼容性

- `panel` 的语义和行为不变。
- `draw` 里不允许放动态控件；要动态控件就放 `children`。
- 旧 v1 / v2 布局不受影响；不写 `self_drawing_panel` 就不会走新路径。
