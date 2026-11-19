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
| Set graces button / Ctrl+F9 | Same limit as automatic (40 on the bingo save, 3 elsewhere) | Asks for confirmation, then forces |

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
  - Race page: the status line
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

## Third round (separate commit): full code review, bug fixes

The whole codebase was reviewed for bugs. No new features were added; these are fixes:

- **Practice card crash** (`app.rs`). The card stored its board as a position in the board list.
  Dropping a board file onto the Boards page while a card was open reloads and re-sorts the list,
  so the card could point at a different board and index past its goals, which crashed the app
  (`panic = "abort"`). The card now tracks its board by file path, and closes if that board is
  gone or changed.
- **Half-written files** (`core.rs`, `bingosave.rs`). Backups were copied straight to the
  final `.sl2` name, so a failed copy (disk full, crash) left a truncated backup that was listed
  and restorable, which would corrupt the save, and auto-backup duplicate detection compared
  against it. Backups now copy to `.part` and are renamed when complete. Settings (`hub.json`),
  the run log (`runs.json`) and the bingo save journal now go through `core::write_atomic`
  (temp file + rename). A corrupt journal was the worst case: `active()` stayed true forever, but
  `swap_out` couldn't read the file, so it did nothing.
- **Unreadable run log** (`core::load_runs`). If `runs.json` fails to parse, it's copied to
  `runs.bad.json` instead of being silently overwritten by the next save.
- **Games launched with anti-cheat showed "Running, mod not confirmed"** (`worker.rs`). EAC stops
  the hub from listing the game's modules, so it was `Unknown`. If the Easy Anti-Cheat service is
  running, bingo launches never start it, so the game is now `Vanilla` ("No mod"). This also makes
  the round-two warning before going online check only `Vanilla`. Before, it also fired on
  `Unknown`, which would include a bingo game running as administrator.
- **Process names that couldn't be read were cached forever** (`sys::ProcCache`). A process
  opened too early would never be detected; empty names are now retried.
- **Manual graces re-scanned the game every press** (`graces::Game::open`). Finding the event
  flag manager means scanning tens of MB of game code. The address is now remembered per game
  process (and re-checked before use), so the button and Ctrl+F9 respond straight away.
- **Compiler warnings** (`ui.rs`, `app.rs`). 16 `Stroke::new(1.0, ..)` calls trigger a lint that
  newer Rust says will become a hard error, which would break the release build after a toolchain
  update. They got `_f32` suffixes; the build has zero warnings now.
- **README step that didn't exist** (`package/README.txt`). It told the host to press a Generate
  button on the Race page; there isn't one. It now says to enter any seed and share it.
- **Saving when a rename is refused** (`core::write_atomic`). If renaming the temp file fails
  (antivirus or a sync tool holding the file), it now falls back to writing in place instead of
  losing the save.

## Findings for next steps (not code changes)

Status: everything above is on `main` and unreleased. v1.0.8 is still the live release. Nothing
here has been run in game yet; `cargo check` passes with zero warnings, but `cargo test` and
release builds weren't possible on the machine these changes were made on.

### Some seeds crash the Randomizer (the owner wants this fixed)

Example: `hello123` and `hello124` work, `hello125` crashes. The seed text goes through SHA-256
into a number that drives every shuffle, so similar seeds give unrelated shuffles; a crash means
that seed's shuffle hits a bug.

Prime suspect, from reading Nordgaren's original source (github.com/Nordgaren/ERBingoRandomizer,
MIT), which `S2_Style` is built on:

- `randomizeShopLineupParam` builds `shopLineupParamList` from
  `_weaponDictionary.Keys.Select(removeWeaponMetadata)` (`id / 10000 * 10000`). Some of those
  stripped IDs aren't weapons the Randomizer knows. `replaceShopLineupParam` pops one and does
  `_weaponNameDictionary[newId]`, which throws `KeyNotFoundException`. Only seeds whose shuffle
  puts such an ID into a shop crash. The starting-class code (`getRandomWeapon(int, list)`) guards
  against exactly this with `_weaponDictionary.ContainsKey`; the shop code doesn't. The fix is to
  filter the pool the same way.
- Not confirmed: `S2_Style` is modified and couldn't be run. To confirm, run a crashing seed, then
  `Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='.NET Runtime'; Id=1026} -MaxEvents 1 | Format-List Message`
  gives the stack trace.
- Also noticed: `buildDictionaries` uses `Enumerable.Range(81, 86)` to leave ammo (types 81-86)
  out of `_weaponDictionary`. `Range(start, count)` covers 81-166, so torches and probably the DLC
  weapon types are left out of weapon randomization too.
- Fixing it means changing the Randomizer. `S2_Style_1.0.exe` is a closed modified build (assembly
  "Randomizer", namespace `ERBingoRandomizer`, no credits inside). The owner thinks it's another
  person's edit of the original but doesn't remember who. Options: get the source or a fix from
  that author; or rebuild from Nordgaren's MIT source with the fix, which only works if it
  randomizes identically to `S2_Style` (it won't unless their changes are known). Whatever build is
  used, everyone has to use the same one; the race code already includes the exe's hash.

### Licences of the bundled tools

| Tool | Licence | Notes |
| --- | --- | --- |
| EldenBingo 0.18 (awsker/EldenBingo) | GPL-3.0 | Modifiable; a modified build must publish its source under GPL. |
| Elden Ring Debug Tool (Nordgaren) | GPL-3.0 | Same. |
| ModEngine 2 (soulsmods) | MIT | Keep the copyright notice. |
| Bingo Randomizer `S2_Style_1.0` | Unknown | See above. Original ERBingoRandomizer is MIT. |

Gap: the release redistributes the two GPL tools but doesn't include their licence text or a link
to their source. Adding both (README and the package) would fix it.

### Answered questions

- **Bingo Board stays on 0.18.** The group's board connects to `bingocard.bingobrawlers.com`.
  EldenBingo 0.19 changed the packet format and doesn't connect there. Its "Elden Ring 1.17"
  support only adds the two new starting classes, so 0.18 is correct. Don't bump it until the
  server updates.
- **The race code is complete.** The Randomizer's `LastSeed.json` holds only `Seed` and
  `Sha256Hash`; there are no options to miss.
- The group only used Cheat Engine for the graces. The other feature ideas raised (race invites,
  character names on backups, Discord status, race modifiers, rivalry stats, goal tracking) were
  turned down as not worth it.

## Ideas considered and left out

- Making the separate save the default or auto-swapping on Randomizer launches. The group
  doesn't want the separate save, and swapping files under a game that's already starting isn't
  safe.
- A configurable hotkey. Ctrl+F9 is only held while a bingo game runs, and the UI says if it's
  taken. Configurability can come later if someone actually hits a conflict.

## Fourth round: review fixes, updater fallback, licences

- **Set graces / Ctrl+F9 use the automatic limit** (`graces::apply_now`): 40 on the separate bingo
  save, 3 elsewhere, and the hub asks before going past it. Before, one Ctrl+F9 press on a real
  character with up to 40 graces cleared them without asking.
- **Set graces button removed from the Race page.** It made the manual fallback a main control,
  which the owner asked not to do. It's still in Setup, and the Race status line mentions Ctrl+F9.
- **Self-update fallback** (`update.rs`, `core.rs`): if Windows refuses to replace the running
  exe ("Access is denied"), a small script swaps it after the hub exits and restarts it. Setup
  also warns when Windows blocks the hub from writing files.
- **Licences**: `package/LICENSES.txt` (shipped in the zip) lists each tool's licence and source,
  with the full GPL-3.0 and MIT texts. READMEs said "Randomizer 0.91"; fixed.
- Tested here: release build with zero warnings, `cargo test`, and the sandboxed save swap test.
  Still not run in game.

### Randomizer crash: cause found

Not the shop code. `S2_Style_1.0.exe` (its build paths name the project `EquipmentRandomizerER`)
names starting weapons in `addDescriptionString` from a hardcoded table
(`Project.Settings.Equipment.EquipmentNameList`), not the game's text. The game's latest patch
added weapons that are in the starting pool but not in that table (Ritual Thrusting Shield,
Idus Sword, Reed Great Katana, Hefty Scimitar, Reverse-Bladed Sword, Silver Grooved Shield, Golden
Order Flail), so any seed that rolls one crashes: 46 of 100 seeds when run against the current
game files with S2's own code (the harness reproduced the real Randomizer's spoiler log exactly).
Fix: add the missing names to that table. The table is only used for description text, so working
seeds randomize identically and crashing seeds start working. Not done yet.

### Open concerns from the review (need the game to check)

- A grace check during a character load could read the flags half-loaded and add the 11 graces
  to a real character. Checking in-game play time (GameDataMan) instead of the grace count would
  close this, the title-screen write and the "real character with 3 or fewer graces" case.
- The go-online warning shows on every normal launch after a race until the save is played.

## Fifth round: patch-day check

- **The Randomizer's crash is fixed** by patching its weapon-name table (`tools/randomizer-patch`):
  all 115 named weapons the table lacked were added. 0/100 test seeds crash (was 46/100), and seed
  1712's spoiler log is byte-identical to the unpatched build. The unpatched exe is in
  `Data\Randomizer backup`.
- **Setup warns when a game update adds weapons the Randomizer can't name** (`src/randocheck.rs`).
  It reads the name table straight from the Randomizer's exe (the .NET bundle, then the IL that
  fills the table) and the weapon names from the Randomizer's copy of the game text (DCX via the
  game's own Oodle DLL, then BND4 and FMG). Read-only; re-checked only when either file changes.
  The list goes to `Data\randomizer-missing-names.tsv`, and `tools/randomizer-patch/patch.ps1`
  patches it in with one command. Fixes go out as a tool update, so every hub runs the same build.
- **Stale Randomizer text** (`core::drop_stale_rando_text`): the Randomizer copies the game text to
  `Cache\msg` on its first run and never refreshes it, and it only picks weapons that have a name
  in that copy. So a copy made before a game update gives different games from the same seed than
  one made after. The hub now deletes the copy when the game's regulation.bin is newer (the next
  randomize remakes it), and the race code includes the copy's hash, so a mismatch shows up as a
  different code instead of a silently different race.

## Sixth round: full review fixes

- **Run log edits could change the wrong run** (`app.rs`). Edit and Delete remembered the run's
  position; a bingo launch adds a race at the top of the log, so with a dialog open the save or
  delete landed one run off. They now find the run by value.
- **A failed bingo-save swap-in could overwrite the stored bingo save** (`bingosave.rs`). If copying
  the stored bingo save into the save folder failed partway, the undo stored the half-written copy
  over the good one. The partial copies are now removed before undoing. The normal save was never
  at risk.
- **Game crash hints ignored a manually located game** (`crash.rs`): they now use the install the
  hub uses.
- **Wording**: "This seed doesn't work" now points at weapons from game updates (Setup lists them)
  instead of a "known bug"; "Elden Ring updated since your last randomize" now says to randomize
  again (the randomized files were made from the old version) instead of "the Randomizer may need
  an update"; READMEs and LICENSES.txt say the Randomizer carries the weapon-name patch instead of
  "included unchanged".
- Duplicate wide-string helper in `randocheck.rs` replaced with `sys::wide`.

Reviewed and fine as they are: settings, backups (copy to `.part` then rename), journal and save
swap, race code inputs, process tracking, graces (open/validate/apply/limits), hotkey thread,
self-update (rename, deferred swap, manifest check), crash parsing, spoiler parsing, release
script. Known edges left alone: `restore_backup` copies straight over the live save (a "before
restore" backup is made first); a Randomizer tool update replaces its folder, so its cache and last
output go and everyone randomizes again (intended).

## Seventh round: limitations fixed

- **Crash explanations work on any Windows language** (`crash.rs`). Events are read as XML; the
  text form's labels and the "Faulting application name" message are translated on non-English
  Windows, so a friend on German or French Windows got no explanations. Checked against a real
  S2 Style crash in this PC's log.
- **Restoring a backup can't leave a half-written save** (`core::restore_backup`). It copies to
  `ER0000.restoring`, checks the copy, then renames it over the save. Sandbox test:
  `cargo test -- --ignored restore_sandbox`.
- **Stale Randomizer text is caught by game version, not only by file dates**
  (`core::drop_stale_rando_text`). Each text copy is recorded in `Data\rando-text.txt` with the
  game's regulation.bin hash; a different game is stale. Copies the hub hasn't seen before still
  use the date check.
- **Setup says when the patch-day check can't run** (an Info row, no badge) instead of silently
  showing nothing, e.g. if a future Randomizer build is packed differently.
- Removed `tools/randomizer-patch/patcher/names.tsv`, an out-of-date list (patch.ps1 uses the list
  the hub writes).

Still limitations (not fixable here, or need the game to test): race graces read flags with a
timing window during a character load (needs an in-game check, see the fourth round); the
separate bingo save only applies to launches from the hub, not the Randomizer's Bingo! button
(swapping files under a starting game isn't safe); fixing the Randomizer needs its closed exe
patched (`tools/randomizer-patch`).

## Eighth round: local 1.0.9 test

- **A hub newer than the latest release no longer offers tool "updates"** (`update::check`). Testing
  a local 1.0.9 build, the hub compared tools against the live 1.0.8 manifest, offered the patched
  Randomizer as an update, and installing it put the unpatched build back (seed `noahisgoat`
  crashed again, key 62520000). Friends are never ahead of the latest release, so this only hit
  local builds. The patched exe was restored from `dist`.
- Tested: seed tester against the game, `noahisgoat` and `hello125` ok on the patched build;
  `noahisgoat` crashes on the original with the same key. `.gitignore` now covers `tools/**/out/`
  (the tester's output holds a copy of the game's Oodle DLL and params).
- **The patched Randomizer still crashed in the real app** (`noahisgoat`, after the restore). The
  exe is a self-extracting .NET bundle: it unpacks to `%TEMP%\.net\S2_Style_1.0\<bundle id>` and
  reuses that folder if it exists. `repack.py` kept the original ID, so anyone who had run the
  Randomizer before (everyone) kept running the old dll. `repack.py` now writes a new ID derived
  from the new dll. Rebuilt from the original exe; installed build 1C8DBBB7. Tested: launching it
  creates a new folder holding the patched dll (7D18C3B3). `seedtest` couldn't catch this because
  it loads the dll directly; the README now says to check the real app.
- **A parked normal save with no journal is now put back** (`bingosave::adopt_orphan`, called from
  the worker). Found on the user's PC: `ER0000.sl2.main` (normal save, matching `Data\Main Save`)
  had sat in the save folder since a 14 Nov pre-1.0.8 test with no `save-swap.json`, so the hub
  never swapped back, turning the setting off did nothing, and swap-in refused. Now a `.main` with
  no journal gets a journal written from its own hash, and the usual swap-out runs once the game is
  closed (stores the bingo save, recycles it from the save folder, renames `.main` back, checks the
  hash). Tested: `swap_round_trips` covers losing the journal mid-swap. Before the fix runs on the
  user's save, every file involved was copied to `Save Backups\2026-11-19 before putting normal
  save back`.
