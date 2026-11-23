<#
  Patches the installed Randomizer.

    .\patch.ps1             add the weapon names Bingo Hub found missing
    .\patch.ps1 -Stones 1   set the Smithing Stone cost per upgrade level (S2 ships with 2)

  Names: Bingo Hub writes the list to Data\randomizer-missing-names.tsv when Setup shows
  "The Randomizer doesn't know some new weapons". Names only feed the class descriptions, so seeds
  randomize exactly as before.

  Stones: changes the one constant in the Randomizer's own patchSmithingStones step (regular
  Smithing Stones [1]-[8]; Somber stones already cost 1). Nothing else a seed writes changes.

  Both can run together. The previous exe is kept in Data\Randomizer backup\<date>. Release a hub
  version afterwards so everyone gets the same build (the race code includes the Randomizer's
  fingerprint, and until then the hub offers the released build as an "update").

  Needs Python 3 and the .NET SDK.
#>
param([int]$Stones = 0)
$ErrorActionPreference = 'Stop'
$here  = $PSScriptRoot
$root  = Split-Path (Split-Path (Split-Path $here -Parent) -Parent) -Parent
$exe   = Join-Path $root 'Bingo Randomizer\S2_Style_1.0.exe'
$list  = Join-Path $root 'Data\randomizer-missing-names.tsv'
$work  = Join-Path $env:TEMP "randomizer-patch-$(Get-Date -Format yyyyMMddHHmmss)"

$haveNames = (Test-Path $list) -and (Get-Content $list)
if (-not $haveNames -and $Stones -eq 0) { throw "Nothing to do. No missing names are listed (open Bingo Hub's Setup page first; if it shows no warning, none are missing), and -Stones wasn't given." }
if (Get-Process S2_Style_1.0 -ErrorAction SilentlyContinue) { throw "Close the Randomizer first." }
New-Item -ItemType Directory "$work\extracted" | Out-Null

Write-Host "Extracting Randomizer.dll" -ForegroundColor Cyan
python -I "$here\unbundle.py" $exe "$work\extracted" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Extracting failed" }

$patchArgs = @("$work\extracted\Randomizer.dll", "$work\Randomizer.dll")
if ($haveNames) { Write-Host "Adding $((Get-Content $list).Count) names" -ForegroundColor Cyan; $patchArgs += $list } else { $patchArgs += '-' }
if ($Stones -gt 0) { Write-Host "Setting Smithing Stones to $Stones per level" -ForegroundColor Cyan; $patchArgs += @('--stones', $Stones) }
dotnet run --project "$here\patcher" -c Release -- @patchArgs
if ($LASTEXITCODE -ne 0) { throw "Patching failed" }

python -I "$here\repack.py" $exe Randomizer.dll "$work\Randomizer.dll" "$work\S2_Style_1.0.exe"
if ($LASTEXITCODE -ne 0) { throw "Repacking failed" }

$bak = Join-Path $root "Data\Randomizer backup\$(Get-Date -Format yyyy-MM-dd-HHmm)"
New-Item -ItemType Directory $bak -Force | Out-Null
Copy-Item $exe $bak
Copy-Item "$work\S2_Style_1.0.exe" $exe -Force
if ($haveNames) { Remove-Item $list }
Remove-Item $work -Recurse -Force
Write-Host "Done. The previous Randomizer is in $bak." -ForegroundColor Green
