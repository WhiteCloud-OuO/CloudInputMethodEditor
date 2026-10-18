# 自绘文本编辑器待办（edit-todo）

状态：**暂缓，暂不实现**。本文件记录可行性结论、目标架构和后续分阶段任务；等确实需要富文本 /
自定义排版 / 跨平台文本核心时再启动。

当前实现继续使用 Windowless RichEdit：`text_edit` 单行、`text_box` 多行，相关代码见
`text_host.rs`、`text_store.rs`、`tsf.rs`。不要因为本文件存在就提前替换。

## 目标

- 去掉 `text_edit` / `text_box` 对 Windowless RichEdit（以及 `EDIT`）的依赖。
- 文本缓冲、布局、光标、选区、滚动、绘制全部由自己控制。
- 为后续能力留口子：富文本 run、语法高亮、内联装饰、占位符、行号、自定义排版、
  跨平台文本核心。
- 保持现有脚本 API、`Document` 控件结构、文本事件和测试布局不变。

## 为什么暂不做

- RichEdit 已经覆盖了编辑语义、剪贴板、撤销、TSF、字体回退和大量边界情况。
- 自绘最容易低估的不是排版，而是 **TSF 组句 / 候选窗定位 / composition 显示属性**。
- 当前 `text_box` 的 8 点滚动条、行距和自动换行已经能用；没有必须重写的紧迫性。
- 没有明确的富文本或跨平台需求前，重写收益不足以覆盖风险。

## 可行性结论

技术上可行，但应作为独立项目推进，不建议直接改现有宿主：

- `cloudime-render` 已依赖 `cosmic-text`，底层具备整形、字体回退、emoji、bidi、
  自动换行、`Buffer::hit` / `cursor_motion`；当前 `TextPainter` 只按单行使用。
- `text_store.rs` 已经是 `ITextStoreACP` 适配层，后端从 RichEdit 换成自绘模型在接口上可行。
- 主要风险集中在 TSF composition、UTF-16 ACP 映射、编辑语义和可访问性，不在字形绘制。

## 建议架构

```text
cloudime-text（新 crate，平台无关）
  TextBuffer        UTF-8 文本 + UTF-16 ACP 映射 + 编辑历史
  EditorState       光标 / 选区 / 首选 x / 视口 / composition
  TextLayout        cosmic-text Buffer、换行、行高、hit / cursor_motion
  EditCommand       插入、删除、替换、撤销 / 重做

Server script_window（Windows）
  EditorWindow      焦点、键盘 / 鼠标、绘制、滚动条
  TextStoreBackend  ITextStoreACP → EditorState / TextBuffer
  TextHost 绘制     cosmic / Canvas → 宿主窗（opaque BitBlt 或 layered）

cloudime-render
  复用 TextPainter / Canvas / Swash 缓存 / 字体库
```

不要把编辑模型放进 `cloudime-render`：渲染只负责画，撤销、TSF、剪贴板属于编辑器核心。

## 分阶段待办

### 阶段 0：只读 spike

- [ ] 新建 `cloudime-text` 或临时模块，建立 `TextBuffer` + UTF-16 ACP 映射。
- [ ] 用 cosmic-text 对 `text_box` 做多行布局：自动换行 / 不换行、行距、长行横向范围。
- [ ] 实现光标矩形、选区矩形、鼠标命中、上下左右移动。
- [ ] 在 `text_box` 宿主窗里自绘文字、光标和选区，不接 TSF。
- [ ] 与现有 RichEdit `text_box` 做截图 / 尺寸 / 换行位置对比。

验收：只读显示和滚动与现有 `text_box` 基本一致；不要求输入。

### 阶段 1：可编辑 `text_box`

- [ ] 输入字符、删除、Backspace、Delete、Home / End、Ctrl+方向键、PageUp / PageDown。
- [ ] 鼠标单击、Shift 扩选、拖选、双击选词、拖动时自动滚动。
- [ ] 剪贴板：复制、剪切、粘贴（`CF_UNICODETEXT`），粘贴替换选区。
- [ ] 撤销 / 重做，连续输入合并成一组；只读模式拒绝所有写操作。
- [ ] 光标闪烁、滚动到光标可见、内部滚动条状态和文本范围同步。
- [ ] 保留 `text_changed` / `key_pressed` / `focused` / `unfocused` 事件载荷不变。
- [ ] 文本 API：`get_text` / `set_text` / `select_all` / `replace` / `clear` 继续工作。

验收：CloudIME 输入法和微软拼音能正常组句、上屏，候选窗跟住光标。

### 阶段 2：TSF 与 composition

- [ ] 把 `text_store.rs` 的 RichEdit 调用改成 `TextStoreBackend` trait。
- [ ] 实现 `GetText` / `SetText` / `InsertTextAtSelection` / `GetSelection` /
      `SetSelection` / `GetTextExt` / `GetACPFromPoint` 的模型后端。
- [ ] ACP（UTF-16）与 UTF-8 字节索引双向映射；处理代理对和组合字符。
- [ ] 跟踪 `ITfComposition` 范围；组句文本下划线 / 显示属性由自己绘制。
- [ ] 组句开始 / 更新 / 结束、提交、取消、候选窗位置逐项回归。
- [ ] TSF 失败时保留普通键盘输入的回退路径。

这是 go/no-go 门槛：如果组句和候选窗无法稳定，停止迁移，保留 RichEdit。

### 阶段 3：`text_edit` 对齐

- [ ] 单行模式：水平滚动、省略号、光标跟随、SVG 旁边的文本区。
- [ ] `passkey` 密码显示；真实文本仍可由 API 读写。
- [ ] 只读、焦点切换、DPI、嵌套 `panel.children`、辅助窗口限制。
- [ ] 和现有文本事件、事件别名、自动区域名完全兼容。
- [ ] 保留 `text_box` 的 8 点内部滚动条和自动换行行为。

验收：随包测试窗口和现有单行 / 多行编辑框手工回归全部通过。

### 阶段 4：扩展能力（按需求再做）

- [ ] 多 run 富文本样式（颜色 / 粗体 / 斜体 / 下划线 / 删除线）。
- [ ] 内联对象、占位符、语法高亮、行号。
- [ ] UIAutomation / 辅助功能。
- [ ] 大文本增量布局和视口裁剪。
- [ ] 词法 / 输入法上下文的增量更新。

## 风险清单

- TSF composition 与显示属性：当前 RichEdit 代为处理，自绘必须补。
- TSF ACP 是 UTF-16 索引，cosmic / Rust 是 UTF-8 字节索引。
- 中文、emoji、组合字符、bidi 的选区与删除边界。
- 撤销分组、剪贴板、拖选、自动滚动等行为与用户预期对齐。
- 自绘宿主窗的透明 / 圆角 / DPI / 阴影，和现有 layered 渲染一致。
- 可访问性、输入法兼容性、不同宿主的回归测试成本。
- 两套引擎并存期间的维护成本；必须有 feature flag / 回退路径。

## 启动条件

满足以下至少一条再考虑启动：

- 产品明确需要富文本、内联装饰或自定义排版，RichEdit 无法低成本满足。
- 需要把文本核心抽成可跨平台复用，而不是只服务 Windows 脚本窗口。
- 现有 RichEdit 在目标场景出现无法规避的字体、输入法或滚动问题。

如果只是继续完善纯文本编辑框，优先修现有 RichEdit / 滚动条实现。

## 参考

- `apps/windows/server/src/ui/script_window/text_host.rs`
- `apps/windows/server/src/ui/script_window/text_store.rs`
- `apps/windows/server/src/ui/script_window/tsf.rs`
- `crates/cloudime-render/src/text/mod.rs`
- `docs/design/text-box.md`
- `docs/design/script.md`
