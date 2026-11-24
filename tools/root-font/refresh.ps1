<#
刷新字根字体：改完 roots.tsv 或生成器里的 SHAPES 之后跑一次。

用法（任意目录都行，脚本自己找仓库根）：
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\root-font\refresh.ps1
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\root-font\refresh.ps1 -Preview
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\root-font\refresh.ps1 -InstallDir 'D:\Program Files\CloudIME'
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\root-font\refresh.ps1 -NoDeploy   # 只生成、不复制到安装目录

做的事：
  1. 用 tools\root-font\gen_roots_font.py 依 roots.tsv 重新生成 zigen.ttf 和 roots_pua.tsv；
  2. 复制到安装目录的 InputSchemes\ShapeCode\wubi\（字体被 Server 占着时会先停 server，下次按键会自动拉起）；
  3. 核对两边的 SHA256，打印结果。

脚本文件要存成「带 BOM 的 UTF-8」，否则 PowerShell 5.1 会按系统 ANSI 解析、中文变乱码。
#>
[CmdletBinding()]
param(
    # 源字体。默认用本机装的更纱黑体 SC Regular；SHAPES 里的坐标就是按它量的。
    [string]$Source = 'C:\Windows\Fonts\sarasa-gothic-sc-regular.ttf',

    # 安装目录。留空时用 %ProgramFiles%\CloudIME。
    [string]$InstallDir = '',

    # 只生成字体，不复制到安装目录。
    [switch]$NoDeploy,

    # 字体被占用时不自动停 cloudime-server（默认会停）。
    [switch]$NoStopServer,

    # 另外输出一张全部字形的对照图到 InputSchemes\ShapeCode\wubi\zigen-preview.png。
    [switch]$Preview
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent (Split-Path -Parent $scriptDir)
$schemeDir = Join-Path $repo 'InputSchemes\ShapeCode\wubi'
$generator = Join-Path $repo 'tools\root-font\gen_roots_font.py'

if (-not (Test-Path -LiteralPath $generator)) { throw "找不到生成器：$generator" }
if (-not (Test-Path -LiteralPath $Source)) { throw "找不到源字体：$Source" }
if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    throw '找不到 python；生成器要 fonttools 和 skia-pathops：python -m pip install fonttools skia-pathops'
}

Write-Host '== 1/3 重新生成字根字体'
$genArgs = @($generator, '--source', $Source)
if ($Preview) { $genArgs += @('--preview', (Join-Path $schemeDir 'zigen-preview.png')) }
Push-Location $repo
try {
    & python @genArgs
    if ($LASTEXITCODE -ne 0) { throw "生成器退出码 $LASTEXITCODE" }
}
finally {
    Pop-Location
}

$font = Join-Path $schemeDir 'zigen.ttf'
$table = Join-Path $schemeDir 'roots_pua.tsv'
if (-not (Test-Path -LiteralPath $font)) { throw "没有生成字体：$font" }
$fontInfo = Get-Item -LiteralPath $font
$tableInfo = Get-Item -LiteralPath $table
Write-Host ("   字体 {0:N0} 字节，码位表 {1:N0} 字节" -f $fontInfo.Length, $tableInfo.Length)

if ($NoDeploy) {
    Write-Host '== 2/3 跳过复制（-NoDeploy）'
    Write-Host '== 3/3 完成'
    return
}

if (-not $InstallDir) {
    # 按常见安装位置找（安装目录不一定在系统盘）
    $candidates = @()
    if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'CloudIME') }
    if (${env:ProgramFiles(x86)}) { $candidates += (Join-Path ${env:ProgramFiles(x86)} 'CloudIME') }
    foreach ($drive in (Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        $candidates += (Join-Path $drive.Root 'Program Files\CloudIME')
    }
    $InstallDir = $candidates |
        Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ 'InputSchemes\ShapeCode\wubi')) } |
        Select-Object -First 1
}
if (-not $InstallDir) {
    throw "找不到安装目录里的 InputSchemes\ShapeCode\wubi；用 -InstallDir 指定，比如 -InstallDir 'D:\Program Files\CloudIME'"
}
$target = Join-Path $InstallDir 'InputSchemes\ShapeCode\wubi'
if (-not (Test-Path -LiteralPath $target)) { throw "安装目录里没有 $target；用 -InstallDir 指定正确路径" }
Write-Host "== 2/3 复制到安装目录：$target"

$names = @('zigen.ttf', 'roots_pua.tsv', 'roots.tsv')
$copyAll = {
    foreach ($name in $names) {
        Copy-Item -LiteralPath (Join-Path $schemeDir $name) -Destination (Join-Path $target $name) -Force
    }
}
try {
    & $copyAll
}
catch {
    if ($NoStopServer) { throw "字体被占用：$_（去掉 -NoStopServer 可自动停 server）" }
    Write-Host '   字体文件被占用，停掉 cloudime-server 再复制（下次按键会自动拉起）'
    Get-Process -Name cloudime-server -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 900
    & $copyAll
}

Write-Host '== 3/3 核对'
foreach ($name in $names) {
    $left = (Get-FileHash -LiteralPath (Join-Path $schemeDir $name) -Algorithm SHA256).Hash
    $right = (Get-FileHash -LiteralPath (Join-Path $target $name) -Algorithm SHA256).Hash
    $state = if ($left -eq $right) { '一致' } else { '不一致！' }
    Write-Host ("   {0,-14} {1}" -f $name, $state)
}

Write-Host ''
Write-Host '完成。切到「形码输入 = 五笔」、组句时就能看到新的字根（键位映射参考是「始终」的话会自动弹）。'
