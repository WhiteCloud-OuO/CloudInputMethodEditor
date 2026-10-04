<#
算 target\installer 下安装包的 SHA-256，把清单写到 target\installer\SHA256SUMS。
每行 `<hash>  <文件名>`，与 GNU sha256sum 一致，可用 `sha256sum -c SHA256SUMS` 复核。

用法：
  .\tools\release\gen-sha256.ps1

脚本存成带 BOM 的 UTF-8（Windows PowerShell 5.1 对无 BOM 的 .ps1 按系统 ANSI 解析，中文会乱码并报语法错）。
#>
[CmdletBinding()]
param(
    # 安装包所在目录；缺省 <仓库根>\target\installer。
    [string]$Dir = ''
)

$ErrorActionPreference = 'Stop'

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Dir) { $Dir = Join-Path $Repo 'target\installer' }
if (-not (Test-Path -LiteralPath $Dir)) { throw "目录不存在：$Dir（先打一次安装包，见 build-installer.ps1）" }

$installers = @(Get-ChildItem -LiteralPath $Dir -Filter '*-setup.exe' -File -ErrorAction SilentlyContinue)
if ($installers.Count -eq 0) { throw "$Dir 下没有安装包（*-setup.exe）" }

[string[]]$lines = @(
    foreach ($file in ($installers | Sort-Object Name)) {
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-Host ('{0}  {1}  ({2:N1} MB)' -f $hash, $file.Name, ($file.Length / 1MB))
        "$hash  $($file.Name)"
    }
)

$out = Join-Path $Dir 'SHA256SUMS'
# 不带 BOM：带 BOM 的话 sha256sum -c 认不出第一行。
[System.IO.File]::WriteAllLines($out, $lines, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "已写 $out（$($lines.Count) 个安装包）" -ForegroundColor Cyan
