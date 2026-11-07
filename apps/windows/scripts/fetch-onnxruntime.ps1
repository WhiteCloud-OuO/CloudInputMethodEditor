# 取知意（ONNX）要的运行库到 target\installer\onnxruntime\：onnxruntime.dll 与许可。
# ONNX Runtime 1.24.4 取自 NuGet 的 Microsoft.ML.OnnxRuntime（CPU 版），按包的 SHA-256 核对，已取过就跳过。
# 打包脚本与 reconfigure.ps1 都从这里取；Server 按 exe 旁边的 onnxruntime.dll 动态加载。
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Version = '1.24.4'
$StampValue = "$Version-cpu"
$Packages = @(
    @{
        Url    = "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/$Version/microsoft.ml.onnxruntime.$Version.nupkg"
        Sha256 = '4b978d5065b85e7004b6c6f60ca494bd978fbe6836cbf0a0b52d82b61ab99638'
        Files  = @{
            'runtimes/win-x64/native/onnxruntime.dll' = 'onnxruntime.dll'
            'LICENSE'                                 = 'onnxruntime-LICENSE.txt'
            'ThirdPartyNotices.txt'                   = 'onnxruntime-ThirdPartyNotices.txt'
        }
    }
)

$RepoRoot = $PSScriptRoot
while ($RepoRoot) {
    if ((Test-Path -LiteralPath (Join-Path $RepoRoot 'version.txt')) -and
        (Test-Path -LiteralPath (Join-Path $RepoRoot 'apps\windows'))) { break }
    $parent = Split-Path -Parent $RepoRoot
    if (-not $parent -or $parent -eq $RepoRoot) { throw "找不到仓库根：$PSScriptRoot" }
    $RepoRoot = $parent
}

$Dest = Join-Path $RepoRoot 'target\installer\onnxruntime'
$Stamp = Join-Path $Dest 'VERSION'
if ((Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $StampValue)) {
    Write-Host "ONNX Runtime $Version 已在 $Dest" -ForegroundColor Cyan
    return
}
if (Test-Path $Dest) { Remove-Item $Dest -Recurse -Force }
New-Item -ItemType Directory -Path $Dest -Force | Out-Null

Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($package in $Packages) {
    $download = Join-Path $env:TEMP ([IO.Path]::GetFileName($package.Url))
    Write-Host "下载 $($package.Url)" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $package.Url -OutFile $download -UseBasicParsing
    $hash = (Get-FileHash $download -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $package.Sha256) {
        Remove-Item $download -Force
        throw "$download 的 SHA-256 是 $hash，应为 $($package.Sha256)"
    }
    $zip = [IO.Compression.ZipFile]::OpenRead($download)
    try {
        foreach ($entry in $package.Files.Keys) {
            $found = $zip.Entries | Where-Object { $_.FullName -eq $entry } | Select-Object -First 1
            if (-not $found) { throw "$download 里没有 $entry" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($found, (Join-Path $Dest $package.Files[$entry]), $true)
        }
    } finally {
        $zip.Dispose()
        Remove-Item $download -Force
    }
}
Set-Content -Path $Stamp -Value $StampValue -Encoding ASCII
Write-Host "ONNX Runtime $Version → $Dest" -ForegroundColor Green
