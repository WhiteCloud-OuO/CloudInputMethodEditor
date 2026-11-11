<#
.SYNOPSIS
    给产物做 Authenticode 签名：默认用开发自签证书，也可以指定已装好的正式证书（Certum）。
.DESCRIPTION
    uiAccess=true 的 exe 只有在「代码签名 + 装在安全位置（Program Files）」时，系统才授予它
    升进 UIAccess 高 z-band 的权限（候选窗盖过微软商店 / 任务栏搜索）。开发全程用自签证书 +
    本机受信任根（0 成本，本机效果同真证书）；发版换 Certum 开源代码签名证书（signtool 同流程，只换指纹）。

    两种模式：
      - 不传 -Thumbprint：造 / 复用一张自签证书，把它装进本机「受信任的根」+「受信任的发布者」。
        开发期用，签完不加时间戳（免网络依赖，证书 5 年够用）。
      - 传 -Thumbprint：只找已装好的证书（CurrentUser\My 或 LocalMachine\My，要有私钥），
        不造、不信任任何东西；正式证书必须带时间戳（见 -TimestampUrl），否则证书到期后旧安装包的签名失效。

    签名后逐个核对 Get-AuthenticodeSignature 的状态，不是 Valid 直接失败——发版包签坏了要当场发现。
.PARAMETER Path
    要签名的文件（.exe / .dll），可多个。
.PARAMETER Thumbprint
    证书指纹（SHA1）。缺省空 = 走本机自签流程。
.PARAMETER CertSubject
    自签证书主题，缺省 "CN=CloudIME Dev CodeSign"（只在自签模式用）。
.PARAMETER TimestampUrl
    时间戳服务地址（RFC 3161）。正式证书请传，自签模式缺省不传。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Path,
    [string]$Thumbprint = '',
    [string]$CertSubject = 'CN=CloudIME Dev CodeSign',
    [string]$TimestampUrl = ''
)

$ErrorActionPreference = 'Stop'

$selfSigned = -not $Thumbprint
if ($selfSigned) {
    # 1) 找已有的自签代码签名证书（带私钥），没有就造一张（5 年，够开发用）。
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $CertSubject -and $_.HasPrivateKey } |
        Sort-Object NotAfter -Descending | Select-Object -First 1
    if (-not $cert) {
        Write-Host "造自签代码签名证书 $CertSubject…" -ForegroundColor Cyan
        $cert = New-SelfSignedCertificate `
            -Type CodeSigningCert `
            -Subject $CertSubject `
            -KeyUsage DigitalSignature `
            -KeySpec Signature `
            -CertStoreLocation Cert:\CurrentUser\My `
            -NotAfter (Get-Date).AddYears(5)
    }
    $thumbprint = $cert.Thumbprint
}
else {
    # 正式证书：只在两个存储里找，找不到就报错——绝不悄悄退回自签（那样发出去的东西没人认）。
    $cert = @('Cert:\CurrentUser\My', 'Cert:\LocalMachine\My') | ForEach-Object {
        Get-ChildItem $_ -ErrorAction SilentlyContinue |
            Where-Object { $_.Thumbprint -eq $Thumbprint -and $_.HasPrivateKey }
    } | Select-Object -First 1
    if (-not $cert) {
        throw "找不到指纹 $Thumbprint 的代码签名私钥（CurrentUser\My 或 LocalMachine\My）。CI 上先导入证书，见 docs/notes/release.md"
    }
    $thumbprint = $cert.Thumbprint
}
Write-Host "证书指纹 $thumbprint" -ForegroundColor Cyan

if ($selfSigned) {
    # 2) 把公钥装进「本机受信任的根」+「受信任的发布者」，uiAccess 的签名校验才认这张自签证书。
    #    已在就跳过（幂等）。
    foreach ($store in @('Root', 'TrustedPublisher')) {
        $storePath = "Cert:\LocalMachine\$store"
        $exists = Get-ChildItem $storePath -ErrorAction SilentlyContinue |
            Where-Object { $_.Thumbprint -eq $thumbprint }
        if (-not $exists) {
            Write-Host "装证书进 LocalMachine\$store…" -ForegroundColor Cyan
            # 只导公钥（.cer），不带私钥。
            $tmp = Join-Path $env:TEMP 'cloudime-dev-codesign.cer'
            Export-Certificate -Cert $cert -FilePath $tmp -Type CERT | Out-Null
            Import-Certificate -FilePath $tmp -CertStoreLocation $storePath | Out-Null
            Remove-Item $tmp -Force -ErrorAction SilentlyContinue
        }
    }
}
else {
    $stamp = if ($TimestampUrl) { $TimestampUrl } else { '无（正式证书建议加 -TimestampUrl）' }
    Write-Host "用已装好的证书签名；时间戳：$stamp" -ForegroundColor Cyan
}

# 3) 找 signtool：Windows SDK 里取版本最新的一个（CI runner 的 SDK 可能在 Program Files 或 Program Files (x86)），
#    再退到 PATH；都没有就用 PowerShell 的 Authenticode 签名（只能签自签，见下面 -TimestampUrl 的约束）。
$signtool = @(
    'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe',
    'C:\Program Files\Windows Kits\10\bin\*\x64\signtool.exe'
) | ForEach-Object { Get-ChildItem $_ -ErrorAction SilentlyContinue } |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1 }

foreach ($f in $Path) {
    if (-not (Test-Path -LiteralPath $f)) { throw "要签的文件不存在：$f" }
}

# 4) 签名（SHA256）。时间戳只在给了 -TimestampUrl 时加：自签不加（免网络依赖），正式证书必须加。
if ($signtool) {
    $signArgs = @('sign', '/sha1', $thumbprint, '/fd', 'sha256', '/v')
    if ($TimestampUrl) { $signArgs += @('/tr', $TimestampUrl, '/td', 'sha256') }
    $signArgs += $Path
    # PowerShell 5.1 会把 signtool 写进 stderr 的内容包装成错误记录，配合上面的 Stop 会打断签名。
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $signtool.FullName @signArgs 2>&1 | ForEach-Object { Write-Host $_ }
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($exitCode -ne 0) { throw "signtool 签名失败（退出码 $exitCode）" }
} else {
    if ($TimestampUrl) {
        throw '找不到 signtool.exe：带时间戳的签名必须有它（PowerShell 的 Set-AuthenticodeSignature 不做 RFC 3161 时间戳）'
    }
    Write-Host '没找到 signtool.exe，改用 PowerShell 的 Authenticode 签名（开发自签，无时间戳）…' -ForegroundColor Yellow
    foreach ($f in $Path) {
        $signature = Set-AuthenticodeSignature -FilePath $f -Certificate $cert -HashAlgorithm SHA256
        if ($signature.Status -ne 'Valid') {
            throw "Authenticode 签名失败：$f（$($signature.Status)）"
        }
    }
}

# 5) 逐个核对签名状态：自签要本机信任根装上才算 Valid，正式证书要整条链可信。签坏了当场失败，别带出去。
$bad = @()
foreach ($f in $Path) {
    $status = (Get-AuthenticodeSignature -FilePath $f).Status
    if ($status -ne 'Valid') { $bad += "$f（$status）" }
}
if ($bad.Count -gt 0) {
    throw "签名校验没通过（证书链没装好？）：$($bad -join '；')"
}
Write-Host "已签名并校验 $($Path.Count) 个文件。" -ForegroundColor Green
