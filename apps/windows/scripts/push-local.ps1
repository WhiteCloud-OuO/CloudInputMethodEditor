<#
编出 release 产物，再交给 reconfigure.ps1 覆盖安装目录并刷新（覆盖 / 重注册 / 重启 ctfmon / Explorer / Server 都在那边）。
适合「只改了代码，数据文件与版本号都没动」的开发循环；换了随包数据、改了版本号、或给别人装机请照旧跑安装包。

用法：
  pwsh -File apps\windows\scripts\push-local.ps1                  # 编译 release + 覆盖 + 刷新（覆盖那步弹一次 UAC）
  pwsh -File apps\windows\scripts\push-local.ps1 -SkipBuild       # 已经编好了，只覆盖与刷新
  pwsh -File apps\windows\scripts\push-local.ps1 -SkipSettings    # 设置程序没改，不编也不拷
  pwsh -File apps\windows\scripts\push-local.ps1 -SkipExplorer    # 刷新时不重启 Explorer
  pwsh -File apps\windows\scripts\push-local.ps1 -InstallDir 'D:\Program Files\CloudIME'

为什么编译要带 `CLOUDIME_UIACCESS=0`：没签名的 Server 带 uiAccess 起不来（os error 740），与 test-local.ps1 一致。
本地覆盖上去的 Server 因此没有 uiAccess，候选窗在少数提升 / UWP 宿主里可能被盖住；要 uiAccess 的正式件请跑安装包。

脚本存成带 BOM 的 UTF-8（Windows PowerShell 5.1 对无 BOM 的 .ps1 按系统 ANSI 解析，中文乱码还报语法错）。
#>

[CmdletBinding()]
param(
    # 已安装目录；缺省交给 reconfigure.ps1 按注册表 / 登录自启快捷方式反查。
    [string]$InstallDir = '',

    # 跳过编译（target\release 里已经有现成的产物）。
    [switch]$SkipBuild,

    # 设置程序没改：不编也不拷 cloudime-settings.exe。
    [switch]$SkipSettings,

    # 透传给 reconfigure.ps1：不重启 Explorer。
    [switch]$SkipExplorer
)

$ErrorActionPreference = 'Stop'

# 仓库根：从脚本所在目录往上找「有 version.txt 且有 apps\windows」的那一层（脚本被拷到别处也能跑）。
$RepoRoot = $PSScriptRoot
while ($RepoRoot) {
    if ((Test-Path -LiteralPath (Join-Path $RepoRoot 'version.txt')) -and
        (Test-Path -LiteralPath (Join-Path $RepoRoot 'apps\windows'))) { break }
    $parent = Split-Path -Parent $RepoRoot
    if (-not $parent -or $parent -eq $RepoRoot) { throw "找不到仓库根：$PSScriptRoot" }
    $RepoRoot = $parent
}

if (-not $SkipBuild) {
    Write-Host '== 编译 release（CLOUDIME_UIACCESS=0）==' -ForegroundColor Cyan
    $env:CLOUDIME_UIACCESS = '0'
    Push-Location -LiteralPath $RepoRoot
    # cargo 把进度写 stderr，PS 5.1 在 $ErrorActionPreference='Stop' 下会把它当错误终止脚本：
    # 编译期间临时放宽，成败仍由 $LASTEXITCODE 判。
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        # `cmd /c "… 2>&1"`：cargo 把进度写 stderr，PS 5.1 会把它当错误记录（红字）甚至终止脚本；
        # 经 cmd 合并到 stdout 后就只是普通输出，退出码仍是 cargo 的。
        & cmd /c "cargo build --release -p cloudime-windows-server -p cloudime-windows-tsf 2>&1"
        if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败（Server + TSF DLL）' }
        & cmd /c "cargo build --release -p cloudime-windows-tsf --target i686-pc-windows-msvc 2>&1"
        if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败（32 位 TSF DLL）' }
        if (-not $SkipSettings) {
            & cmd /c "cargo build --release -p cloudime-windows-settings 2>&1"
            if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败（设置程序）' }
        }
    }
    finally {
        $ErrorActionPreference = $previous
        Pop-Location
    }
}

# TSF DLL 按位数起固定名（`cloudime_tsf_x64.dll` / `cloudime_tsf_x86.dll`，不带版本号）：Cargo 的 cdylib
# 输出名来自 lib name，两个 target 只能同名，所以编完在这里改名（与 cargo-build.ps1 一致）。
foreach ($pair in @(
        @{ From = 'target\release\cloudime_tsf.dll'; To = 'target\release\cloudime_tsf_x64.dll' },
        @{ From = 'target\i686-pc-windows-msvc\release\cloudime_tsf.dll'; To = 'target\i686-pc-windows-msvc\release\cloudime_tsf_x86.dll' }
    )) {
    $tsfFrom = Join-Path $RepoRoot $pair.From
    if (Test-Path -LiteralPath $tsfFrom) {
        Move-Item -LiteralPath $tsfFrom -Destination (Join-Path $RepoRoot $pair.To) -Force
    }
}

# 用 hashtable 传具名参数：PowerShell 5.1 的数组 splat（@('-Name', 值)）只按位置传，
# 会把 '-Name' 当成值、后面的值当参数名（reconfigure.ps1 里也记了这一条）。
$reconfigureArgs = @{ InstallDir = $InstallDir }
if ($SkipSettings) { $reconfigureArgs.SkipSettings = $true }
if ($SkipExplorer) { $reconfigureArgs.SkipExplorer = $true }
& (Join-Path $PSScriptRoot 'reconfigure.ps1') @reconfigureArgs
