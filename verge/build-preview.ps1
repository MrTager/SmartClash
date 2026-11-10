param(
    [string]$Toolchain = '1.99.1',
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    pnpm web:build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    cargo "+$Toolchain" build -p clash-verge --locked --profile fast-release --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed; run pnpm prebuild for this target first' }
    $binaryRoot = Join-Path $PSScriptRoot "target/$Target/fast-release"
    # Vite empties verge/dist during every web build.
    $previewRoot = Join-Path (Split-Path $PSScriptRoot -Parent) 'dist/SmartClash-Verge-Preview'
    New-Item -ItemType Directory -Path $previewRoot -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $binaryRoot 'clash-verge.exe') -Destination (Join-Path $previewRoot 'SmartClash.exe')
    foreach ($core in @('verge-mihomo', 'verge-mihomo-alpha')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot "src-tauri/sidecar/$core-$Target.exe") -Destination (Join-Path $previewRoot "$core.exe")
    }
    $webviewLoader = Join-Path $binaryRoot 'WebView2Loader.dll'
    if (Test-Path -LiteralPath $webviewLoader) { Copy-Item -LiteralPath $webviewLoader -Destination $previewRoot }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'src-tauri/resources') -Destination $previewRoot -Recurse -Force
    Copy-Item -LiteralPath 'LICENSE', 'SMARTCLASH.md', 'UPSTREAM.json', '../THIRD_PARTY_NOTICES.md' -Destination $previewRoot
    Write-Output "Preview ready: $previewRoot/SmartClash.exe"
} finally {
    Pop-Location
}
