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
| Bingo Randomizer 0.91 | Randomized gear from a shared seed |
| ModEngine 2 | Loading the randomized files |
| Elden Ring Debug Tool | Practice |

These tools belong to their authors and are included unchanged. The Bingo Randomizer and Debug Tool were originally made by Nordgaren, ModEngine 2 by the soulsmods team.

## Building

```
cargo build --release
```

`release.ps1 <version> "<notes>"` builds, packages and publishes a release.
