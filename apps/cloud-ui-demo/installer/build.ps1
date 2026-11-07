<#
.SYNOPSIS
    打包 cloud-ui 独立应用示例（Cloud UI Demo）的安装包。
.DESCRIPTION
    先 release 构建 apps/cloud-ui-demo，把 exe、图标与说明文件铺进
    target\installer\cloud-ui-demo\stage，再用 Inno Setup 编 apps\cloud-ui-demo\installer\cloud-ui-demo.iss。
    成品在 target\installer\cloud-ui-demo\cloud-ui-demo-<版本>-windows-x86_64-setup.exe。

    安装包按当前用户安装（PrivilegesRequired=lowest），不需要管理员，不写注册表；
    卸载直接删安装目录。这个包只作为独立应用打包的参考，不随云朵输入法安装包发布。
.PARAMETER SkipBuild
    跳过 cargo build，直接用 target\release\cloud-ui-demo.exe。
.PARAMETER SkipPack
    只准备 stage，不调用 ISCC，方便核对安装内容。
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File apps\cloud-ui-demo\installer\build.ps1
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File apps\cloud-ui-demo\installer\build.ps1 -SkipBuild -SkipPack
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipPack
)

$ErrorActionPreference = 'Stop'

# PowerShell 5.1 会把外部命令写进 stderr 的进度当错误；这里统一转成普通输出再判断退出码。
function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments 2>&1 | ForEach-Object { Write-Host $_ }
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($exitCode -ne 0) { throw "$Exe $($Arguments -join ' ') 失败（退出码 $exitCode）" }
}

$Repo      = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$AppDir    = Join-Path $Repo 'apps\cloud-ui-demo'
$Iss       = Join-Path $PSScriptRoot 'cloud-ui-demo.iss'
$Readme    = Join-Path $PSScriptRoot 'README.txt'
$Icon      = Join-Path $AppDir 'resources\cloud-ui-demo.ico'
$Exe       = Join-Path $Repo 'target\release\cloud-ui-demo.exe'
$OutDir    = Join-Path $Repo 'target\installer\cloud-ui-demo'
$Stage     = Join-Path $OutDir 'stage'

# 版本号跟 workspace 里的 cloud-ui-demo 包走（避免用 cargo metadata，PS 5.1 的 JSON 解析器有长度上限）。
$appManifest = [IO.File]::ReadAllText((Join-Path $AppDir 'Cargo.toml'))
if ($appManifest -match '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} elseif ($appManifest -match '(?m)^version\.workspace\s*=\s*true') {
    $rootManifest = [IO.File]::ReadAllText((Join-Path $Repo 'Cargo.toml'))
    if (-not ($rootManifest -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')) {
        throw '根 Cargo.toml 的 [workspace.package] 里没有 version'
    }
    $Version = $Matches[1]
} else {
    throw 'apps\cloud-ui-demo\Cargo.toml 里没有 version / version.workspace'
}
$VersionNumeric = $Version -replace '-.*$', ''
if ($VersionNumeric -notmatch '^\d+(\.\d+){0,3}$') {
    throw "版本号 '$Version' 不能写成 Inno 的 VersionInfoVersion"
}

if (-not $SkipBuild) {
    Write-Host '构建 release 版 cloud-ui-demo…' -ForegroundColor Cyan
    Push-Location $Repo
    try {
        Invoke-Checked 'cargo' @('build', '--release', '--locked', '-p', 'cloud-ui-demo')
    } finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $Exe)) {
    throw "找不到 $Exe；先 cargo build --release -p cloud-ui-demo"
}
if (-not (Test-Path -LiteralPath $Icon)) {
    throw "找不到 $Icon"
}

# stage 只放安装包真正需要的文件。
if (Test-Path -LiteralPath $Stage) { Remove-Item -LiteralPath $Stage -Recurse -Force }
New-Item -ItemType Directory -Path $Stage -Force | Out-Null
Copy-Item -LiteralPath $Exe -Destination $Stage
Copy-Item -LiteralPath $Icon -Destination $Stage
Copy-Item -LiteralPath $Readme -Destination $Stage
Write-Host "stage：$Stage" -ForegroundColor DarkGray

if ($SkipPack) {
    Write-Host '已跳过打包（-SkipPack）。' -ForegroundColor Green
    return
}

$iscc = $env:CLOUDIME_ISCC
if (-not $iscc) {
    $candidates = @(
        "${env:ProgramFiles}\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}
if (-not $iscc) {
    $command = Get-Command iscc.exe -ErrorAction SilentlyContinue
    if ($command) { $iscc = $command.Source }
}
if (-not $iscc) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}
if (-not $iscc) {
    throw '找不到 ISCC.exe：安装 Inno Setup 7，或用 CLOUDIME_ISCC 指定路径'
}
Write-Host "用 $iscc" -ForegroundColor Cyan

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
Invoke-Checked $iscc @(
    "/DAppVersion=$Version",
    "/DAppVersionNumeric=$VersionNumeric",
    "/DStage=$Stage",
    "/DOutputDir=$OutDir",
    $Iss
)

$out = Join-Path $OutDir "cloud-ui-demo-$Version-windows-x86_64-setup.exe"
if (-not (Test-Path -LiteralPath $out)) { throw "没有生成 $out" }
Write-Host "完成：$out" -ForegroundColor Green
