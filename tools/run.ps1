# Build (release) and start ChordKu, then open it in the default browser.
#   tools\run.ps1            normal start
#   tools\run.ps1 -NoOpen    do not open the browser

param([switch]$NoOpen)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path "web\dist\index.html")) {
    Push-Location web
    if (-not (Test-Path "node_modules")) { npm install --no-fund --no-audit }
    npm run build
    Pop-Location
}
if (-not (Test-Path "models\btc.onnx")) {
    Write-Warning "models\btc.onnx belum ada. Jalankan tools\setup.ps1 dulu (export model chord)."
}

cargo build --release -p chordku-server
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# The server looks for models in .\models (next to the working directory) and in its data folder.
$open = if ($NoOpen) { @() } else { @("--open") }
& "target\release\chordku.exe" @open
