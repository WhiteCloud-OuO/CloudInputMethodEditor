# 图标

- `logo.png`（866×866，带透明通道）：应用图标源文件。`apps/windows/tsf/resources/cloudime.ico` 是由它生成的
  多尺寸 Windows 图标（随仓库提交）：DLL `include_bytes!` 内嵌并注册到 `%ProgramData%\CloudIME\`，设置程序与
  Server 的 exe、安装器（`SetupIconFile`）都用这一份。
- `windows/mode-zh.svg` / `mode-en.svg` / `mode-caps.svg`：Windows 任务栏的中 / 英 / A 图标源文件（16×16 画布，单色）。
  `windows/render-mode-icons.sh` 用 rsvg-convert + magick 栅格化成 16 / 20 / 24 / 32 四档的 8 位 alpha 蒙版，
  写到 `apps/windows/tsf/resources/mode/`，DLL 用 `include_bytes!` 嵌入、运行时按任务栏深浅色填色（`com/mode/icon.rs`）。
  改了 svg 重跑脚本，生成物随仓库提交。
