# 本地联调：拿刚编出来的 Server 与 TSF DLL，配一份装着真实数据的安装目录跑起来。
#
# 为什么不直接在仓库里跑：`bundled_root()` 按 exe 位置找数据，`target\debug\` 下会落到仓库根，
# 而 `data\generated\`（词库 / LM）是 gitignore 的、本机多半没有，于是退回 `assets\sample\`
# 样例，**候选只剩样例里的几条**，会让人误以为输入法没装好。
# 拷一份安装目录出来跑，`bundled_root()` 就落在它上，数据是真的。
#
# 为什么要换 DLL：`cloudime_core::Candidate` 是线上格式的一部分（见 `protocol/mod.rs`），
# Core 那边加一个 `CandidateKind` 变体，老 DLL 就解不出整条帧、把按键原样放行——表现是
# 「输入法突然只出英文」。所以两边必须同一份源码编出来的。
#
# 用法：
#   pwsh -File apps\windows\scripts\test-local.ps1
#   pwsh -File apps\windows\scripts\test-local.ps1 -SkipBuild -SkipRegister
#
# 管理员不是必须的：注册 DLL 那一步脚本会自己判断，不是管理员就把命令打出来让你自己跑。

[CmdletBinding()]
param(
    # 已安装的目录（真实数据从这里拷）。
    [string]$InstallDir = "$env:ProgramFiles\CloudIME",

    # 联调用的工作目录。**不要用 %TEMP%**：它在这台机器上是 8.3 短名（`C:\Users\TONYWU~1\...`），
    # PowerShell 走不通，`cd` 会报「An object at the specified path ... does not exist」。
    [string]$WorkDir = "$env:USERPROFILE\cloudime-devtest",

    # 跳过编译（已经编好了）。
    [switch]$SkipBuild,

    # 跳过注册 DLL（只想换 Server）。
    [switch]$SkipRegister
)

$ErrorActionPreference = 'Stop'

# apps\windows\scripts -> apps\windows -> apps -> 仓库根
$repo = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$server = Join-Path $repo 'target\debug\cloudime-server.exe'
$dll = Join-Path $repo 'target\debug\cloudime_tsf.dll'
$config = Join-Path $env:APPDATA 'CloudIME\config.toml'
$logs = Join-Path $env:APPDATA 'CloudIME\logs'

# 文本服务的 CLSID，与 `apps/windows/tsf/src/com/mod.rs` 的 `CLSID_CLOUDIME` 同步。
# 注册完查这里指向哪儿——比 regsvr32 的退出码可靠（见下）。
$TsfClsid = '{50020AC6-80FE-4709-BC25-A173B14A6E7F}'

function Assert-Path($path, $what) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "$what 不在：$path"
    }
}

Write-Host '== 前置检查' -ForegroundColor Cyan
Assert-Path $InstallDir '安装目录（先装一次云朵输入法，或者用 -InstallDir 指过去）'
if (-not $SkipBuild) {
    Write-Host '   安装目录 ✓'
}
else {
    Assert-Path $server 'Server 产物（去掉 -SkipBuild 先编一次）'
    Assert-Path $dll 'TSF DLL 产物（去掉 -SkipBuild 先编一次）'
}

if (-not $SkipBuild) {
    # 旧的那次注册可能还指着 `target\debug`，TSF 会按需把 DLL 载进各个宿主（记事本、终端、资源管理器…）。
    # 被载着时编译写不进去，cargo 报的是「failed to remove file ... 拒绝访问」，看着像代码坏了。
    # 先查出来点名，别让人去猜。
    $holders = @(
        tasklist /m cloudime_tsf_x64.dll /m cloudime_tsf.dll /NH 2>$null |
            Select-String -SimpleMatch 'cloudime_tsf'
    )
    if ($holders.Count -gt 0) {
        Write-Host '== 有进程正加载着 TSF DLL，编译写不进去' -ForegroundColor Yellow
        $holders | ForEach-Object { Write-Host "   $($_.Line.Trim())" -ForegroundColor Yellow }
        Write-Host ''
        Write-Host '   切走输入法就能放开（TSF 在切走时卸载 DLL）：Win+Space 换到别的输入法，或 Shift 切英文。' -ForegroundColor Yellow
        Write-Host '   还不行就关掉那个窗口重开，在新窗口里别切到云朵输入法，直接跑本脚本。' -ForegroundColor Yellow
        Write-Host '   （查：tasklist /m cloudime_tsf.dll）' -ForegroundColor Yellow
        exit 1
    }

    Write-Host '== 编译 Server 与 TSF DLL' -ForegroundColor Cyan
    # uiAccess manifest 缺省是开的，没签名的 exe 带它会直接起不来（Permission denied）；
    # 与 ci.yml 的 windows job 同一个开关
    $env:CLOUDIME_UIACCESS = '0'
    Push-Location $repo
    try {
        cargo build -p cloudime-windows-server -p cloudime-windows-tsf
        if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
    }
    finally {
        Pop-Location
    }
    Assert-Path $server 'Server 产物'
    Assert-Path $dll 'TSF DLL 产物'
}

Write-Host '== 停掉在跑的 Server' -ForegroundColor Cyan
# 管道名只有一个，旧的占着不放，新起来的连不上——那是静默的二选一，会让人以为新代码没生效
$running = Get-Process -Name cloudime-server -ErrorAction SilentlyContinue
if ($running) {
    $running | Stop-Process -Force
    Write-Host "   停掉 $($running.Count) 个"
}
else {
    Write-Host '   没有在跑的'
}

Write-Host "== 拷一份安装目录到 $WorkDir" -ForegroundColor Cyan
# 不动 Program Files：不需要管理员，也不会把「装好的那份」搞成半新半旧
robocopy $InstallDir $WorkDir /MIR /NFL /NDL /NJH /NJS | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy 失败（exit $LASTEXITCODE）" }

Copy-Item -LiteralPath $server -Destination $WorkDir -Force
# DLL 也拷过来再注册：从 `target\debug` 注册的话，那个文件会被加载它的进程锁住，
# 之后 `cargo build` 写不进去（链接报 LNK1104，看着像代码坏了）
$workDll = Join-Path $WorkDir 'cloudime_tsf.dll'
Copy-Item -LiteralPath $dll -Destination $workDll -Force
Write-Host '   新 Server 与 TSF DLL 已就位'

# 悬浮状态条的按钮：装好的那份可能还没带这两个，直接从仓库补一份
# （与 server\build.rs 拷到 target\{debug,release}\data\ 的是同一份）
$statusSrc = Join-Path $repo 'apps\windows\server\src\ui\status'
$workData = Join-Path $WorkDir 'data'
New-Item -ItemType Directory -Path $workData -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $statusSrc 'icons-arrangement.cfg') -Destination $workData -Force
Copy-Item -LiteralPath (Join-Path $statusSrc 'icons') -Destination (Join-Path $workData 'icons') -Recurse -Force
Write-Host '   状态条图标已就位'

# 词库：都放在 WordBank\ 下（主词库 Dict.db），装好的那份可能还是旧布局，直接从仓库补一份
$bankSrc = Join-Path $repo 'WordBank'
if (Test-Path -LiteralPath $bankSrc) {
    Copy-Item -LiteralPath $bankSrc -Destination (Join-Path $WorkDir 'WordBank') -Recurse -Force
    Write-Host '   词库目录已就位'
}
else {
    Write-Host '   仓库里还没有 WordBank\，状态条会退回样例词库（先跑 tools\dict-convert）' -ForegroundColor Yellow
}

$elevated = [Security.Principal.WindowsPrincipal]::new(
    [Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $SkipRegister) {
    Write-Host '== 注册 TSF DLL' -ForegroundColor Cyan
    if ($elevated) {
        & regsvr32.exe /s $workDll
        $exit = $LASTEXITCODE
        # regsvr32 的退出码不可信（实测注册成功仍返回 3），所以读注册表看结果。
        # 读 HKLM 那一份：提权时 regsvr32 写的就是它。别读 HKCR——那是 HKLM + HKCU 的合并视图，
        # 实测给过假警报，把一次成功的注册报成失败、白停一轮。
        $registered = (
            Get-ItemProperty `
                -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\$TsfClsid\InprocServer32" `
                -ErrorAction SilentlyContinue
        ).'(default)'
        if ($registered -ieq $workDll) {
            Write-Host "   已注册新 DLL（regsvr32 退出码 $exit，不代表失败）"
        }
        else {
            # 只警告不中断：这个判据本身出过假警报，中断的代价是白跑一整轮
            Write-Host "   ！注册可能没生效：InprocServer32 = «$registered»，期望 «$workDll»（regsvr32 退出码 $exit）" -ForegroundColor Yellow
            Write-Host '     先继续；输入法不出候选时再手工 regsvr32 一次。' -ForegroundColor Yellow
        }
    }
    else {
        Write-Host '   不是管理员，请另开一个管理员 PowerShell 跑：' -ForegroundColor Yellow
        Write-Host "     regsvr32 `"$workDll`"" -ForegroundColor Yellow
    }
}

Write-Host '== 起 Server' -ForegroundColor Cyan
Start-Process -FilePath (Join-Path $WorkDir 'cloudime-server.exe') -WorkingDirectory $WorkDir
Start-Sleep -Seconds 3

Write-Host ''
Write-Host '接下来手动做这四步：' -ForegroundColor Cyan
Write-Host ''
Write-Host '  1. 看日志确认对了（应该没有「协议」相关的警告）：'
Write-Host "       Get-Content `"$logs\cloudime-server.*.log`" -Tail 20"
Write-Host '     词库那行应该是一万以上，不是 148（148 说明落到样例数据了）。'
Write-Host ''
Write-Host '  2. 配置在（保存即热加载，不用重启）：'
Write-Host "       $config"
Write-Host ''
Write-Host '  3. 关掉记事本再打开（已经在跑的进程手里攥着旧 DLL，注册新的对它没用），切到云朵输入法。'
Write-Host ''
Write-Host '  4. 敲这几组：'
Write-Host '       nihao  → 你好'
Write-Host '       rq     → 日期'
Write-Host '       v1+2   → 3'
Write-Host '       v      → 表达式模式（数字与运算符进算式）'
Write-Host '     再看候选右侧有没有译文小字、悬浮状态条第一格是不是「中」。'
Write-Host ''
$installedDll = Get-ChildItem -LiteralPath $InstallDir -Filter 'cloudime_tsf*.dll' -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
Write-Host '回滚：注销新 DLL，再注册装好的那个'
Write-Host "      regsvr32 /u `"$workDll`""
if ($installedDll) {
    Write-Host "      regsvr32 `"$installedDll`""
}
else {
    Write-Host "      （$InstallDir 下没找到 cloudime_tsf-*.dll，回滚时用安装包重装一遍）"
}

#regsvr32 "C:\OpenSource\cloudime\target\debug\cloudime_tsf.dll"