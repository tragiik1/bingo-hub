# Changes: race graces on any save, hotkey, backups on every game start

Context for whoever works on this next. These changes were made on top of v1.0.8 and are **not
released yet**: they're on `main` for testing, and the live version is still v1.0.8. The exact
v1.0.8 state is kept as branch `backup/v1.0.8-working` and tag `v1.0.8-working-backup`.

## Background from the owner (why any of this was changed)

- The pre-set race graces feature **wasn't working for some people**.
- **Most friends don't like the separate bingo save** and use their own save. Graces were
  deliberately limited to the separate save, so on their own save the feature never ran. They
  went back to the **Cheat Engine table**, which works but is tedious: open CE, attach to the
  game, expand a huge table, find the event flag script, and run it.
- **When CE works:** after creating the character, on the screen where you name it and pick the
  keepsake, right before pressing **Finish**. Running the event flag script there means the
  graces are already lit when you load in. The owner asked for a reliable manual backup that
  does the same thing, usable on that screen or mid-game, plus a better automatic solution.
- **The original safety feature still matters:** it exists so the hub can never overwrite the
  graces of someone's real, non-bingo playthrough. Any solution has to keep that guarantee.
- The owner asked for meaningful changes only, with no bloat, and wants them assessed before
  they're released.

## The problem

Race graces (the 11 standard graces, with every other grace cleared, done by writing the game's
event flags in `src/graces.rs`) only worked for some people. The cause was the safety gate:

- Automatic setting only ran when the **separate bingo save** was active
  (`bingosave::active()`). Most of the group didn't like the separate save and played on their
  own save, so the worker just reported `"nosave"` and never applied anything.
- The separate save is only swapped in by the hub's own Launch button. Launching with the
  Randomizer's **Bingo!** button runs the same ModEngine 2 command but skips the swap, so even
  people with the setting on often ended up on their normal save.
- The manual "Apply now" button refused to run off the bingo save too, so there was no fallback.
  People went back to the Cheat Engine table, running the event flag script on the name screen
  (name / keepsake, right before Finish).

The gate existed so a non-bingo playthrough can never be wiped. That still matters, but the save
file was the wrong thing to check. The character is what matters.

## The new safety model

The check is now "is this character new?", not "which save file is this?". Everything still
requires a **bingo launch** (`GameState::Modded`: ModEngine 2 loaded, EAC off), as before.

| Path | Applies when | Over the limit |
| --- | --- | --- |
| Automatic, separate bingo save active | 40 or fewer graces lit (`FRESH_LIMIT`, unchanged) | Left alone |
| Automatic, any other save | 3 or fewer graces lit (`NEW_LIMIT`, new) | Left alone |
| Set graces button / Ctrl+F9 | 40 or fewer graces lit | Asks for confirmation, then forces |

Why `NEW_LIMIT = 3`: a brand-new character (name screen onwards) has no graces lit, so it passes.
Any real playthrough has more than the 1–3 tutorial graces, so it's never touched automatically.
In the worst case a misfire touches a character with at most 3 graces. The limit is 3 rather than
0 in case the game sets a default flag at creation.

Automatic setting re-checks every 2 s (was 3 s) while a bingo game runs. That catches the name
screen, which is where the CE script worked, and otherwise the first moments after Finish.
Writes are only reported as done if all 11 flags read back as set (`Outcome::WriteFailed`
otherwise; before, failed writes were silently reported as success).

## What changed, by file

- `src/graces.rs`
  - `NEW_LIMIT` added. `Game::apply(limit: Option<usize>)` takes the limit (`None` = force).
  - Verifies the writes (`Outcome::WriteFailed`).
  - New `Status` enum (`Idle / Waiting / Set / Skipped(n) / Error`) replaces the stringly-typed
    `"set"`, `"skipped:N"`, `"error:..."`, `"nosave"` status that went to the UI.
  - `apply_now(pid, force) -> Result<Outcome, String>`: the UI and hotkey format the result.
  - If `OpenProcess` fails with access denied, the error now says to run the hub as administrator
    when Steam or the game runs elevated. That's a likely cause of "didn't work for me".
- `src/sys.rs`
  - `hotkey_loop`: global **Ctrl+F9** via `RegisterHotKey` on its own thread with a thread timer
    (no window). It's **only registered while a bingo game runs**, so it doesn't take Ctrl+F9
    from other programs the rest of the time. State: `sys::hotkey::{OFF, ON, TAKEN}`.
  - `chime(ok)`: Windows asterisk / error sound, so results are heard while the game is fullscreen.
- `src/worker.rs`
  - Hotkey thread: on press, applies to the running modded game, plays the sound, and sends
    `Msg::Graces(pid, result)`. The work happens on that thread on purpose. egui may not repaint
    a window hidden behind a fullscreen game, so it can't depend on the UI thread.
  - Automatic setting uses `FRESH_LIMIT` on the bingo save and `NEW_LIMIT` elsewhere, and plays
    the sound when it applies.
  - **Backup on every game start**: when a new `eldenring.exe` appears (under 60 s old, so not
    when the hub is opened mid-session), an auto backup "game started" is made, unless the
    separate bingo save is active. This covers Randomizer launches, which previously got no
    backup. With hub launches it's identical to the "before launch" backup, and `create_backup`
    already skips identical auto backups, so there's no duplicate.
  - `Snapshot.hotkey` carries the hotkey state so the UI repaints when it changes.
- `src/app.rs`
  - Race page: **Set graces** button next to Show while a bingo game runs. The status line
    shows "race graces set" or "Ctrl+F9 in game sets race graces".
  - `graces_result` handles button and hotkey results: toast on success. A character with many
    graces gets a confirm (`Confirmed::ForceGraces(pid)`, and it checks the same game is still
    running). Other failures get an alert.
  - Setup: status texts for the new `Status`, a Hotkey row (warns if another program has
    Ctrl+F9). The toggle now reads "Set race graces on new bingo characters". The backup toggle
    is now "Back up save when the game starts" (same `AutoBackupOnLaunch` setting key, so
    existing `hub.json` files keep working).
- `Cargo.toml`: added the `Win32_UI_Input_KeyboardAndMouse` windows-sys feature (RegisterHotKey).
- `package/README.txt`: new RACE GRACES section; SAVES mentions the game-start backup.

The separate bingo save is unchanged and still works. It's just no longer required for graces.

## Not verified yet (needs a real game)

These were compile-checked (`cargo check`, no new warnings) but not run against Elden Ring.
Tests also weren't run: the machine had no MSVC linker.

1. Own save, new character: on the name screen the sound plays and the toast shows. Load in and
   check the 11 graces are there and The First Step etc. aren't.
2. Own save, existing non-bingo character: Setup shows "Left alone: … N graces", nothing changes.
3. **Title screen.** Automatic setting may write to the event flags at the title screen if they
   read as all-zero there. That should be harmless, because loading a character replaces every
   event flag from the save. To confirm: launch modded on your own save, wait about 10 s on the
   title screen, load a non-bingo character, and check its graces are unchanged. If they aren't,
   automatic setting needs a "character exists" check (e.g. via GameDataMan) before writing.
4. Ctrl+F9 in game (name screen and mid-game). Mid-game with more than 40 graces should play
   the error sound, and the confirm dialog appears when you switch to the hub.
5. Launch with the Randomizer's Bingo! button: a "[auto] game started" backup appears on the
   Saves page.

## Second round (separate commit): safer bingo on your own save, two fixes

A backup of the last known-working state (v1.0.8, before any of this) is kept as branch
`backup/v1.0.8-working` and tag `v1.0.8-working-backup`. To go back:
`git checkout backup/v1.0.8-working`, or rebuild from that tag.

- **"before bingo" backup that never rotates** (`worker.rs`, `core::BEFORE_BINGO`). With bingo on
  the normal save now supported, the README's advice ("restore a backup from before your bingo
  runs before playing online") could become impossible: auto backups keep only the newest 10,
  and every game start, randomize and launch adds one. Now, when a bingo game starts on the
  normal save (game under 120 s old, not the separate save) and the save differs from what the
  last bingo game left it as, a non-auto backup labelled `before bingo` is made. Non-auto
  backups are never rotated. One is made per switch from normal play to bingo, not per race.
- **Warning before going online with a bingo save.** When a bingo game on the normal save closes,
  the save's SHA-256 is stored in `Settings.bingo_played_hash` (`hub.json` key
  `BingoPlayedHash`). If the game later runs without the mod (`Vanilla`, or `Unknown`, because
  EAC blocks inspecting the process) for over 45 s, and the save still has that hash, the hub
  shows a one-time alert per game process pointing at the newest `before bingo` backup. Once
  the save is restored or played normally the hash no longer matches, so it doesn't nag.
- **Restore is blocked while the separate bingo save is swapped in** (`app.rs`). Before, a
  restore in that window wrote over the bingo save, then the swap-out stored it as the bingo
  save and put the old normal save back, so the restore silently did nothing.
- **Tool updates are checked against the manifest** (`update.rs`). The extracted files are
  hashed against `manifest.json` before the installed folder is replaced. Before, only tar's
  exit code was checked.
- `Msg::Crash` was renamed to `Msg::Alert`, since it's a general (title, body) alert now.

To test: race on your own save, close, then start Elden Ring from Steam. The warning should appear
after about 45 s, and the Saves page should have a `before bingo` backup. Restore it, start from
Steam again, and there should be no warning.

## Ideas considered and left out

- Making the separate save the default or auto-swapping on Randomizer launches. The group
  doesn't want the separate save, and swapping files under a game that's already starting isn't
  safe.
- A configurable hotkey. Ctrl+F9 is only held while a bingo game runs, and the UI says if it's
  taken. Configurability can come later if someone actually hits a conflict.
