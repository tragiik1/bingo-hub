<#
  Rebuilds assets\items.tsv, assets\weapons.tsv and assets\icons.bin (the Classes page's weapon
  types, Somber/Smithing tags, attack ratings, skills and icons) from the installed game. Run it
  after a game patch changes weapons or spells, then rebuild the hub.

    .\make.ps1                       finds the game the same way the hub does (Steam)
    .\make.ps1 -Game 'D:\...\Game'   or point it at the Game folder

  icons.bin is game art, so it stays out of git (see .gitignore); release builds embed it.
  Needs Python 3 and the .NET SDK.
#>
param([string]$Game = '')
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$src  = Split-Path (Split-Path $here -Parent) -Parent
$root = Split-Path $src -Parent
if (-not $Game) {
  $steam = (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Valve\Steam').InstallPath
  $libs = @($steam) + (Select-String -Path "$steam\steamapps\libraryfolders.vdf" -Pattern '"path"\s+"([^"]+)"' | ForEach-Object { $_.Matches[0].Groups[1].Value -replace '\\\\', '\' })
  $Game = $libs | ForEach-Object { Join-Path $_ 'steamapps\common\ELDEN RING\Game' } | Where-Object { Test-Path "$_\eldenring.exe" } | Select-Object -First 1
  if (-not $Game) { throw "Couldn't find the game. Pass -Game 'path\to\ELDEN RING\Game'." }
}

if (-not (Test-Path "$here\extracted\Randomizer.dll")) {
  python -I "$src\tools\randomizer-patch\unbundle.py" (Join-Path $root 'Bingo Randomizer\S2_Style_1.0.exe') "$here\extracted" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "Extracting the Randomizer failed" }
}
dotnet build "$here" -c Release -v q
if ($LASTEXITCODE -ne 0) { throw "Build failed" }
$bin = "$here\bin\Release\net9.0-windows"
Copy-Item "$here\extracted\*.dll" $bin -Force
# The Randomizer reads its param layouts from Resources next to the exe.
Copy-Item (Join-Path $root 'Bingo Randomizer\Resources') $bin -Recurse -Force
Push-Location $bin   # it looks for Resources (and writes its Cache) in the current folder
try { & .\item-icons.exe $Game "$src\assets" 96 pal } finally { Pop-Location }
if ($LASTEXITCODE -ne 0) { throw "Extracting icons failed" }
Write-Host "Done. Rebuild the hub to embed the new assets." -ForegroundColor Green
