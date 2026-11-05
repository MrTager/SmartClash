# 下载并安装 SmartClash 使用的三个代理内核：
#   resources\mihomo.exe   MetaCubeX/mihomo
#   resources\sing-box.exe SagerNet/sing-box
#   resources\xray.exe     XTLS/Xray-core（同时提供 geoip.dat / geosite.dat）
#
# 内核体积较大，不纳入 Git 仓库。首次克隆后运行本脚本；
# 也可以只用 -Source 系列参数指定本地已有的可执行文件，跳过下载。
#
#   .\fetch-core.ps1
#   .\fetch-core.ps1 -MihomoSource "D:\kernels\mihomo.exe" -SingBoxSource "D:\kernels\sing-box.exe" -XraySource "D:\kernels\xray.exe"
#   .\fetch-core.ps1 -GeoSource "D:\kernels"   # 目录内需包含 geoip.dat / geosite.dat

param(
    [string]$MihomoVersion = "v1.19.33",
    [string]$SingBoxVersion = "v1.12.10",
    [string]$XrayVersion = "v26.10.31",
    # 兼容旧参数：-Version / -Source 等同于 Mihomo 版本与本地文件。
    [string]$Version = "",
    [string]$Source = "",
    [string]$MihomoSource = "",
    [string]$SingBoxSource = "",
    [string]$XraySource = "",
    [string]$GeoSource = ""
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

if ($Version) { $MihomoVersion = $Version }
if ($Source) { $MihomoSource = $Source }

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Resources = Join-Path $Root "resources"
New-Item -ItemType Directory -Path $Resources -Force | Out-Null

function Invoke-Download([string]$Url, [string]$OutFile) {
    # GitHub 直连优先；不通时依次尝试国内加速镜像。
    foreach ($prefix in @("", "https://ghfast.top/", "https://gh-proxy.com/")) {
        try {
            $full = "$prefix$Url"
            Write-Host "下载：$full"
            Invoke-WebRequest -Uri $full -OutFile $OutFile -UseBasicParsing -TimeoutSec 180
            if ((Test-Path $OutFile) -and (Get-Item $OutFile).Length -gt 0) {
                return
            }
        } catch {
            Write-Warning "下载失败：$($_.Exception.Message)"
        }
    }
    throw "无法下载：$Url"
}

function Expand-Kernel([string]$ZipPath, [string]$WorkDir, [string]$Pattern) {
    New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
    Expand-Archive -Path $ZipPath -DestinationPath $WorkDir -Force
    $file = Get-ChildItem $WorkDir -Recurse -File -Filter $Pattern | Select-Object -First 1
    if (-not $file) { throw "压缩包内未找到 $Pattern" }
    return $file
}

function Copy-Local([string]$From, [string]$To) {
    # 允许 -XxxSource 直接指向 resources 内已有文件（脚本可重复运行）。
    $src = (Resolve-Path $From).Path
    $dst = Join-Path (Resolve-Path (Split-Path -Parent $To)).Path (Split-Path -Leaf $To)
    if ($src -ne $dst) {
        Copy-Item $src $To -Force
    }
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("smartclash-core-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

try {
    # ---- Mihomo ---------------------------------------------------------
    $mihomoDest = Join-Path $Resources "mihomo.exe"
    if ($MihomoSource) {
        Copy-Local $MihomoSource $mihomoDest
    } else {
        $mihomoZip = Join-Path $tmp "mihomo.zip"
        Invoke-Download "https://github.com/MetaCubeX/mihomo/releases/download/$MihomoVersion/mihomo-windows-amd64-$MihomoVersion.zip" $mihomoZip
        $mihomoExe = Expand-Kernel $mihomoZip (Join-Path $tmp "mihomo") "mihomo*.exe"
        Copy-Item $mihomoExe.FullName $mihomoDest -Force
    }
    & $mihomoDest -v

    # ---- sing-box -------------------------------------------------------
    $singboxDest = Join-Path $Resources "sing-box.exe"
    if ($SingBoxSource) {
        Copy-Local $SingBoxSource $singboxDest
    } else {
        $singboxZip = Join-Path $tmp "singbox.zip"
        Invoke-Download "https://github.com/SagerNet/sing-box/releases/download/$SingBoxVersion/sing-box-$($SingBoxVersion.TrimStart('v'))-windows-amd64.zip" $singboxZip
        $singboxExe = Expand-Kernel $singboxZip (Join-Path $tmp "singbox") "sing-box.exe"
        Copy-Item $singboxExe.FullName $singboxDest -Force
    }
    & $singboxDest version

    # ---- Xray + geo 数据 ------------------------------------------------
    $xrayDest = Join-Path $Resources "xray.exe"
    $geoIpDest = Join-Path $Resources "geoip.dat"
    $geoDest = Join-Path $Resources "geosite.dat"
    $geoFileRoot = $null
    if ($XraySource) {
        Copy-Local $XraySource $xrayDest
        $geoFileRoot = Split-Path -Parent (Resolve-Path $XraySource)
    } else {
        $xrayZip = Join-Path $tmp "xray.zip"
        Invoke-Download "https://github.com/XTLS/Xray-core/releases/download/$XrayVersion/Xray-windows-64.zip" $xrayZip
        $xrayExe = Expand-Kernel $xrayZip (Join-Path $tmp "xray") "xray.exe"
        Copy-Item $xrayExe.FullName $xrayDest -Force
        $geoFileRoot = $xrayExe.DirectoryName
    }
    & $xrayDest version

    if ($GeoSource) {
        Copy-Local (Join-Path $GeoSource "geoip.dat") $geoIpDest
        Copy-Local (Join-Path $GeoSource "geosite.dat") $geoDest
    } else {
        $geoIp = Get-ChildItem $geoFileRoot -Recurse -File -Filter "geoip.dat" -ErrorAction SilentlyContinue | Select-Object -First 1
        $geoSite = Get-ChildItem $geoFileRoot -Recurse -File -Filter "geosite.dat" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($geoIp) { Copy-Local $geoIp.FullName $geoIpDest }
        if ($geoSite) { Copy-Local $geoSite.FullName $geoDest }
    }
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

foreach ($file in @("mihomo.exe", "sing-box.exe", "xray.exe", "geoip.dat", "geosite.dat")) {
    $path = Join-Path $Resources $file
    if (-not (Test-Path $path)) {
        throw "缺少 resources\$file；Xray 需要 geoip.dat / geosite.dat，请用 -GeoSource 指定。"
    }
}

Write-Host ""
Write-Host "三个内核与 geo 数据已就位："
Get-ChildItem $Resources | Where-Object { $_.Name -in @("mihomo.exe", "sing-box.exe", "xray.exe", "geoip.dat", "geosite.dat") } |
    ForEach-Object { Write-Host ("  {0}  {1:N1} MB" -f $_.Name, ($_.Length / 1MB)) }
