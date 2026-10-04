# 把任务栏�?/ �?/ A 图标�?SVG 栅格化成四档 DPI �?8 �?alpha 蒙版�?
# DLL �?include_bytes! 嵌入，运行时按主题填色�?
# 需�?rsvg-convert �?magick �?PATH 中�?
$ErrorActionPreference = 'Stop'

# 脚本所在目�?-> 上溯三级得到 ROOT
$ROOT = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$OUT  = Join-Path $ROOT 'apps\windows\tsf\resources\mode'

New-Item -ItemType Directory -Force -Path $OUT | Out-Null

foreach ($name in 'zh','en','caps','off') {
    foreach ($size in 16,20,24,32) {
        $svg   = Join-Path $ROOT "assets\icon\windows\mode-$name.svg"
        $tmp   = Join-Path $OUT  'tmp.png'
        $alpha = Join-Path $OUT  "$name-$size.alpha"

        & rsvg-convert -w $size -h $size $svg -o $tmp
        if ($LASTEXITCODE -ne 0) { throw "rsvg-convert 失败: $name/$size" }

        # 注意�?${alpha}，否�?PowerShell 会把冒号当成变量作用�?驱动器前缀
        & magick $tmp -alpha extract -depth 8 "gray:${alpha}"
        if ($LASTEXITCODE -ne 0) { throw "magick 失败: $name/$size" }
    }
}

Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $OUT 'tmp.png')

Get-ChildItem $OUT | Format-Table Name, Length