# Downloads the FFmpeg build Lumen links against (the same pinned LGPL shared build as rimelight-space's
# `cargo xtask ffmpeg`), checks its SHA-256 and unpacks it to app/ffmpeg/current. Run once per machine:
#   powershell -ExecutionPolicy Bypass -File tools/get-ffmpeg.ps1
$ErrorActionPreference = 'Stop'
$Release = 'autobuild-2026-11-19-12-39'
$Asset = 'ffmpeg-n9.0.2-58-g8c7ef49002-win64-lgpl-shared-9.0.zip'
$Sha256 = '1fdb837def412ad7cc019874b6310156f40179bd30df58d041a82413f800a331'

$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root 'ffmpeg'
$current = Join-Path $dest 'current'
if (Test-Path (Join-Path $current 'lib')) {
    if ((Get-Content (Join-Path $current 'PINNED_RELEASE.txt') -ErrorAction SilentlyContinue) -eq "$Release/$Asset") {
        Write-Host "FFmpeg is already set up in $current"
        exit 0
    }
}
New-Item -ItemType Directory -Force $dest | Out-Null
$zip = Join-Path $dest $Asset
if (-not (Test-Path $zip)) {
    $url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$Release/$Asset"
    Write-Host "Downloading $url"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
}
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
if ($hash -ne $Sha256) {
    Remove-Item $zip
    throw "$Asset has SHA-256 $hash, expected $Sha256; the file was removed"
}
$tmp = Join-Path $dest 'unpack'
if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
Expand-Archive -LiteralPath $zip -DestinationPath $tmp
$inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
if (Test-Path $current) { Remove-Item -Recurse -Force $current }
Move-Item $inner.FullName $current
Remove-Item -Recurse -Force $tmp
Remove-Item $zip
Set-Content -Path (Join-Path $current 'PINNED_RELEASE.txt') -Value "$Release/$Asset"
Write-Host "FFmpeg is set up in $current"
