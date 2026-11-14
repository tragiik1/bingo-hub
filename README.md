# Bingo Hub

Everything for an Elden Ring bingo race in one download: the Bingo Board, the Bingo Randomizer, the Debug Tool and a small hub app that ties them together.

## Download

Get **Elden-Ring-Bingo-vX.Y.Z.zip** from the [latest release](../../releases/latest).

1. Extract it somewhere outside OneDrive, e.g. `C:\Games\Elden Ring Bingo`. OneDrive locks files and makes the Randomizer crash.
2. Open `Bingo Hub.exe`. If Windows SmartScreen appears, click **More info → Run anyway** (the app isn't code-signed).
3. Check the **Setup** page. Anything that needs fixing has a button.

The hub updates itself: when a new version is out it shows **Update available** in the sidebar.

## What the hub does

- **Race**: seed, then a checklist: open the Board, randomize (seed is copied for you), back up, launch with the bingo mod.
- **Saves**: automatic backups of `ER0000.sl2` before launching and after randomizing; restore any of them.
- **Boards**: goal lists, search, and random practice cards.
- **Run log**: results, opponents and seeds, with CSV export.
- **Setup**: checks the install (game path, Steam, runtimes, OneDrive, stray mod DLLs) and fixes what it can.

It also tells you if the game was started without the bingo mod.

## Included tools

These are made by their own authors and ship unchanged:

- **EldenBingo**: lobby board and map
- **Bingo Randomizer 0.91**: originally by Nordgaren
- **ModEngine 2**: by the soulsmods team
- **Elden Ring Debug Tool**: by Nordgaren, for practice only

## Building

Rust 1.88+ on Windows.

```
cargo build --release
```

Releasing (needs the GitHub CLI): put `owner/repo` in `repo.txt`, then

```
.\release.ps1 1.1.0 "What changed"
```
