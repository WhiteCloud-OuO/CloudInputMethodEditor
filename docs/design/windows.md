# Server 侧自绘窗口的模型（2026-10-14）

## 0. 前提

- Server 进程里自绘的窗口都跑在 UI 线程上（普通置顶窗会被商店 / 任务栏搜索这些高 z-band 宿主盖住，
  只有本进程配合 uiAccess 签名才能盖过）。本文规定窗口之间的关系（§1–§5）、画法（§6 渲染）与
  实现落点（§7），不管画成什么样。
- 实现落在 `apps/windows/server/src/ui/`：两类窗口各有一个**模板**（见 §7）。
- **置顶**（`top_most`）是模板的窗口属性，缺省 **false**；Server 建好候选窗、状态条、状态提示与脚本窗口后
  显式 `set_top_most(true)`，需要回落时传 `false`（内部 `SetWindowPos(HWND_TOPMOST / HWND_NOTOPMOST)`）。

## 1. 两类窗口

按「谁管生命周期」分：

| 类 | 说明 | 现在的例子 |
|---|---|---|
| **独立窗口**（independent window） | 有自己那块窗，也有**自己的生命周期**（UI 线程建、按需显示 / 隐藏、随进程销毁） | 候选窗口、悬浮工具栏、状态切换提示、脚本建的窗口（`cloudime.window.create`） |
| **辅助窗口**（auxiliary window） | 挂在某块独立窗口上的辅助信息 / 提示窗口；它是那块独立窗口的**资产**（asset） | 候选窗口的「键位映射参考」，脚本建的辅助窗口（`cloudime.window.create_auxiliary`） |

- 每个辅助窗口**有且只有一个所有者**（owner）—— 它的宿主独立窗口；
- 资产挂在宿主名下的**资产表**里：宿主**先建好自己那块窗，再建资产**；宿主销毁**之前**先把资产放掉。
  资产的寿命就是这段区间。

## 2. 所有权

- 辅助窗口的所有权**在它的生命周期内不发生转移**，也**不会被直接借用** ——
  资产表只有「收进来」与「放掉」，没有「取出来」的口子。
- 这条只管辅助窗口。**独立窗口之间不受它约束**：它们各有自己的生命周期，需要通讯就**直接**通讯。

## 3. 桥接（bridging）

辅助窗口**不能**直接跟其他辅助窗口通讯，也**不能**跟不是它所有者的独立窗口通讯 ——
要几何、要开关、要内容，都得**委托它的所有者**。这条委托路叫**桥接**（bridging）。

- 桥上由**所有者**这一侧实现（实现上是一个 trait），每次驱动资产时把 `&dyn Bridge` **借**给它用一下；
- **借完就还**：借出去的是引用，资产**存不下**它（想留存就得给自己加生命周期参数，一眼看得出违规）；
- 宿主与资产之间**只有**这条桥：宿主驱动资产（`sync` / `hide` / `set_lines`），
  资产经桥向宿主要东西（`host()` / `asset_on()`）。
- 进程级的共用件（共享渲染器、显示器查询那套）不是窗口，不算「通讯对象」。

## 4. 焦点（focus，2026-10-17）

**焦点**是「让窗口具备响应键盘 / 鼠标的功能」的那个特性。规则：

1. **一组里任何时刻恰好一个**：焦点属于**一组**（一个独立窗口 + 它名下的资产，即「独立窗口及其所有的辅助
   窗口」），这一组里**同一时间只有一个**窗口具备焦点 —— 要么是所有者自己，要么是名下的某一件资产。
2. **缺省在所有者身上**：独立窗口创建并显示后**总是具备焦点**；辅助窗口**默认不具备焦点**（不响应鼠标 / 键盘）。
   所有者可以把它**指定**给某件资产，也可以**收回来**。
3. **允许获取焦点**（`able_to_get_focus`，**创建时**声明）：辅助窗口想让焦点能落到自己身上，得在建它的时候
   声明；没声明的**永远**接不住。声明了也只是「能」—— 创建之后的焦点仍在所有者手上。
4. **指定资产获取焦点**（`designated_asset_to_get_focus`）：**独立窗口都具备**这个特性 —— 它能把焦点从自己
   身上**指定**给名下某件声明过的资产（按资产的**名字**认）。只有所有者能指定，资产自己说了不算。
5. **同一所有者的两件辅助窗口之间转移焦点经桥接**：资产之间不直接通讯（§3），所以 A1 要让位给 A2 只能
   **委托所有者**，路径是 `A1 → 桥 → A（所有者）→ A2`。
6. **指定的后果**：焦点**转移**（不是复制）到那件资产上 —— 此后由它响应鼠标 / 键盘，**原先的所有者不再接受**。
7. **归还给所有者**（`return_focus_to_owner`）有三条路：所有者主动收回、那件资产**收起**、它走到**生命周期末**
   —— 后两条**自动**触发（收起时先归还再收起，销毁时先归还再放掉）：焦点不会落在一件看不见、或者马上就要
   没了的窗口上。
8. **克隆 / 窃取来的资产不抢焦点**：通过**资产克隆**（§5.1）或**资产窃取**（§5.2）拿到的辅助窗口与原件具备
   **同样的特性**（可以 / 不可以获得焦点照旧），但即便它能获得焦点，**克隆 / 窃取之后也不会第一时间获得** ——
   焦点仍在其所有者、或同一所有者的**姊妹辅助窗口**之一身上（要它拿焦点，还是得所有者**指定**）。窃取时
   原件被销毁，它原来那一组按规则 7 自动归还焦点；克隆则两边都不动（原件还归 B，焦点照旧）。

**术语**：把自己的焦点**转移给另一个窗口**的过程叫**失焦**（unfocusing），**获得**焦点的过程叫**聚焦**
（focusing）。两边都收得着这两个通知 —— 资产是 `Asset::focusing` / `Asset::unfocusing`，所有者是
`DesignatedAssetToGetFocus::focusing` / `unfocusing`（实现里拿它开关自己那块窗的鼠标）。

⚠️ 这是**逻辑焦点**，不是 Win32 的 `SetFocus`：咱们的窗口全是 `WS_EX_NOACTIVATE`，真去抢系统焦点会把用户
正在打字的那个应用弄丢。落地上「具备焦点」= 这一组里**由谁接受输入**：没有焦点的一方鼠标一律**穿透**
（点不着），键也轮不到它。

形状上大致是这样（**机制已实现**，触发留给后面几部分）：

```rust
/// 焦点在谁身上：所有者自己，还是名下的第几件资产（`Assets` 里的下标）。
pub(super) enum Focus {
    Owner,
    Asset(usize),
}

/// 资产：创建时声明「允许获取焦点」（缺省 `false` = 不具备焦点），并收**聚焦 / 失焦**的通知。
/// 名字在一个所有者的名下唯一，跨资产指认（桥上）与日志用它。
pub(super) trait Asset {
    fn name(&self) -> &'static str;
    fn able_to_get_focus(&self) -> bool { false }
    fn focusing(&self) {}
    fn unfocusing(&self) {}
}

/// 桥：资产**委托所有者**办事的那条路 —— 同一所有者的两件资产之间转手也走这儿。
pub(super) trait Bridge {
    fn designate_focus(&self, name: &str) -> bool;
    fn return_focus_to_owner(&self);
}

/// 独立窗口都实现它：把焦点指定给名下某件资产，或者收回来。
pub(super) trait DesignatedAssetToGetFocus {
    fn designate_focus(&self, name: &str) -> bool;
    fn return_focus_to_owner(&self) -> bool;
    fn focus(&self) -> Focus;
    fn focusing(&self) {}
    fn unfocusing(&self) {}
}
```

## 4.1 模态（modal，2026-11-02）

**模态**只属于**辅助窗口**（以及辅助窗口模板）：把它设成模态时，它的所有者**暂时不再响应鼠标 / 键盘**，
但所有者**不会被关闭**，桥接、`sync`、内容更新、取文本照旧。和焦点是两件事：模态管「挡住所有者」，
焦点管「这一组里由谁接受输入」；模态不自动改焦点，要模态窗口接输入还是由所有者**指定**（§4）。

规则：

1. **API**：辅助窗口实现 `Asset::set_modal(true/false)`；所有者一侧统一走
   `Assets::set_modal(name, modal)`（独立窗口模板暴露成 `set_asset_modal`）。
2. **同一所有者同时只有一个生效模态**：普通模态资产只允许一件；已经有模态（包括 dialog）时，再设另一件
   普通资产返回 `false`。**dialog 是例外**：创建 dialog 时把原来的模态栈顶**临时挂起**（不销毁、不丢失，
   `set_modal(false)` 只是暂时收起），把 dialog 压到栈顶；dialog 销毁后原来的模态自动恢复。
   多个 dialog 可以嵌套成栈，内层销毁后外层 dialog 恢复。
3. **始终在所有者上层**：辅助窗口建出来就是所有者的 **owned window**（`CreateWindowExW` 的 owner），
   Windows 保证 owned window 排在自己的所有者上面；设模态 / 每次显示时再 `HWND_TOP` 抬一次，
   免得刚显示 / 刚抬起的 RichEdit 宿主盖住它。所有者那侧在模态期间**不再把 RichEdit 宿主抬到最前**。
4. **所有者让开**：模态期间所有者整块窗鼠标穿透（`WS_EX_TRANSPARENT` + `WM_NCHITTEST -> HTTRANSPARENT`），
   名下的 RichEdit 宿主也跟着屏蔽（鼠标穿透 + 放掉键盘焦点）。这部分只走窗口输入，不影响
   [`Bridge`]（桥接仍是每次 `sync` 借出去的那个引用）。
5. **自动取消**：模态资产走到生命周期末（`Assets::retire` / `Assets::retire_named`）时**先取消模态再销毁**；
   只**收起**（`hide`）不取消 —— 再显示时模态还在。dialog 只能 `close`，不能 `hide`（否则会留下
   「挡着所有者但看不见」的窗口）；dialog 在生命周期内也**不能** `set_modal(false)`。
6. **dialog 的非法取消**：对 dialog 调 `set_modal(false)` 会**异常终止程序**（`std::process::abort`，
   不是 unwind panic），并在终止前写一条 `tracing::error!`。这是硬规则：dialog 的模态无法手动解除。

形状：

```rust
pub enum AssetKind { Auxiliary, Dialog }

pub(super) trait Asset {
    // ……
    fn kind(&self) -> AssetKind { AssetKind::Auxiliary }
    fn set_modal(&self, modal: bool) {}
    fn raise(&self) {}
    fn detach(&self, owner: &dyn Bridge) {}
}

impl Assets {
    /// 普通资产：同时只允许一件；已有模态时返回 false。
    /// dialog：由 add 自动压栈，set_modal(false) 直接 abort。
    fn set_modal(&self, name: &str, modal: bool) -> bool;
    fn modal(&self) -> Option<&str>;
    fn modal_is_dialog(&self) -> bool;
}
```

**规范用法**（独立窗口开一个模态辅助窗口）：

1. 先给独立窗口开**蒙版**（`WindowOptions::mask` / `Context::set_mask`，见 `docs/design/cloud-ui-app.md`）；
2. 再创建辅助窗口；
3. 辅助窗口建出来（并 `show`）之后调 `set_modal(true)`；
4. 辅助窗口销毁时，先取消模态 / 关闭辅助窗口，再关独立窗口的蒙版。

蒙版与模态**不自动联动**：蒙版是视觉上的「变暗」，模态是输入上的「挡住所有者」，
调用方按上面的顺序自己配。这样也能做「只蒙版不模态」或「模态但自绘遮罩」的变体。

### 4.1.1 内置对话框（dialog，2026-11-03）

**dialog 是辅助窗口模板的一种内置种类**（`AssetKind::Dialog`），正文布局由 Cloud UI 内部代码生成，
用户不需要写 JSON。`dialog` 只是统称；目前提供消息对话框（`MessageDialog`）和文本输入对话框
（`TextInputDialog`），两者共用 `dialog::layout` 的尺寸 / 排版、`dialog::DialogStyle` 的属性与
`dialog::blocking` 的同步模态宿主。

**所有对话框都是类**：`new(...)` 建实例 → 改属性 → `show(self)`。属性只在 `show` 之前有效：
Rust 的 `show(self)` 吃掉实例，编译期就挡住之后再改；脚本侧 `dlg:show()` 之后再赋值属性会抛 Lua 错误。

**属性（Rust `DialogStyle` / 脚本同名）**：

| 属性 | 缺省 | 意思 |
|---|---|---|
| `font` | `candidate` | 正文 / 标题 / 按钮字体槽：`candidate` / `pinyin` / `item_number` / `translate` |
| `text_size` | 14 | 正文与按钮字号（点） |
| `title_size` | 标题栏缺省（13） | 标题字号（点） |
| `background` | 主题背景色 | 面板底色 |
| `title_color` | 主题次要文字色 | 标题颜色 |
| `text_color` | 主题正文色 | 正文 / 编辑框文字颜色 |
| `button_background` | 主题强调色 | 按钮底色 |
| `button_text_color` | 主题强调文字色 | 按钮文字颜色 |

颜色都是 `Color`（脚本写 `"#RRGGBB"` / `"#AARRGGBB"`）；字号必须是正数，否则脚本侧报错。

- **创建即模态**：`Assets::add` 看到 `AssetKind::Dialog` 就自动把它压到模态栈顶；不需要、也不允许调用方再
  手动 `set_modal(true)`。
- **临时挂起原模态**：栈顶原来有普通模态（或外层 dialog）时，先 `set_modal(false)` 挂起，不销毁；
  dialog 销毁时 `retire_named` 弹出栈顶并恢复下一个。嵌套 dialog 同理。
- **位于最顶层**：dialog 建出来就 `set_top_most(true)`，并且是宿主的 owned window；每次 `show` / 开蒙版后
  再 `HWND_TOP`，不会被所有者或它的 RichEdit 宿主盖住。
- **自动转移焦点**：创建时如果焦点在别的资产上，先经桥接让那件资产 `unfocusing` / 把焦点交还所有者，
  再由所有者指定给 dialog（dialog 默认 `able_to_get_focus = true`）。
- **可选蒙版**：`DialogOptions::mask = true` / 脚本 `mask = true` 时，dialog 在第一次显示时记录所有者原来的
  蒙版并切到 `WindowMask::dim(0.35)`；销毁前 `Asset::detach` 通过 `Bridge::set_mask` 恢复原值。
- **不可取消**：生命周期内 `set_modal(false)` → `std::process::abort()`；`hide` 也不允许（脚本侧会被拒绝并记警告）。
- **可拖拽（带着所有者一起走）**：在 dialog 面板上按下左键并拖动（按钮区域除外；编辑框由它自己的宿主接鼠标），
  对话框和所有者按**同样的位移**一起移动。所有者仍是模态状态：`EnableWindow` 保持禁用，拖拽只是挪窗口位置，
  不恢复鼠标 / 键盘输入。拖动超过 4 像素才算拖拽；拖拽后的左键抬起不再当成按钮点击。
  所有者的 **RichEdit 宿主**和**蒙版**都是独立顶层窗口，不会跟着窗口移动，dialog 拖拽时由
  `AuxiliaryWindowTemplate` 按相同位移一起搬（`follow_edit_host` / `follow_windows`）；文本输入对话框
  自己的编辑框宿主也登记为 follower。脚本所有者另外由 `ScriptWindow` 的 `WM_MOVE` 重新对齐缓存位置、
  RichEdit 宿主、蒙版与资产。
- **脚本入口**：`cloudime.window.create_dialog{ owner = win, mask = true }`，返回和普通窗口一样的句柄；
  `win:close()` 销毁，`win:set_modal(false)` 会触发异常终止。

#### 消息对话框（message dialog）

```rust
let mut dialog = MessageDialog::new(title, text, icon, &["确定", "取消"]);
dialog.style.text_size = Some(15.0);   // 属性只在 show 之前改
let index = dialog.show();             // u8：被点按钮的下标 0..=3
```

- **布局**：从上到下三段 —— 标题栏（只有标题文字，**没有关闭按钮**；高度 / 字号 / 颜色 / 左留白与
  `cloud-ui-demo` 主窗口标题栏一致，标题栏下**不画分隔线**）、内容区（左 SVG + 右 `TextBlock`）、
  按钮组（文字按钮，1–4 个）。
- **结果**：返回被点按钮的下标 `0..=3`。没有关闭按钮，必须点一个按钮；`buttons` 为空或超过 4 个时
  **异常终止程序**（`std::process::abort()`，不是 panic）。
- **同步**：`show` 在当前独立窗口的 UI 上下文里创建 owned window，跑一个嵌套消息循环；
  期间所有者窗口 `EnableWindow(FALSE)`，返回前恢复。所有者来自线程本地当前窗口
  （`crate::current`，由独立窗口的 UI 回调设置）；没有当前窗口也异常终止。
  所有者名下的 RichEdit 宿主是**独立顶层窗口**，`EnableWindow` 管不到：`BlockingPanel` 在打开前
  通过 `current::CurrentWindow.edit_hosts` 逐个 `set_input_blocked(true)`（鼠标穿透 + 放掉键盘焦点），
  关闭后取消并恢复原焦点。
  背景由消息对话框自己的整块 `Element::Rect` 画，模板底色关掉，避免深色模式下两套圆角底色在边缘叠出浅色亮边。
- **文本**：`\n` 换行，`\\` 表示反斜杠；按内容区宽度自动折行，左图标和按钮组固定尺寸。
- **图标**：`icon` 取值 `""`（缺省，不画图）/ `failure` / `success` / `query` / `warning`；
  四张 SVG 直接作为 `&str` 常量内嵌在 `cloud-ui`（`dialog/message.rs`），
  保留源文件颜色，不跟随主题染色。
- **脚本**：

  ```lua
  local dialog = cloudime.window.message_dialog{
      owner = win,
      title = "删除确认",
      text = "要删除这条记录吗？\n删除后无法恢复。",
      icon = "warning",
      buttons = { "删除", "取消" },
  }
  dialog.text_size = 15
  dialog.button_background = "#C42B1C"
  local index = dialog:show()          -- 同步阻塞，返回 0..=3
  ```

  实例上可直接改 `title` / `text` / `icon` / `buttons` 与上表属性；`show` 之后再改报 Lua 错误。

#### 文本输入对话框（text input dialog）

```rust
let mut dialog = TextInputDialog::new(title, text, allow_multiline, default_text);
dialog.style.title_color = Some(Color::rgb(150, 150, 162));
let value = dialog.show();             // String
```

- **布局**：标题栏（与 demo 主窗口同款，无关闭按钮、无分隔线）/ 内容区（说明 `TextBlock` + 编辑框）/
  按钮组（确定 / 取消）。尺寸与按钮绘制与消息对话框共用 `dialog::layout`。
- **编辑框**：`allow_multiline = true` 用多行 `text_box`，否则用单行 `text_edit`；背景 / 边框由
  `Element::TextEdit` / `Element::TextBox` 画，真正的输入由 `TextHostWindow` 盖在 `edit_rect` 上。
  打开即把焦点交给编辑框并**全选缺省文本**，直接输入就是替换。
- **返回**：
  - 确定 → 编辑框当前内容；
  - 取消 → `default_text`；`default_text` 为空时返回空字符串。
- **单行换行**：`allow_multiline = false` 时，`default_text` 里的真实换行在编辑框里显示为字面量 `\n`，
  确定后原样返回。
- **同步 / 所有者 / 背景**：与消息对话框相同，走 `dialog::blocking` 的同一个嵌套消息循环、
  `current` 当前窗口和模板背景关闭逻辑。
- **脚本**：

  ```lua
  local dialog = cloudime.window.text_input_dialog{
      owner = win,
      title = "重命名",
      text = "输入新名字：",
      allow_multiline = false,
      default_text = "旧名字",
  }
  dialog.font = "candidate"
  local value = dialog:show()          -- 同步阻塞，返回字符串
  ```

  实例上可直接改 `title` / `text` / `allow_multiline` / `default_text` 与上表属性；
  `show` 之后再改报 Lua 错误。底层是同一个 `TextInputDialog` 的 `show`。

## 5. 资产克隆与资产窃取

> 本输入法暂时用不上这两条，但规则和**来源侧入口**先定下来（`Assets::clone_asset` / `Assets::steal_asset`）；
> 具体资产要额外实现 `Clonable` 才允许被克隆。

**模态资产禁止离开它的所有者**：如果 **B1 正被 B 设为模态**，A **不能**克隆或窃取它 —— 这是硬规则，
来源侧入口会**直接 panic**，不会返回 `None`。理由：模态窗口的 z-order、输入屏蔽、蒙版配合都绑在 B 上，
复制一份到 A 会出现两个「模态」而只有一个所有者，规则立刻自相矛盾。检查顺序固定为
「先查模态 → 再查 `Clonable`」，所以模态资产不会因为没实现 `Clonable` 而绕过 panic。

### 5.1 资产克隆（asset cloning）

独立窗口 **A** 想拿到另一个独立窗口 **B** 的资产 **B1**，要**同时**满足四条：

1. **B1 在生命周期之内**（B 还没把它放掉）；
2. **B1 具备允许克隆的特性** —— 它除了实现「资产」那个 trait，还要额外实现一个**可克隆** trait
   （下称 `Clonable`）；
3. **B 收到来自 A 的克隆请求**（独立窗口之间可以直接通讯，请求走它们自己那条路）；
4. **B1 不是模态资产** —— 模态窗口不能离开它的所有者（见 §5 开头），来源侧入口直接 panic。

四条都满足时：

- **A 添加一件 A1**：**除所有者以外**，其余内容与 B1 一样（连 [`Asset::name`] 一起照抄 —— 名字只要求
  在同一所有者的名下唯一；要放进一个已经有同名资产的所有者名下，得另起一个名字，`Assets::add` 的
  `debug_assert` 会叫）；
- **A1 的所有者是 A**（不复制所有者，也不会是 B）；
- **B1 不受影响**：还归 B，还在 B 的生命周期里 —— **焦点也不受影响**（B1 本来拿着 B 那一组的焦点就还拿着：
  克隆不扰动别人的组）；
- **焦点不跟着走**（§4 规则 8）：A1 进来时 A 那一组的焦点**不动**，还在 A 自己、或它的某件姊妹资产身上 ——
  A1 能不能获得焦点照旧（`able_to_get_focus` 是资产自己的特性，克隆件自然带着），但要拿到还是得 A **指定**。

形状（来源侧入口已实现；具体资产还没实现 `Clonable`）：

```rust
pub trait Asset {
    // ……
    /// 允许被克隆时返回自己；缺省 None（不可克隆）。实现 Clonable 的类型覆盖它。
    fn as_clonable(&self) -> Option<&dyn Clonable> { None }
}

/// 允许被别的独立窗口克隆的资产额外实现它。
pub trait Clonable: Asset {
    /// 克隆一份「除所有者以外」都一样的自己；克隆件由**请求方**持有、所有者是请求方。
    fn clone_asset(&self) -> Box<dyn Asset>;
}

impl Assets {
    /// 克隆 / 窃取前的硬检查：name 是当前模态资产时 panic。
    fn ensure_cloneable(&self, name: &str);

    /// 来源侧克隆：原件不动。没这件 / 不可克隆 → None；模态 → panic。
    fn clone_asset(&self, name: &str) -> Option<Box<dyn Asset>>;

    /// 来源侧窃取：先克隆、再 retire_named 原件。模态 → panic。
    fn steal_asset(&mut self, name: &str) -> Option<Box<dyn Asset>>;
}
```

### 5.2 资产窃取（asset stealing）

**A 从 B 克隆 B1 成 A1，且 B1 在克隆之后被立即销毁** —— 这整个过程叫**资产窃取**。

- **B1 正模态时禁止窃取**：和克隆同一条硬规则，来源侧 `Assets::steal_asset` 在销毁原件**之前**
  就先 panic，不会出现「原件已经没了、副本没拿到」的半截状态；
- 之后 A 名下的资产是 **A1**，**不再是 B1**；**B1 已经被抹去**；
- 克隆件是**新的**资产（不是引用、也不是句柄），所以 B1 被销毁不影响 A1，反过来也一样；
- **焦点两边都不跟着走**：A 那一组按 §4 规则 8 不动（还在 A 或它的姊妹资产身上）；B 那边在 B1 被抹去**之前**
  按 §4 规则 7（生命周期末）**自动归还**焦点 —— 焦点不会跟着一件正在消失的资产走。

## 6. 渲染（rendering）

两类窗口画内容只有这一种方法：**先画进一块内存区域，画完整块贴到窗口上**。云朵渲染器把一整帧（阴影、圆角、
文字、图标）出成一块**预乘 RGBA 位图**（tiny-skia 画形状 / 路径，cosmic-text + swash 画字，resvg 画 SVG，
阴影是手写盒式模糊），`cloud_ui::layered` 换通道后 `UpdateLayeredWindow` **整块一次**贴上（`ULW_ALPHA`，逐像素 alpha）。
落地上：

- 窗口过程**不处理 `WM_PAINT`**（分层窗口本来就不走这条路）；窗口类**不设 `hbrBackground`**、也不调
  `InvalidateRect` —— 没有「画布被系统擦掉」这种时机；
- 一块内存区域 = **一帧**：内容变了（新候选、翻页、高亮滑动、换 DPI、换主题）才重新出图再贴；没变就不贴。
  贴上去的那一帧一直留在窗口上 —— 这就是渲染的**持续性**；
- **超出窗口内容区的部分不显示**：`Canvas::set_clip` 把后续内容裁到内容区（不含阴影留白），
  阴影 / 背景在设置裁剪前画，所以不受影响；文字、路径、SVG、图像都走同一个裁剪。行 / 容器排不下时
  不会把内容画到窗口外面。
- 要「擦掉」只有两条路：**贴一帧新的**（覆盖），或者**销毁窗口**。不存在「刷新之后要重画一遍」这回事；
- 为什么只能是它：**逐像素 alpha**（透明边、柔和阴影、抗锯齿圆角）只有带 alpha 的位图能做；候选窗每帧都在动，
  贴整块才不会闪。

**能画的元素**（八类）：

| 元素 | 是什么 | 落在哪儿（实现） |
|---|---|---|
| `text` | 文本 | 字形管线：cosmic-text 选字体 / 整形 / 回退，swash 栅格；彩色 emoji 直通 RGBA（`cloudime-render` 的 `text`） |
| `svg` | SVG 图形 | `usvg` 解析 + `resvg` 光栅化成一整张带 alpha 的位图再叠上，可按 alpha 染色（`svg.rs`） |
| `window_icon` | 窗口字形（最小化 / 最大化 / 还原） | 按当前 DPI 取整图标盒与线宽（`round(1.2×DPI/96)`），同一 DPI 下三个字形线宽一致；不经过 SVG 缩放、不走 AA（`renderer::draw_window_icon`） |
| `img` | 图像（来自**文件**或**文件句柄**） | PNG 解码成预乘 RGBA 位图（`image::decode_image` / `load_image` / `read_image`），按目标矩形双线性缩放画上（`draw_image`） |
| `line` | 线条 | 细矩形（水平 / 垂直）或细长的**填充路径**（任意角度，带抗锯齿）—— 画布的 `fill_rect` / `fill_path`。**眼下没有窗口用**：光标是 `rect` 的细条，下划线 / 删除线在文字管线里画 |
| `rect` | 矩形（含**圆角矩形**与**圆形**） | `fill_rect` / `fill_round_rect`：圆角用三次贝塞尔近似四分之一圆；**圆** = 边长相等、半径取半边长（**眼下没有窗口用**） |
| `path` | 路径 | `fill_path`：tiny-skia 的 `Path`（直线 / 三次贝塞尔 / 闭合，按非零环绕填充）—— `round_rect` 就是拿它拼的，**但没有窗口直接调 `path`**（都经 `round_rect` 走） |

**现在各窗口画了什么**（自绘的几个窗口全部走**渲染**）：

| 元素 | 候选窗口 | 悬浮工具栏 / 状态切换提示 | 辅助窗口（键位映射参考） |
|---|---|---|---|
| `text` | 拼音行（已敲 / 未敲 / 划掉三段）、候选文字、序号、页码、译文 Tip、来源角标、提示行（如「辅助码对不上，没有候选」） | —（状态条与状态切换提示只有图标，没有文字格） | 键名、声母（`zh` / `ch` / `sh`）、韵母及其首字母分色（a / o / e / i / u / ü） |
| `svg` | — | 12 个状态图标（顺序与显隐来自 `data\icons-arrangement.cfg`） | — |
| `img` | — | — | —（**能力已备**：`image.rs` 能从文件 / 句柄读 PNG 并缩放画上，眼下没有窗口用） |
| `line` | — | — | — |
| `rect` | 面板背景（圆角）、高亮条（圆角）、拼音光标（一条细竖条）、阴影 | 面板背景（圆角）、阴影 | 面板背景（圆角）、格线（相邻两格共用一条，宽 1 点） |
| `path` | — | — | — |

> `line` / `path` / `img`（以及 `rect` 里的「圆」）眼下**没有窗口直接调**：光标与格线都是 `rect` 的细条，下划线 / 删除线在文字管线里画，圆角矩形走 `round_rect`（内部才是路径），图像能力备着还没用。
> 阴影不是单独一类元素：它是「圆角矩形 → 3 次盒式模糊成遮罩 → 按主题色上色」（`shadow.rs`），算 `rect` 的合成。
> 状态条的**悬停提示**是系统 tooltip 控件（comctl32 自己画自己擦），不在上表里，不归我们的渲染器；
> 设置程序走 XAML、特殊字符输入器是另一个 exe，也都不在上表范围。

**实现**：元素各是一条 trait（`elements` 模块），实现是 `Render`（`cloudime-render`：一块离屏 tiny-skia 位图
+ 共享的字形管线）。窗口里只准经这些 trait 画，别在别处再抄一遍原语：

- `TextElement::text` / `measure`、`SvgElement::svg`、`ImageElement::image`、`LineElement::line`、
  `RectElement::rect` / `round_rect` / `circle`、`PathElement::path`；
- `Render` 与量字共用同一套字形管线（`TextPainter` 句柄共享）：**画字的时候顺便量字**（`Render::measure`）。

**取舍**（2026-10-17 把元素收进 traits 之后）：

- **换来的**：元素只在**一处**定义（线的粗细、圆的贝塞尔、图像的缩放都落在实现里），窗口不再各抄原语；
  可以**按元素测**（`Render` 一篇像素断言）；出图的方法不再要 `&mut self`（字形管线共享），借用约束少了一大截。
- **付出的**：① 字形管线成了 `Rc<RefCell<…>>` —— 编译期独占变成**运行期检查**，嵌套借用会 panic
  （眼下的调用都满足「量字与画字不同时持有」）；② 量字有**两条路**：出图途中用 `render.measure`、排版前置用
  `self.measure`，边界是「手上有没有 `Render`」。

### 6.1 区域（region，2026-10-17）

窗口内容里的一块**可交互范围**：**名字** + **命中形状**（与元素同一套：矩形 / 圆角 / 圆 / 线段 / 路径）+
**能不能拖**。元素只管画；鼠标点击 / 拖拽看区域 ——

- **悬浮工具栏**：每个图标一个区域（名字 = 当前图标名，如 `ch` / `options`），拖它们都是拖整块状态条；
- **脚本窗口**：内容里声明 `regions`（`cloudime.window`），事件里带命中的区域名；`drag = false` 的区域
  按住了不拖窗口（点击照常）；
- 命中规则：后写的区域盖住先写的；**先看区域、再看元素**（`event.region` / `event.element` 各给一份）。
- **合并 / 剪除**：区域可以带 `combine`（并集）与 `prune`（差集）形状；命中先看 `prune`，再看原始形状与
  `combine`。命中检测只做**成员判断**（并集 = 任一形状命中、差集 = 任一剪除形状命中就不算），
  不把轮廓真的做布尔运算，所以对矩形 / 圆角 / 圆 / 路径都成立。
- **形状与关系**：区域的形状不限于矩形（脚本里 `shape = "rect" | "round_rect" | "circle" | "path"`）。
  `Region::contains_region` / `intersects` 用 `i_overlay` 把形状转成多边形做并 / 差 / 交，
  给出内含 / 相交 / 重合 / 相离（只贴一条边不算相交）；脚本侧另有 `region_contains` 查点。

实现：`cloudime_render::Region`（数据：名字 / 形状 / `draggable`）、`cloud_ui::region::hit`（命中）、
脚本窗口的 `begin_drag` 按命中区域的 `draggable` 决定要不要开始拖窗口。

## 7. 实现落点

- **独立窗口模板**（`crates/cloud-ui/src/independent_window_template/`）：注册窗口类 + 建一块**分层、不抢焦点**的弹出窗
  （纯展示的再加 `TRANSPARENT` 让鼠标穿透；置顶缺省关，`set_top_most(true)` 才进 topmost band），并**管名下的资产** —— `Assets` 的 `add` / `sync` / `hide` /
  `set_lines` / `retire`，`Drop` 里**先 `retire` 再 `DestroyWindow` 自己**（§1 的寿命规矩由模板保证）。
  候选窗口、悬浮工具栏、状态切换提示与脚本独立窗口都用它建窗（候选 / 脚本窗口再往里挂资产）。
  **焦点**也由它兜着（§4）：焦点状态在资产表里，模板实现 `DesignatedAssetToGetFocus` —— 指定 / 收回时自己
  **失焦 / 聚焦**（`unfocusing` / `focusing`，两边都落到 `sync_input()` 上），并给窗口过程一个 `focused()`：
  焦点不在自己身上时整块让开，自己那块窗鼠标穿透（`WS_EX_TRANSPARENT`，`cloud_ui::window::set_mouse_transparent`），
  窗口过程的 `WM_NCHITTEST` 也按 `focused()` 回 `HTTRANSPARENT`。
  - **辅助窗口模板**（`crates/cloud-ui/src/auxiliary_window_template/`）：资产用的那块**分层面板**（主题背景 + 圆角 + 阴影
    都出一张预乘 RGBA 位图里；宽 / 高 / 间隙三个属性，没设过按宿主形状算默认；`show(宿主内容区矩形, 排布方向)`
    摆位 —— 宿主右边 / 下边放不下就换左边 / 上边；置顶缺省关，`set_top_most` 显式开）。画什么三选一：**空面板** / **键位映射参考** / **脚本文档**
    （`Document`，`set_document`；dialog 目前先用空面板）；出图 `Painter::render_panel` / `render_key_map` / `render_document`，
    `layered::present` 一次贴上；窗口过程**不处理 `WM_PAINT`**。`show` 按「位置 / 尺寸 / 版面版本 / 渲染器
    代号」判重，没变就不重贴。**焦点**：`new` 收一个 `able_to_get_focus`，`set_focused` 开关鼠标穿透
    （`WS_EX_TRANSPARENT` 与命中测试读同一个位，两边不会打架）。**鼠标**：`set_events` 装一个出口，
    点击 / 悬停 / 离开变成内容区坐标 + 命中元素（脚本辅助窗口用；键位映射参考不装，一律穿透）。
    `new` 还收宿主的 `HWND`：辅助窗口建出来就是宿主的 **owned window**（排所有者上面），模态靠这条关系；
    `set_modal` / `raise` 再 `HWND_TOP` 兜底，`show` 时如果正模态也抬一次。
    另一种内置种类是 **dialog**：`AuxiliaryWindowTemplate::dialog(painter, owner, DialogOptions { mask })`，
    `AssetKind::Dialog`、默认可获取焦点、创建即置顶；正文当前先留空（`Content::Empty`），之后按具体种类补。
    `mask = true` 时 `apply_mask` 通过 `Bridge` 记下原蒙版并切到 `WindowMask::dim(0.35)`，`restore_mask` 恢复。
- **资产这层**（`crates/cloud-ui/src/auxiliary/`）：`Asset`（资产：`name`、`kind`（普通 / dialog）、声明 `able_to_get_focus`、
  收 `focusing` / `unfocusing`、`set_modal` / `raise`、生命周期末的 `detach`）、
  `Assets`（宿主名下的资产表 + 这一组的**焦点**与**模态栈**；`add` 普通资产**不动焦点**，§4 规则 8；
  `add` dialog 自动压栈并转移焦点；`retire_named` 放掉指定的一件——脚本 `close` 用；`set_modal(name, modal)`
  仲裁普通模态，dialog 上取消会 abort，模态资产销毁时先取消再放掉）、`Bridge`（桥：
  `designate_focus(name)` 是同一所有者的两件资产之间转手的路，`return_focus_to_owner()` 是把焦点交回去，
  `mask()` / `set_mask()` 给 dialog 开关蒙版）、
  `Host`（宿主喂给资产的形状：内容区矩形 / 排布方向 / 是不是展开着）、`Focus`（焦点在谁身上，§4）、
  `DesignatedAssetToGetFocus`（指定资产获取焦点）、`IndependentWindowTemplate::set_asset_modal`（所有者一侧的模态开关，
  开 / 关都同步自身的鼠标穿透）。
- **脚本窗口**（`crates/cloud-ui/src/script_window/`，2026-10-17）：Lua（`cloudime.window.*`）建出来的两类窗口。管理器
  `ScriptWindows` 拿脚本请求（`WindowRequest`，经 `UiCommand::ScriptWindow` 从工人线程过来）建 / 改 / 收；
  `ScriptWindow` 用**独立窗口模板**、`ScriptAuxiliary` 用**辅助窗口模板**并作为宿主资产（`Assets::add`）；
  `WindowRequest::CreateDialog` 建内置 dialog，`ScriptAuxiliary` 的 `kind = Dialog`。
  脚本窗口自己实现 `Bridge::mask` / `set_mask`：`sync_mask` 用共享 `mask::MaskWindow` 按内容区和主题圆角摆放，
  dialog 要求蒙版时由它开、销毁时恢复。
  内容是一份声明式 `Document`，经 `Painter::render_document` 一次出图；命中按形状（`HitShape`：圆角 / 圆 /
  线段 / 路径填充）；鼠标、滚轮、拖动（整块窗）、键盘焦点（`key` 载荷带 `window`）都在这层。
  新建脚本窗口默认用 JSON 布局文件描述控件，Lua 只做交互；只有动态整份变化的场景才内联 `set_content`。
  规则（API、限额、事件载荷）在 `docs/design/script.md`「脚本窗口」。
- **画元素**（`cloudime-render`）：`elements`（六条元素 trait）、`render`（`Render`：一块离屏位图 + 共享的
  字形管线；候选窗 / 状态条 / 状态提示 / 辅助窗口面板 / 脚本窗口全走它）、`document`（脚本内容的元素表 +
  `Renderer::render_document`：面板 + 元素 + 每个元素的命中框）。
- 现在有两类资产：候选窗口的**键位映射参考**（`apps/windows/server/src/ui/key_map/`，名字 `key_map`；它**没**声明
  `able_to_get_focus`，所以候选窗那一组一直由候选窗口持有焦点）与**脚本辅助窗口**（名字 `script-<id>`，
  `interactive = true` 时声明可获取焦点、由脚本 `aux:focus(true)` 走所有者指定）。
