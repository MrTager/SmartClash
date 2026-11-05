$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
# 版本号从 Cargo.toml 读取，避免与实际编译版本不一致。
$CargoToml = Get-Content (Join-Path $Root "Cargo.toml") -Raw
if ($CargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "无法从 Cargo.toml 解析版本号" }
$Version = $Matches[1]
$DistRoot = Join-Path $Root "dist"
$Dist = Join-Path $DistRoot "SmartClash-Windows-$Version"
$Zip = Join-Path $DistRoot "SmartClash-Windows-$Version.zip"

function Find-Cargo {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargo) { return $cargo.Source }
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) { return $fallback }
    throw "未找到 cargo。请先安装 Rust（x86_64-pc-windows-gnu 工具链）。"
}

$Cargo = Find-Cargo
Write-Host "使用 cargo: $Cargo"

$Kernels = @("mihomo.exe", "sing-box.exe", "xray.exe", "geoip.dat", "geosite.dat")
foreach ($file in ($Kernels + @("wintun.dll"))) {
    $path = Join-Path $Root "resources\$file"
    if (-not (Test-Path $path)) {
        if ($Kernels -contains $file) {
            throw "缺少 resources\$file。内核与 geo 数据不入库，请先运行 .\fetch-core.ps1 下载（可用 -Source 系列参数指定本地文件）。"
        }
        throw "缺少资源文件 resources\$file，请先放入 wintun.dll。"
    }
}

& $Cargo build --release --manifest-path (Join-Path $Root "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "编译失败。" }

$exe = Join-Path $Root "target\release\SmartClash.exe"
if (-not (Test-Path $exe)) { throw "未生成 SmartClash.exe。" }

if (Test-Path $Dist) { Remove-Item -Recurse -Force $Dist }
New-Item -ItemType Directory -Path (Join-Path $Dist "resources") -Force | Out-Null
Copy-Item $exe (Join-Path $Dist "SmartClash.exe")
foreach ($file in ($Kernels + @("wintun.dll"))) {
    Copy-Item (Join-Path $Root "resources\$file") (Join-Path $Dist "resources\$file")
}
Copy-Item (Join-Path $Root "README.md") (Join-Path $Dist "README.md")
Copy-Item (Join-Path $Root "..\LICENSE") (Join-Path $Dist "LICENSE")
Copy-Item (Join-Path $Root "..\THIRD_PARTY_NOTICES.md") (Join-Path $Dist "THIRD_PARTY_NOTICES.md")

if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path (Join-Path $Dist "*") -DestinationPath $Zip -Force

Write-Host ""
Write-Host "打包完成："
Write-Host "  目录: $Dist"
Write-Host "  压缩包: $Zip"
