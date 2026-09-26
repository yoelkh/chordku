# Publish a setlist page (downloaded from ChordKu: Pustaka > setlist > "Bagikan") to GitHub Pages.
#
#   tools\publish-setlist.ps1 "C:\Users\me\Downloads\ibadah-minggu.html"
#   tools\publish-setlist.ps1 ibadah-minggu.html -Slug minggu-28-sept   # choose the URL path
#   tools\publish-setlist.ps1 -Remove minggu-28-sept                     # take a page down
#
# Needs the GitHub CLI (`gh auth login` once). The first run creates the public repository
# <user>/chordku-setlists and turns on Pages; every run afterwards adds or replaces one page and
# prints its URL. Pages are public: they contain chord charts only, never audio.

param(
    [Parameter(Position = 0)] [string]$File,
    [string]$Slug,
    [string]$Repo = "chordku-setlists",
    [string]$Remove
)
$ErrorActionPreference = "Stop"

# Native git prints progress on stderr, which PowerShell would otherwise treat as an error.
$gitExe = (Get-Command git -CommandType Application | Select-Object -First 1).Source
function Git {
    $ErrorActionPreference = "Continue"  # git's warnings on stderr are not errors
    $out = & $gitExe @args 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') gagal:`n$($out -join "`n")" }
    $out
}

function Slugify([string]$s) {
    $t = ($s.ToLowerInvariant() -replace "[^a-z0-9]+", "-").Trim("-")
    if ($t) { $t } else { "setlist" }
}

$user = (gh api user --jq .login)
if (-not $user) { throw "gh belum login: jalankan `gh auth login` dulu." }
$full = "$user/$Repo"
$work = Join-Path $env:LOCALAPPDATA "ChordKu\pages"

# Repository + Pages, once.
$exists = $true
try { gh repo view $full --json name *> $null } catch { $exists = $false }
if (-not $exists) {
    Write-Host "Membuat repositori publik $full ..." -ForegroundColor Cyan
    gh repo create $full --public --description "Setlist chord dari ChordKu (halaman statis)" | Out-Null
}
if (-not (Test-Path (Join-Path $work ".git"))) {
    New-Item -ItemType Directory -Force (Split-Path $work) | Out-Null
    gh repo clone $full $work -- --quiet
}
Push-Location $work
try {
    $hasCommits = $true
    try { Git rev-parse --verify HEAD | Out-Null } catch { $hasCommits = $false }
    if ($hasCommits) { Git pull --quiet --rebase | Out-Null } else { Git checkout -q -B main | Out-Null }
    if ($Remove) {
        if (Test-Path $Remove) { Remove-Item -Recurse -Force $Remove -Confirm:$false }
    } else {
        if (-not $File) { throw "Sebutkan file HTML setlist (hasil unduhan dari ChordKu)." }
        if (-not (Test-Path $File)) { throw "File tidak ditemukan: $File" }
        if (-not $Slug) { $Slug = Slugify([IO.Path]::GetFileNameWithoutExtension($File)) }
        New-Item -ItemType Directory -Force $Slug | Out-Null
        Copy-Item $File (Join-Path $Slug "index.html") -Force
    }
    # Root page: a plain list of every published setlist.
    $dirs = Get-ChildItem -Directory | Where-Object { Test-Path (Join-Path $_.FullName "index.html") } | Sort-Object LastWriteTime -Descending
    $items = ($dirs | ForEach-Object {
        $title = $_.Name
        $m = Select-String -Path (Join-Path $_.FullName "index.html") -Pattern '"setlist":\{"id":\d+,"name":"([^"]*)"' | Select-Object -First 1
        if ($m) { $title = $m.Matches[0].Groups[1].Value }
        "      <li><a href=`"./$($_.Name)/`">$([Net.WebUtility]::HtmlEncode($title))</a><span>$($_.LastWriteTime.ToString('d MMM yyyy'))</span></li>"
    }) -join "`n"
    @"
<!doctype html>
<html lang="id"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="dark light"><title>Setlist ChordKu</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;background:#14161c;color:#e9ebf2}
@media(prefers-color-scheme:light){body{background:#fff;color:#1a1a1a}}
ul{list-style:none;padding:0}li{display:flex;justify-content:space-between;gap:1rem;padding:.7rem .9rem;border:1px solid #2f3444;border-radius:10px;margin:.5rem 0}
a{color:#f2b84b;text-decoration:none;font-weight:600}span{opacity:.6;font-size:.9em}</style></head>
<body><h1>Setlist</h1><ul>
$items
</ul><p><small>Dibuat dengan <a href="https://github.com/yoelkh/chordku">ChordKu</a>.</small></p></body></html>
"@ | Set-Content -Encoding utf8 index.html
    if (-not (Test-Path ".nojekyll")) { "" | Set-Content .nojekyll }
    Git add -A | Out-Null
    $msg = if ($Remove) { "Hapus $Remove" } else { "Terbitkan $Slug" }
    try { Git -c "user.name=$user" -c "user.email=$user@users.noreply.github.com" commit -q -m $msg | Out-Null } catch { Write-Host "Tidak ada perubahan." }
    Git push -q -u origin main | Out-Null
} finally { Pop-Location }

# Pages on, once (legacy build from the main branch root).
$pages = $null
try { $pages = gh api "repos/$full/pages" 2>$null | ConvertFrom-Json } catch {}
if (-not $pages) {
    gh api -X POST "repos/$full/pages" -f build_type=legacy -f "source[branch]=main" -f "source[path]=/" | Out-Null
    Write-Host "GitHub Pages diaktifkan. Halaman pertama biasanya siap dalam 1-2 menit." -ForegroundColor Cyan
}
$base = "https://$user.github.io/$Repo"
if ($Remove) { Write-Host "Dihapus: $base/$Remove/" -ForegroundColor Green }
else {
    Write-Host ""
    Write-Host "Tautan setlist:  $base/$Slug/" -ForegroundColor Green
    Write-Host "Daftar semua:    $base/"
    Write-Host "Kirim tautan itu lewat WhatsApp; perubahan berikutnya cukup jalankan perintah yang sama lagi."
}
