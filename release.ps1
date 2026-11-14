<#
  Builds and publishes a Bingo Hub release.

    .\release.ps1 1.1.0 "What changed"

  1. Sets the version in Cargo.toml and builds the exe.
  2. Makes dist\Bingo-Hub.exe (what installed hubs download to update themselves)
     and dist\Elden-Ring-Bingo-v<version>.zip (the full package for new people).
  3. Copies the new exe over your own "Bingo Hub.exe".
  4. Commits, pushes and creates the GitHub release (needs the GitHub CLI, `gh`).
     Add -NoPublish to stop after step 3.
#>
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Notes = "",
    [switch]$NoPublish
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3" }

$src   = $PSScriptRoot
$tools = Split-Path $src -Parent
$repo  = "$(Get-Content "$src\repo.txt" -ErrorAction SilentlyContinue | Select-Object -First 1)".Trim()
$dist  = Join-Path $src 'dist'
$stage = Join-Path $dist 'Elden Ring Bingo'
$zip   = Join-Path $dist "Elden-Ring-Bingo-v$Version.zip"

Write-Host "Building Bingo Hub $Version" -ForegroundColor Cyan
$toml = Get-Content "$src\Cargo.toml" -Raw
$toml = [regex]::Replace($toml, '(?m)^version = ".*?"', "version = `"$Version`"", 1)
Set-Content "$src\Cargo.toml" $toml -NoNewline -Encoding utf8
# Build output stays out of the repo.
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'bingo-hub-target' }
Push-Location $src
cargo build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "Build failed" }
Pop-Location
$exe = Join-Path $env:CARGO_TARGET_DIR 'release\bingo-hub.exe'

Write-Host "Packaging" -ForegroundColor Cyan
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item $exe "$stage\Bingo Hub.exe"
Copy-Item $exe "$dist\Bingo-Hub.exe"
Copy-Item "$src\package\README.txt" "$stage\README.txt"

# The original tools, minus logs, caches and randomizer output.
robocopy "$tools\Bingo Board" "$stage\Bingo Board" /E /NFL /NDL /NJH /NJS /NP | Out-Null
robocopy "$tools\Debug Tool" "$stage\Debug Tool" /E /NFL /NDL /NJH /NJS /NP | Out-Null
robocopy "$tools\Boards" "$stage\Boards" *.json /NFL /NDL /NJH /NJS /NP | Out-Null
robocopy "$tools\Bingo Randomizer" "$stage\Bingo Randomizer" /E /XD Cache /XF *.log regulation.bin *.dcx /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "Copying the tools failed" }
New-Item -ItemType Directory -Force "$stage\Bingo Randomizer\Resources\ME2\bingo\msg\engus" | Out-Null
# Read-only folders here make the Randomizer crash when it deletes its old output.
attrib -R "$stage\Bingo Randomizer\Resources\ME2\bingo" | Out-Null
attrib -R "$stage\Bingo Randomizer\Resources\ME2\bingo\*" /S /D | Out-Null

& "$env:SystemRoot\System32\tar.exe" -a -c -f $zip -C $dist 'Elden Ring Bingo'
if ($LASTEXITCODE -ne 0) { throw "Zipping failed" }
$mb = [math]::Round((Get-Item $zip).Length / 1MB)
Write-Host "  $zip ($mb MB)"

if (-not (Get-Process 'Bingo Hub' -ErrorAction SilentlyContinue)) {
    Copy-Item $exe "$tools\Bingo Hub.exe" -Force
    Write-Host "  Updated your own Bingo Hub.exe"
} else {
    Write-Host "  Bingo Hub is open, so your own copy wasn't replaced." -ForegroundColor Yellow
}

if ($NoPublish) { Write-Host "Done (not published)." -ForegroundColor Green; return }
if (-not $repo) { throw "Put your GitHub repo (owner/name) in repo.txt first." }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Write-Host "GitHub CLI not found. Publish by hand: create a release tagged v$Version on github.com/$repo and upload:" -ForegroundColor Yellow
    Write-Host "  $dist\Bingo-Hub.exe"
    Write-Host "  $zip"
    return
}

Write-Host "Publishing to github.com/$repo" -ForegroundColor Cyan
Push-Location $src
git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) { git commit -q -m "Release v$Version" }
git push -q origin HEAD
if (-not $Notes) { $Notes = "Bingo Hub $Version" }
# Notes go through a file: passing quotes straight to gh from PowerShell mangles them.
$notesFile = Join-Path $dist 'notes.md'
Set-Content $notesFile $Notes -Encoding utf8
gh release create "v$Version" "$dist\Bingo-Hub.exe" $zip --repo $repo --title "Bingo Hub $Version" --notes-file $notesFile
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "Creating the GitHub release failed" }
Pop-Location
Write-Host "Released. Installed hubs will offer the update within a few hours (or on next start)." -ForegroundColor Green
