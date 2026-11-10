# cloudime-uiless-host

开发用的最小 TSF UI-less 宿主，验证云朵输入法的 UIElement 数据链路：

- 以 `TF_TMAE_UIELEMENTENABLEDONLY` 激活线程；
- 实现 `ITfUIElementSink`，`BeginUIElement` 回 `pbShow=FALSE`（宿主自己画）；
- 在 `UpdateUIElement` 里读 `ITfCandidateListUIElement`，打印条数、高亮、页码、候选文本；
- 自带最小 `ITextStoreACP`，组句 / 上屏能落进文本存区；
- 退出时打印文本存区内容。

## 用法

先确保 Windows 当前输入法是**云朵输入法**，然后在仓库根跑：

```powershell
cargo run -p cloudime-uiless-host -- --auto
```

`--auto` 会在窗口出现 1.2 秒后自动敲 `nihao` + 空格。去掉 `--auto` 则手动点一下窗口再打字：

```powershell
cargo run -p cloudime-uiless-host
```

`--auto-pick` 用来验宿主动作：敲 `nihao`（不补空格），0.8 秒后调用
`ITfCandidateListUIElementBehavior::SetSelection(1)` 和 `Finalize()`，应看到：

```text
[auto-pick] SetSelection(1)
[uielement] Update id=0
  [uielement] count=9 selection=Some(1) ...
[auto-pick] Finalize()
[uielement] End id=0
文本存区: "你会"    # 第二个候选，具体词随词库
```

预期输出（节选）：

```text
[uielement] Begin id=0 -> pbShow=FALSE
[uielement] Update id=0
  [uielement] count=9 selection=Some(0) page=Some(0) items=["你好", "你会", ...]
[uielement] End id=0
文本存区: "你好"
```

## 说明

- 不随安装包发布，也不注册 TSF；只用于开发验证。
- 这个宿主只验证 UIElement **数据**。服务端按 `host_draws` / `ui_less` 收起自己的候选窗、以及 `SetSelection` /
  `Finalize` 的完整效果，要等 P1-D（服务端抑制与动作）接入后再验。
- 控制台如果显示中文乱码，是 PowerShell 代码页的问题；文本存区内容本身是 UTF-8。
