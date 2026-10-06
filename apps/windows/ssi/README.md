# 特殊字符输入器（SpecialSymbolsInserter）

一个用 **勇芳 VisualFreeBasic（VFB）** 编写的特殊字符输入小工具，用于向任意程序快速输入键盘上难以打出的特殊符号（序号、角标、希腊字母、西里尔字母、日文假名、制表符等）。

- 点 **左键**：把按钮上的字符以「模拟键盘输入」的方式直接打到你当前正在打字的程序里；
- 点 **右键**：把按钮上的字符复制到系统剪贴板，方便手动粘贴；
- 窗口 **置顶 + 不抢焦点**：点击本工具时，键盘焦点仍停留在目标程序里，因此可以连续点击、连续输入，无需来回切换窗口。

---

## 一、目录结构

```
特殊字符输入器/
├─ SpecialSymbolsInserter.ffp     VFB 工程文件（双击用 VFB 打开）
├─ forms/
│  ├─ Form1.frm                   主窗口（标题栏 + 选项卡容器）
│  ├─ Form2.frm                   常用符号（第 1 个标签，56 个字符）
│  ├─ Form3.frm                   序号和角标（第 2 个标签，111 个字符）
│  ├─ Form4.frm                   希腊字母（第 3 个标签，76 个字符）
│  ├─ Form5.frm                   西里尔字母（第 4 个标签，100 个字符）
│  ├─ Form6.frm                   日文假名（第 5 个标签，110 个字符，五十音图）
│  └─ Form7.frm                   制表符（第 6 个标签，33 个字符，框线）
├─ modules/
│  └─ SpecialCharInput.inc        核心模块：模拟键盘输入 / 复制剪贴板
├─ images/icon.ico                程序图标
├─ release/                       32 位编译输出
└─ release64/                     64 位编译输出
```

总计约 **486 个** 特殊字符按钮，全部按分类分布在 6 个标签页。

---

## 二、界面与操作

| 位置 | 说明 |
| --- | --- |
| 顶部选项卡 | 共 6 页：常用符号 / 序号和角标 / 希腊字母 / 西里尔字母 / 日文假名 / 制表符 |
| 页面按钮 | 每个按钮显示一个特殊字符 |
| 左键单击 | 模拟键盘输入该字符（送入当前焦点窗口） |
| 右键单击 | 复制该字符到剪贴板 |
| 窗口标题栏 | 可拖动窗口；右上角可最小化 / 关闭 |

> 排版说明：
> - 第 2 页「序号和角标」按**类别分排**——带圈数字、括号数字、中文括号数字、中文带圈数字、大写罗马数字、小写罗马数字、上标、下标，每类各占新的一行（每行最多 14 个）；
> - 第 3 页「希腊字母」把**大写、小写、变体、带重音**分成四块，块间空出 32 像素；
> - 第 4 页「西里尔字母」同样分组（大写、小写、乌克兰语、塞尔维亚语、哈萨克语），块间 32 像素；
> - 第 5 页「日文假名」是**五十音图**：左 5 列平假名、右 5 列片假名，中间空出 32 像素；第 1–10 行为各行，第 11 行只有「ん / ン」。

> 典型用法：在记事本、Word、聊天窗口等程序里把光标放到要输入的位置 → 点击本工具对应字符 → 字符立即出现在目标程序中。

---

## 三、实现原理

### 1. 字符如何“打”进别的程序
核心模块 `modules/SpecialCharInput.inc` 中的 `InsertUnicodeText` 使用 Win32 的
`SendInput` + `KEYEVENTF_UNICODE`，把字符逐个以 Unicode 按键事件注入系统。

- 与键盘布局无关，可输入任意 Unicode 字符（中文、假名、序号、制表符等）；
- 自动处理 UTF-16 代理对（如少量非 BMP 字符）。

剪贴板部分 `CopyUnicodeToClipboard` 使用 `OpenClipboard` / `SetClipboardData(CF_UNICODETEXT)`，
以 UTF-16 文本形式写入剪贴板。

### 2. 为什么不抢焦点也能输入
主窗口 `Form1` 通过 `TopMost = True`，并在 `Form1_WM_Create`（窗口显示前）中再调用一次 `SetWindowPos(..., HWND_TOPMOST, ...)` 确保始终置顶；
同时用**两手准备**保证“点击不抢焦点、字符能送进目标程序”：

1. **点击不激活本窗口**：在 `Form1_WM_Create` 里（窗口显示前）给窗口加上 `WS_EX_NOACTIVATE`，
   并在 `WM_MOUSEACTIVATE` 里对客户区返回 `MA_NOACTIVATE`：
```vb
SetWindowLongPtr hWndForm, GWL_EXSTYLE, GetWindowLongPtr(hWndForm, GWL_EXSTYLE) Or WS_EX_NOACTIVATE
...
Function Form1_Custom(...) As LResult
   If wMsg = WM_ACTIVATE Then SetInputTarget Cast(HWND, lParam)   '记录“上一次的目标窗口”
   If wMsg = WM_MOUSEACTIVATE Then
      If HiWord(lParam) = HTCLIENT Then Return MA_NOACTIVATE      '点击客户区：不激活本窗口
   End If
   Function = False
End Function
```

2. **万一还是被激活了，就切回目标程序**：`modules/SpecialCharInput.inc` 里记录“目标窗口”
   （`SetInputTarget`，由上面的 `WM_ACTIVATE` 调用），发送前 `EnsureInputTarget` 若发现焦点在自己身上，
   就用 `AttachThreadInput + SetForegroundWindow` 把前台切回目标程序，再 `SendInput`。

> 为什么不用工程属性 `NoActivate=True`：它会触发 VFB 的“无焦点窗体多线程显示”机制，对**启动窗体**并不稳妥；
> 所以改为运行时加 `WS_EX_NOACTIVATE`。`MA_NOACTIVATE` 只对顶层窗口生效，点击子窗体时可能收不到，
> 因此还需要第 2 步兜底。


### 3. 标签页如何切换
主窗口只有一个 `TabControl1`，它的 `Custom` 属性记录了「标签数、当前选中项，以及每个标签绑定的子窗口」：

```
Custom=6|0|常用符号||Form2|0|序号和角标||Form3|0|希腊字母||Form4|0|西里尔字母||Form5|0|日文假名||Form6|0|制表符||Form7|0|
```

格式为：`标签数|当前选中|` + 对每个标签重复 `标题|图标|绑定子窗口|附加数据|`。

运行时 TabControl 会在切换标签时自动 `Show` / `Hide` 对应的子窗口（`Form2`~`Form7`），
无需手写切换代码。每个子窗口都是 `Child=True` 的子窗口，内部放了一排 `YFbutton` 虚拟按钮。

### 4. 按钮事件
每个符号页的 `[AllCode]` 中只有两个事件处理（以 Form3 为例）：

```vb
'左键：模拟键盘输入
Sub Form3_YFbutton1_WM_LButtonUp(ControlIndex As Long, hWndForm As hWnd, MouseFlags As Long, xPos As Long, yPos As Long)
   InsertSymbol Form3.YFbutton1(ControlIndex).Caption
End Sub

'右键：复制到剪贴板
Sub Form3_YFbutton1_WM_RButtonUp(ControlIndex As Long, hWndForm As hWnd, MouseFlags As Long, xPos As Long, yPos As Long)
   CopySymbol Form3.YFbutton1(ControlIndex).Caption
End Sub
```

`YFbutton1` 是控件数组，事件回调会传入被点按钮的索引 `ControlIndex`；
处理函数直接读取该按钮的 `Caption`（即字符本身）并送入模块，因此新增字符无需改动代码。

### 5. 字符的显示与编码（重点）
VFB 生成的源码是按工程代码页（本项目为 **GBK / 936**）保存的，因此**不在 GBK 字符集内**的字符
（例如 `۞ ♥ ♣ ♠ ♦ ✓ ☑ ⑫ ⑬ ₀ ⅓ °±` 等）在自动生成时会被写成“？”，运行时便显示成“？”。

解决办法：
- `.frm` 里仍写真实字符（设计器可正常显示）；
- 各页 `FormN_WM_Create` 事件里用 `WChr(&H码点)` 在运行时把这些按钮的 `Caption` 补正回来
  （`WChr(...)` 是纯 ASCII，不受代码页影响）。用 `WM_Create`（窗口显示前）而不是 `Shown`（显示后），
  是为了避免启动时看到“先显示 ? 再被修正”的闪烁。例如：

```vb
Sub Form3_WM_Create(hWndForm As hWnd, UserData As Integer)
   Form3.YFbutton1(95).Caption = WChr(&H207C)   ' 补正 ⁼
End Sub
```

此外，系统自带的“微软雅黑”缺少不少符号字形，因此按钮字体按字符**自动选择**：
优先 `Microsoft YaHei`（微软雅黑），缺字时用 `MS Gothic` 或 `Segoe UI`，已在 `.frm` 的 `Font=` 中逐按钮设置好。

---

## 四、如何编译 / 运行

1. 用 **勇芳 VisualFreeBasic（建议 5.8.4 及以上）** 打开 `SpecialSymbolsInserter.ffp`；
2. 工程默认编译为 **64 位**（`DefaultCompiler=64`），可点工具栏的「编译」「运行」；
3. 生成的可执行文件位于 `release64/`（32 位则在 `release/`）；
4. 主窗口标题为「特殊字符输入器」，启动后置顶显示。

> 提示：`#define UNICODE` 与 WinFBX（`afx/CWindow.inc`）已由工程模板自动包含，无需手工添加。

---

## 五、二次开发指南

### 增加 / 修改某页的字符
两种方式：

1. **在 VFB 设计器里改**：打开对应 `FormN.frm`，直接给按钮改 `Caption`（可增删按钮）；
2. **直接改 `.frm` 文本**：每个按钮对应一个 `[YFbutton]` 段，修改其 `Caption=` 即可。
   按钮网格参数（所有符号页统一）：
   - 按钮 **28×28**、每行 **14 个**、步长 31、从 (4,4) 开始；
   - `Form3`（序号和角标）每个类别另起一行，`Form4`（希腊字母）大写/小写分块。

   选项卡显示区约 **446 × 540** 像素（100% DPI，当前窗口尺寸下）。

### 增加新的一页
1. 复制一个现有 `FormN.frm` 为新的表单（例如 `Form8.frm`），改 `Name=` / `Caption=` 与按钮；
2. 在 `.ffp` 的 `[Objects]` 增加 `Form=.\forms\Form8.frm|0|0||Yes|`，并同步 `NumObjects`（=对象数+3）；
3. 在 `TopTab=` 增加一行；
4. 修改 `Form1.frm` 中 `TabControl1` 的 `Custom`：把标签数 +1，并追加 `新标题||Form8|0|`，
   同时保证整串以 `|` 结尾（这样解析器才能取到最后一组）。

### 修改“左键 / 右键”的行为
只需改 `modules/SpecialCharInput.inc` 里的 `InsertSymbol` / `CopySymbol`（或底层两个函数）。
所有符号页都会同步生效。

---

## 六、已知限制与可扩展点

- **首次使用**：程序刚启动时本窗口是前台窗口，请先点击一下目标程序把光标定位好，再点击符号（之后本工具不再抢焦点）。
- **管理员权限程序**：由于 Windows 的 UIPI 机制，普通权限的本工具无法向「以管理员身份运行」的程序注入按键；如需支持，请给本工具勾选 `UseAdminPriv`（工程属性）后以管理员运行。
- **个别特殊宿主**：少数使用底层键盘钩子 / 独占输入的程序可能收不到 `SendInput` 的 Unicode 字符，此时可用「右键复制」再手动粘贴。
- **常用符号页（Form2）布局**：该页由原有工程保留，其最右列按钮在设计宽度上略宽于选项卡显示区，可能存在轻微裁切；如需调整，可在设计器里整体左移或缩小间距。
- **剪贴板反馈**：目前右键复制后没有弹窗提示（避免打扰）；如需提示，可在 `CopySymbol` 后调用 `Form1.Caption` 短暂变化或用托盘气泡提示。
- **可扩展**：`SpecialCharInput.inc` 是通用模块，可复用到其它需要「模拟输入 / 复制文本」的工程中。

---

## 七、关键文件说明

| 文件 | 作用 |
| --- | --- |
| `modules/SpecialCharInput.inc` | 输入/复制的核心逻辑（`InsertUnicodeText`、`CopyUnicodeToClipboard`、`InsertSymbol`、`CopySymbol`） |
| `forms/Form1.frm` | 主窗口、选项卡、置顶与不抢焦点设置 |
| `forms/Form2.frm` ~ `Form7.frm` | 6 个符号页，每页含按钮数组及其左键/右键事件 |
| `SpecialSymbolsInserter.ffp` | 工程文件，登记所有表单与模块 |
