<#
.SYNOPSIS
    在 Windows 上打云朵输入法安装包：release 构建三个产物 + 用 Inno Setup 编 cloudime.iss。
.DESCRIPTION
    在编译机（MSVC 工具链 + Inno Setup）上跑。步骤：
      1) cargo build --release 出 DLL / Server / 设置程序，再单独编一份 32 位 DLL；
      2) 从 apps\windows\server\Cargo.toml 读版本号（-dev 版接 git 短哈希）；
      3) 找 ISCC.exe（PATH 或常见安装位置）；
      4) iscc /DAppVersion=<版本> 编脚本，成品在 target\installer\cloudime-<版本>-windows-x86_64-setup.exe。
    随包数据（.qj / .tsv）直接由 .iss 从仓库 data\generated 与 assets 里取，不另建暂存目录；
    确保打包前 data\generated 里的 .qj 是最新的（打包流程见仓库 CLAUDE.md）。
    uiAccess 跟着 -Sign 走，不用手设 CLOUDIME_UIACCESS（见 -Sign）。
.PARAMETER SkipBuild
    跳过 cargo build（数据或 .iss 改了、二进制没变时重编安装包用）。
.PARAMETER Sign
    给产物签名并开 uiAccess（候选窗才能盖过商店 / 任务栏搜索）。uiAccess=true 的 exe 要有受信任的签名才起得来：
    缺省用 sign-local.ps1 的本机自签证书（只有编译机信任）——本机真机测用；对外分发时证书指纹从
    -CertThumbprint / CLOUDIME_CERT_THUMBPRINT 传（CI 导入 Certum 证书后自动带上），那时会同时给每个签名加时间戳。
    不带 -Sign 的包不签名、关 uiAccess：任何机器都能起，但候选窗在那几个系统界面里会被盖住。
.PARAMETER CertThumbprint
    签名证书指纹（SHA1）。缺省读环境变量 CLOUDIME_CERT_THUMBPRINT；都没有就走本机自签。
.PARAMETER TimestampUrl
    时间戳服务地址（RFC 3161）。给了正式证书又没指定时缺省用 DigiCert 的；自签模式缺省不加时间戳。
.PARAMETER SystemDeploy
    打出带「推荐 / 完整安装」选择的包；完整安装会把 DLL 写进 System32\IME\CloudIME / SysWOW64\IME\CloudIME 并注册。
    只允许与 -Sign 一起用（未签名构建只能推荐安装）；发版用正式签名时由 CI 打开。
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$Sign,
    [switch]$SystemDeploy,
    [string]$CertThumbprint = $env:CLOUDIME_CERT_THUMBPRINT,
    [string]$TimestampUrl = ''
)

$ErrorActionPreference = 'Stop'

# 调外部 exe（cargo / iscc / rustup…）：PowerShell 5.1 会把它们写进 stderr 的进度行与警告包装成错误记录，
# 配合上面的 Stop 会直接中断脚本。统一转成普通输出，只按退出码判断成败。
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

# 仓库根：本脚本在 apps\windows\installer 下，往上三层是 ime\。
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$Iss  = Join-Path $PSScriptRoot 'cloudime.iss'

# uiAccess 跟着 -Sign 走（server\build.rs 读这个变量，改了会自动重编 Server）；理由见 -Sign 的说明。
$env:CLOUDIME_UIACCESS = if ($Sign) { '1' } else { '0' }

# 签名身份：给了指纹就是正式证书（Certum），没给就走本机自签。
# 正式证书必须带时间戳：不带的话证书一过期，用户手里旧安装包的签名就跟着失效；自签不加（免网络依赖）。
if ($Sign -and -not $TimestampUrl -and $CertThumbprint) {
    $TimestampUrl = 'http://timestamp.digicert.com'
}

# 签名一组文件（.exe / .dll）。签名器是 sign-local.ps1，正式证书只认指纹、找不到就失败，不会悄悄退回自签。
function Invoke-Sign {
    param([string[]]$Files)
    $signArgs = @{ Path = $Files }
    if ($CertThumbprint) { $signArgs.Thumbprint = $CertThumbprint }
    if ($TimestampUrl) { $signArgs.TimestampUrl = $TimestampUrl }
    & (Join-Path $PSScriptRoot 'sign-local.ps1') @signArgs
}

if ($Sign) {
    $who = if ($CertThumbprint) { "正式证书 $CertThumbprint" } else { '本机自签证书（只有编译机信任，别对外分发）' }
    Write-Host "uiAccess=1，签名：$who" -ForegroundColor Yellow
} else {
    Write-Host 'uiAccess=0（对外分发：Server 任何机器都能起；候选窗在商店 / 任务栏搜索里可能被盖）' -ForegroundColor Cyan
}

# 1) 构建三个产物。
if (-not $SkipBuild) {
    Write-Host '构建 release 产物…' -ForegroundColor Cyan
    Push-Location $Repo
    try {
        Invoke-Checked 'cargo' @('build', '--release', '--locked',
            '-p', 'cloudime-windows-server', '-p', 'cloudime-windows-tsf', '-p', 'cloudime-windows-settings',
            '-p', 'cloudime-windows-ids',
            '-p', 'cloudime-mcp-server', '-p', 'cloudime-mcp-client-sidecar', '-p', 'cloudime-wordbank-transformer')
        Invoke-Checked 'cargo' @('build', '--release', '--locked',
            '-p', 'cloudime-windows-tsf', '--target', 'i686-pc-windows-msvc')
        # TSF DLL 按位数起固定名（cloudime_tsf_x64.dll / cloudime_tsf_x86.dll，不带版本号）：
        # Cargo 的 cdylib 输出名来自 lib name，两个 target 只能同名，所以编完在这里改名。
        foreach ($pair in @(
                @{ From = 'target\release\cloudime_tsf.dll'; To = 'target\release\cloudime_tsf_x64.dll' },
                @{ From = 'target\i686-pc-windows-msvc\release\cloudime_tsf.dll'; To = 'target\i686-pc-windows-msvc\release\cloudime_tsf_x86.dll' }
            )) {
            $from = Join-Path $Repo $pair.From
            if (Test-Path -LiteralPath $from) {
                Move-Item -LiteralPath $from -Destination (Join-Path $Repo $pair.To) -Force
            }
        }
    } finally { Pop-Location }
}

# 缺一个产物就早报错（TSF DLL 按位数起固定名，见 cargo-build.ps1 里的改名）。
$targets = @(
    'release\cloudime_tsf_x64.dll',
    'i686-pc-windows-msvc\release\cloudime_tsf_x86.dll',
    'release\cloudime-server.exe',
    'release\cloudime-settings.exe',
    'release\cloudime-ids.exe',
    'release\cloudime-mcp-server.exe',
    'release\cloudime-mcp-client.exe'
)
foreach ($t in $targets) {
    $p = Join-Path $Repo "target\$t"
    if (-not (Test-Path $p)) { throw "缺产物 $p，先跑一次不带 -SkipBuild 的构建" }
}

# 组字数据与 `cloudime-ids.exe` 配套；`.iss` 没有 skipifsourcedoesntexist，缺了会编到那行才失败。
$idsData = Join-Path $Repo 'data\generated\ids.qj'
if (-not (Test-Path $idsData)) {
    throw "缺组字数据：$idsData（先跑 cargo run --release -p cloudime-ids-build -- --ids assets/ids/IDS.TXT --components assets/ids/components.tsv --out data/generated/ids.qj）"
}

# 特殊字符输入器现在是 Rust 程序（apps/windows/ssi，cloudime-windows-symbols）：缺了先编。
$ssiExe = Join-Path $Repo 'target\release\SpecialSymbolsInserter.exe'
if (-not (Test-Path $ssiExe)) {
    throw "缺特殊字符输入器：$ssiExe（先跑 cargo-build.ps1 或 cargo build --release -p cloudime-windows-symbols）"
}

# 知意（增强档）模型随包：缺了安装包会不完整。
$zhiyiModel = Join-Path $Repo 'Models\laya-zhen-long-l12-int8g\laya.int8g.onnx'
if (-not (Test-Path $zhiyiModel)) { throw "缺知意模型：$zhiyiModel（增强档随安装包提供）" }

# 1.1) 知意（ONNX）运行库：取不到不阻塞打包，安装包里对应文件是 skipifsourcedoesntexist。
try {
    & (Join-Path $PSScriptRoot '..\scripts\fetch-onnxruntime.ps1')
} catch {
    Write-Warning "ONNX Runtime 没取到，增强（知意）档在装机后不可用：$($_.Exception.Message)"
}

# 本地词典（翻译 Tip）：清单与词典文件一起随包装进 {app}\LocalDictionary\。
$localDict = Join-Path $Repo 'LocalDictionary'
if (-not (Test-Path (Join-Path $localDict 'dictionaries.list'))) {
    throw "缺本地词典清单：$localDict\dictionaries.list（翻译 Tip 的选项就是它里面的显示名）"
}

# 1.2) 自包含 Windows App Runtime：设置程序不再依赖机器上装的框架包（Windows 10 上框架依赖的引导用不了，
#      见 apps\windows\settings\build.rs）。cargo 构建时 windows-reactor-setup 已按清单把运行时铺到
#      target\release\，这里挑进暂存目录；target\release 里还有 deps\ 之类的中间产物，不能整个目录装。
$runtimeStage = Join-Path $Repo 'target\installer\settings-runtime'
$runtimeList  = Join-Path $PSScriptRoot 'settings-runtime.txt'
# 清单是 UTF-8 且带中文注释：不指定编码时 PowerShell 5.1 按 GBK 读，注释末尾的字节会吞掉换行，紧跟其后的一项被当成注释漏掉。
$wanted = Get-Content $runtimeList -Encoding UTF8 | Where-Object { $_ -and -not $_.StartsWith('#') } | ForEach-Object { $_.Trim() }
if (Test-Path $runtimeStage) { Remove-Item $runtimeStage -Recurse -Force }
New-Item -ItemType Directory -Path $runtimeStage -Force | Out-Null
$missing = @()
foreach ($name in $wanted) {
    $src = Join-Path $Repo "target\release\$name"
    if (Test-Path $src) {
        Copy-Item $src -Destination (Join-Path $runtimeStage $name) -Recurse -Force
    } else {
        $missing += $name
    }
}
# 缺文件说明自包含运行时没铺成功（build.rs 下载 NuGet 或解 MSIX 失败），早报错，别打出个跑不起来的包。
if ($missing.Count -gt 0) { throw "自包含 Windows App Runtime 缺 $($missing.Count) 项：$($missing -join ', ')" }
Write-Host "自包含运行时 $($wanted.Count) 项 → target\installer\settings-runtime" -ForegroundColor Cyan

# 1.5) 签名（必须在 iscc 打包前：Inno 把已签的文件原样拷进安装包）。
# 完整安装（系统目录）只允许出现在签名构建里；未签名构建只能推荐安装。
if ($SystemDeploy -and -not $Sign) {
    throw '-SystemDeploy 只用于签名构建：请同时加 -Sign（发版用 Certum 签名）'
}
if ($Sign) {
    Write-Host '签产物（uiAccess 要求 Server 代码签名；Inno 打包时把已签的文件原样拷进去）…' -ForegroundColor Cyan
    Invoke-Sign -Files ($targets | ForEach-Object { Join-Path $Repo "target\$_" })
}

# 2) 从 server 的 Cargo.toml 读版本（apps\* 各自写死版本，不跟 workspace）。
$cargoToml = Get-Content (Join-Path $Repo 'apps\windows\server\Cargo.toml')
$verLine = $cargoToml | Where-Object { $_ -match '^\s*version\s*=\s*"(.+)"' } | Select-Object -First 1
if (-not ($verLine -match '"(.+)"')) { throw '在 server\Cargo.toml 里没找到 version' }
$Version = $Matches[1]
# 开发版接 git 短哈希（0.0.1-dev-1a2b3c4，脏加 +），有 bug 能定位到哪次改动；发版提交去掉 -dev 就不接。
if ($Version.EndsWith('-dev')) {
    Push-Location $Repo
    try {
        $rev = (git rev-parse --short HEAD 2>$null)
        if ($LASTEXITCODE -eq 0 -and $rev) {
            if (git status --porcelain 2>$null) { $rev = "$rev+" }
            $Version = "$Version-$rev"
        }
    } finally { Pop-Location }
}
# Inno 的 VersionInfoVersion 只认数字：去掉 -alpha.1 这类预发布后缀。
$VersionNumeric = $Version -replace '-.*$', ''
Write-Host "版本 $Version" -ForegroundColor Cyan

# 3) 找 ISCC.exe：先 Program Files 里的 7（与开发机同版本；CI 镜像 PATH 上自带 Chocolatey 的 6，不带简中翻译，不能让它抢先），
#    再 PATH，最后 6。CLOUDIME_ISCC 环境变量可直接指定。
$iscc = $env:CLOUDIME_ISCC
if (-not $iscc) {
    $candidates = @(
        "${env:ProgramFiles}\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { $iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source }
if (-not $iscc) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw '找不到 ISCC.exe：装 Inno Setup 7 或用 CLOUDIME_ISCC 指定' }
Write-Host "用 $iscc" -ForegroundColor Cyan

# 4) 编安装包。
# 打包前提醒一句：tutorial.md（使用手册）随包装进 {app}\tutorial.md，用户可见的改动要写进它；
# 拿 CHANGELOG.md 的修改时间当参照，只提醒不拦（修 bug 的发版可能确实不用动它）。
$tutorial = Join-Path $Repo 'tutorial.md'
$changelog = Join-Path $Repo 'CHANGELOG.md'
if ((Test-Path $tutorial) -and (Test-Path $changelog) -and
    (Get-Item $tutorial).LastWriteTime -lt (Get-Item $changelog).LastWriteTime) {
    Write-Host 'tutorial.md（使用手册）比 CHANGELOG.md 旧：确认这一版有没有用户可见的改动要写进手册。' -ForegroundColor Yellow
}

$isccArgs = @("/DAppVersion=$Version", "/DAppVersionNumeric=$VersionNumeric")
if ($SystemDeploy) { $isccArgs += '/DSystemDeploy=1' }
$isccArgs += $Iss
Invoke-Checked $iscc $isccArgs

$out = Join-Path $Repo "target\installer\cloudime-$Version-windows-x86_64-setup.exe"
# 安装包本身也要签：SmartScreen 看的是它。放在 ISCC 之后（签的是最终那个 exe，不是脚本里的内嵌文件）。
if ($Sign) {
    Write-Host '签安装包…' -ForegroundColor Cyan
    Invoke-Sign -Files @($out)
}
Write-Host "完成：$out" -ForegroundColor Green
