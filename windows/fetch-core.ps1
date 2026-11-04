# 下载 Mihomo Windows amd64 内核到 resources\mihomo.exe
#
# 内核二进制体积约 60 MB，不纳入 Git 仓库（与 macOS 版的 Resources/mihomo 约定一致）。
# 首次克隆后运行本脚本；也可用 -Source 指定本地已有的 mihomo.exe。
#
#   .\fetch-core.ps1
#   .\fetch-core.ps1 -Source "D:\path\to\mihomo.exe"

param(
    [string]$Version = "v1.19.33",
    [string]$Source = ""
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Resources = Join-Path $Root "resources"
$Dest = Join-Path $Resources "mihomo.exe"
New-Item -ItemType Directory -Path $Resources -Force | Out-Null

if ($Source) {
    Copy-Item $Source $Dest -Force
    & $Dest -v
    Write-Host "已从本地复制内核：$Dest"
    exit 0
}

$Api = "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/$Version"
Write-Host "查询 Mihomo 发布信息：$Api"
$release = Invoke-RestMethod -Uri $Api -Headers @{ "User-Agent" = "SmartClash-Windows" }

$asset = $release.assets | Where-Object { $_.name -eq "mihomo-windows-amd64-$Version.zip" } | Select-Object -First 1
if (-not $asset) {
    $names = ($release.assets | Where-Object { $_.name -like "*windows-amd64*" } | ForEach-Object { $_.name }) -join ", "
    throw "未找到 mihomo-windows-amd64-$Version.zip。可用资产：$names"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("smartclash-core-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
    $zip = Join-Path $tmp "core.zip"
    Write-Host "下载：$($asset.name) ($([math]::Round($asset.size / 1MB, 1)) MB)"
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing

    if ($asset.digest -and $asset.digest.StartsWith("sha256:")) {
        $expected = $asset.digest.Substring(7)
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($actual -ne $expected.ToLower()) {
            throw "SHA-256 校验失败：期望 $expected，实际 $actual"
        }
        Write-Host "SHA-256 校验通过"
    }

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $exe = Get-ChildItem $tmp -Recurse -Filter "mihomo*.exe" | Select-Object -First 1
    if (-not $exe) { throw "压缩包内未找到 mihomo 可执行文件。" }
    Copy-Item $exe.FullName $Dest -Force
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

& $Dest -v
Write-Host "内核已就位：$Dest"
