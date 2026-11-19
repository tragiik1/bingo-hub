# Bingo Hub

Elden Ring bingo in one download: the bingo board, the randomizer, a practice tool, and a small app that runs them.

## Install

Download `Elden-Ring-Bingo-vX.Y.Z.zip` from [Releases](../../releases/latest), extract it, and open `Bingo Hub.exe`.

Windows may say it protected your PC because the app isn't signed. Click **More info**, then **Run anyway**.

The app updates itself.

## Included

| Tool | Used for |
| --- | --- |
| EldenBingo | Bingo board and map |
| Bingo Randomizer (S2 Style 1.0) | Randomized gear from a shared seed |
| ModEngine 2 | Loading the randomized files |
| Elden Ring Debug Tool | Practice |

These tools belong to their authors and are included unchanged. The Bingo Randomizer and Debug Tool were originally made by Nordgaren, ModEngine 2 by the soulsmods team. EldenBingo and the Debug Tool are GPL-3.0, ERBingoRandomizer and ModEngine 2 are MIT; licence texts and source links are in [package/LICENSES.txt](package/LICENSES.txt).

## Building

```
cargo build --release
```

`release.ps1 <version> "<notes>"` builds, packages and publishes a release.
