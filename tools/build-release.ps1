# Assemble a portable folder: dist\ChordKu\{chordku.exe, chordku-cli.exe, models\, README}.
# ONNX Runtime is linked statically, so the exe is self-contained. Only the MIT-licensed
# BTC export is bundled; the other models are fetched from their publishers by the app.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Push-Location web
if (-not (Test-Path "node_modules")) { npm install --no-fund --no-audit }
npm run build
Pop-Location

cargo build --release -p chordku-server -p chordku-cli
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$out = Join-Path $root "dist\ChordKu"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $out "models") | Out-Null
Copy-Item "target\release\chordku.exe", "target\release\chordku-cli.exe" $out
# GPU back ends live in provider DLLs next to the exe (see docs\ACCELERATION.md).
foreach ($dll in "onnxruntime_providers_shared.dll", "onnxruntime_providers_cuda.dll", "DirectML.dll") {
    if (Test-Path "target\release\$dll") { Copy-Item "target\release\$dll" $out }
}
foreach ($f in "btc.onnx", "btc.json") {
    $src = Join-Path $root "models\$f"
    if (Test-Path $src) { Copy-Item $src (Join-Path $out "models") } else { Write-Warning "models\$f missing: run tools\setup.ps1" }
}
Copy-Item "README.md", "README.id.md", "LICENSE", "THIRD_PARTY.md" $out -ErrorAction SilentlyContinue

$size = "{0:N0} MB" -f ((Get-ChildItem -Recurse $out | Measure-Object Length -Sum).Sum / 1MB)
Write-Host "dist\ChordKu siap ($size). Jalankan chordku.exe --open" -ForegroundColor Green
