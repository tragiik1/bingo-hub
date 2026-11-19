<#
  Adds the weapon names Bingo Hub found missing to the installed Randomizer.

    .\patch.ps1

  Bingo Hub writes the list to Data\randomizer-missing-names.tsv when Setup shows
  "The Randomizer doesn't know some new weapons". This patches those names into the installed
  S2_Style_1.0.exe (names only feed the class descriptions, so seeds randomize exactly as before),
  keeps the previous exe in Data\Randomizer backup\<date>, and installs the new one. Release a hub
  version afterwards so everyone gets the same build.

  Needs Python 3 and the .NET SDK.
#>
$ErrorActionPreference = 'Stop'
$here  = $PSScriptRoot
$root  = Split-Path (Split-Path (Split-Path $here -Parent) -Parent) -Parent
$exe   = Join-Path $root 'Bingo Randomizer\S2_Style_1.0.exe'
$list  = Join-Path $root 'Data\randomizer-missing-names.tsv'
$work  = Join-Path $env:TEMP "randomizer-patch-$(Get-Date -Format yyyyMMddHHmmss)"

if (-not (Test-Path $list) -or -not (Get-Content $list)) { throw "No missing names listed. Open Bingo Hub's Setup page first; if it shows no warning, nothing needs patching." }
if (Get-Process S2_Style_1.0 -ErrorAction SilentlyContinue) { throw "Close the Randomizer first." }
New-Item -ItemType Directory "$work\extracted" | Out-Null

Write-Host "Extracting Randomizer.dll" -ForegroundColor Cyan
python -I "$here\unbundle.py" $exe "$work\extracted" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Extracting failed" }

Write-Host "Adding $((Get-Content $list).Count) names" -ForegroundColor Cyan
dotnet run --project "$here\patcher" -c Release -- "$work\extracted\Randomizer.dll" "$work\Randomizer.dll" $list
if ($LASTEXITCODE -ne 0) { throw "Patching failed" }

python -I "$here\repack.py" $exe Randomizer.dll "$work\Randomizer.dll" "$work\S2_Style_1.0.exe"
if ($LASTEXITCODE -ne 0) { throw "Repacking failed" }

$bak = Join-Path $root "Data\Randomizer backup\$(Get-Date -Format yyyy-MM-dd-HHmm)"
New-Item -ItemType Directory $bak -Force | Out-Null
Copy-Item $exe $bak
Copy-Item "$work\S2_Style_1.0.exe" $exe -Force
Remove-Item $list
Remove-Item $work -Recurse -Force
Write-Host "Done. The previous Randomizer is in $bak. Reopen Bingo Hub's Setup page to confirm the warning is gone." -ForegroundColor Green
