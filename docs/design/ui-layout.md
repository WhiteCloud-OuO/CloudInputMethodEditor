# 前台 UI 布局文件

2026-10-18 定。脚本窗口的内容除了写在 Lua 里，也可以写成一份 **JSON 布局文件**（前台 UI 文件）；
本页只定文件契约与解析位置，元素的画法与窗口模板仍见 `script.md` / `windows.md`。

## 目标与边界

- 让脚本作者把控件布局从 Lua 里拆出来：用 JSON 描述尺寸、控件、区域和事件响应，Lua 只负责
  生命周期与业务分支。
- **默认规则（2026-10-18）**：凡是创建脚本窗口（独立窗口 / 辅助窗口），除非确实需要运行时
  动态生成大量控件、或整份布局频繁变化，否则一律使用 JSON 布局文件；
  `cloudime.window.create{ layout = … }` / `win:load_layout(…)` 是默认入口，`set_content` 是例外。
- JSON 只是**另一种内容来源**，解析后仍得到同一个 `cloudime_render::Document`：
  工人线程读文件、解析、生成元素与区域；UI 线程照旧经 `Renderer::render_document` 出图。
  渲染器不关心内容来自 Lua 还是 JSON，也不在 UI 线程做文件 IO。
- 不引入第二套控件字段：JSON 的字段与 `set_content` 元素表一一对应，非必填项省略 / `null`
  时用同一套缺省值。这样布局、Lua、文档不会各自漂移。

## 文件格式

顶层是一个对象（也可以直接是控件数组）：

```json
{
  "width": 500,
  "height": 360,
  "controls": [
    { "name": "background", "type": "rect", "x": 0, "y": 0, "w": 500, "h": 360,
      "radius": 12, "color": "#FFFFFFFF" },
    { "name": "close", "type": "flat_svg_button", "x": 468, "y": 8,
      "w": 24, "h": 24, "svg_path": "Images/SVG/打叉.svg",
      "shape": "round_rect", "radius": 4,
      "normal": { "background": "#00000000", "svg": "#FF000000" },
      "events": { "mouse_clicked": "close_window" } }
  ],
  "regions": [
    { "name": "content", "x": 0, "y": 0, "w": 500, "h": 360, "drag": false }
  ]
}
```

- `controls` 与 `elements` 等价；根数组等价于只有控件、尺寸用窗口缺省。
- **每个控件都必须写 `name`**：以英文字母开头，后续只允许英文字母 / 数字 / 下划线，最长 255 个字符；
  同一份布局里区分大小写唯一（`A1` 与 `a1` 是两个名字）；不能是 Rust 关键字。
  它是脚本认控件的标识符，窗口事件里对应 `event.control`；静态 `label` / `text_block` 也可用
  `win:update_content(name, content)` 按名称更新显示内容（`label` 需带每段颜色；仍不参与交互）。
- `width` / `height` 可省：`win:load_layout` 用句柄上的尺寸，`create{ layout = … }` 用创建参数；
  文件里写了就覆盖。
- `regions` 与 `set_content` 的 `regions` 同义；`drag` 缺省 `true`。
- 路径字段（`svg_path` / `img.path`）先按布局文件所在目录找，再逐级向上找；放在 `Scripts/lib/`
  的布局可以直接引用安装根目录的 `Images/...`，也可以把素材放在布局旁边。
- 文件上限 4 MB；控件数量、文字长度、图片像素等沿用 `script.md` 的内容限额。

## 事件响应

动态控件（`flat_svg_button` / `text_edit` / `text_box` / `hscroll_bar` / `vscroll_bar`）和区域条目可以带：

```json
"events": { "mouse_clicked": "close_window", "mouse_enter": "close_hover" }
```

- 键是窗口动作名，值是由脚本分发的处理器名；解析时校验动作名，未知动作让整份布局失败。
  扁平按钮的单击也接受 `click` / `double_click` 别名，真正派发的动作仍是 `mouse_clicked` /
  `mouse_double_clicked`。
- 解析器把事件表挂到对应**自动区域**（按钮 / 编辑框）或显式区域上；窗口事件载荷增加
  `handler` 和 `control`（命中的控件名）。脚本可以集中处理：

  ```lua
  cloudime.on("window", function(event)
      if event.handler == "close_window" then
          win:close()
      end
  end)
  ```

- 没写 `events` 的控件完全沿用旧行为：按 `event.region` / `event.element` + `event.action` 分发。
- `events` 只允许写在动态控件或 `regions` 上；写在 `rect` / `label` 等静态元素上会报错，
  避免“看起来绑上了、其实没有命中框”。普通元素要接事件请显式加 `regions`。

## 脚本接口

```lua
-- 直接以布局为初始内容；文件里的尺寸优先
local win = cloudime.window.create{
    name = "面板", layout = "Scripts/lib/main.json", interactive = true,
}
win:show{ x = 100, y = 100 }

-- 运行中换布局（整份替换）
win:load_layout("Scripts/lib/main.json")
```

- `load_layout` 与 `set_content` 一样只排队一个请求；失败时本次调用报 Lua 错误，窗口保持原样。
- 布局文件不监听变化；改文件后重新调用或重启 Server。
- 辅助窗口同样支持 `create_auxiliary{ layout = … }` / `aux:load_layout(...)`。

## 未做

- 布局文件中的窗口级选项（`interactive` / `shadow` / `background`）仍在 Lua 创建时给；
  JSON 只描述控件。
- 不做相对布局文件目录的路径换算、不做 `include` / 变量替换 / 自动布局约束。
- 不自动把 `events` 里的名字绑定成 Lua 函数；处理器名只是事件载荷的一部分，由脚本自己分发。
