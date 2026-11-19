//! Separate bingo save.
//!
//! Elden Ring keeps every character slot in one file, ER0000.sl2. With this on, bingo launches
//! from the hub use their own save file, and the player's normal save is never in the save folder
//! while a bingo game runs.
//!
//! Swap in (before a bingo launch, game closed):
//!   1. Copy the normal save to Data\Main Save and check the copy byte for byte.
//!   2. Write the journal (so a crash at any later point can be undone).
//!   3. Rename the normal save to ER0000.sl2.main (a rename, nothing is deleted).
//!   4. Copy the stored bingo save in (none yet = the game starts a fresh one).
//! Swap out (after the game closes, or on the next start if the hub was closed):
//!   1. Store the bingo save in Data\Bingo Save and check it.
//!   2. Move the bingo save out of the save folder (Recycle Bin, it's already stored).
//!   3. Rename ER0000.sl2.main back to ER0000.sl2 and check it matches the parked copy.
//!   4. Remove the journal.
//! At no point does the normal save exist only in one place.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::sys;

const SAVE: &str = "ER0000.sl2";
const BAK: &str = "ER0000.sl2.bak";

#[derive(Serialize, Deserialize)]
struct Journal {
    save_dir: PathBuf,
    /// SHA-256 of the normal save at swap-in.
    main_hash: String,
    /// Unix seconds of the swap-in.
    started: u64,
}

fn data() -> PathBuf {
    core::paths().data_dir.clone()
}
fn journal_path() -> PathBuf {
    data().join("save-swap.json")
}
fn main_park() -> PathBuf {
    data().join("Main Save")
}
pub fn bingo_store() -> PathBuf {
    data().join("Bingo Save")
}

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_journal() -> Option<Journal> {
    serde_json::from_str(&fs::read_to_string(journal_path()).ok()?).ok()
}

/// The bingo save is in the save folder right now.
pub fn active() -> bool {
    journal_path().is_file()
}

/// Seconds since the swap-in, if active.
pub fn active_for() -> Option<u64> {
    read_journal().map(|j| now().saturating_sub(j.started))
}

pub fn has_bingo_save() -> bool {
    bingo_store().join(SAVE).is_file()
}

fn hash(p: &Path) -> Result<String, String> {
    core::file_hash_hex(p).ok_or_else(|| format!("Couldn't read {}", p.display()))
}

/// Copies `from` to `to` and checks the copy is identical.
fn copy_checked(from: &Path, to: &Path) -> Result<String, String> {
    let h = hash(from)?;
    fs::copy(from, to).map_err(|e| format!("Couldn't copy {}: {e}", from.display()))?;
    if hash(to)? != h {
        return Err(format!("The copy of {} didn't match. Nothing was changed.", from.display()));
    }
    Ok(h)
}

fn touch(p: &Path) {
    if let Ok(f) = fs::OpenOptions::new().write(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Puts the bingo save in place of the normal save. The game must be closed.
pub fn swap_in() -> Result<(), String> {
    if active() {
        return Ok(());
    }
    let dir = core::save_dir().ok_or("Couldn't find your Elden Ring save folder. Start the game normally once first.")?;
    let (live, bak) = (dir.join(SAVE), dir.join(BAK));
    let (live_main, bak_main) = (dir.join(format!("{SAVE}.main")), dir.join(format!("{BAK}.main")));
    if live_main.exists() {
        return Err("A parked save (ER0000.sl2.main) is already in your save folder. Nothing was changed; ask for help before continuing.".into());
    }

    // 1. Parked copy outside the save folder, checked. (No normal save yet = nothing to park.)
    let has_main = live.is_file();
    fs::create_dir_all(main_park()).map_err(|e| e.to_string())?;
    let main_hash = if has_main { copy_checked(&live, &main_park().join(SAVE))? } else { String::new() };
    if has_main && bak.is_file() {
        copy_checked(&bak, &main_park().join(BAK))?;
    }

    // 2. Journal before touching the save folder.
    let j = Journal { save_dir: dir.clone(), main_hash, started: now() };
    core::write_atomic(&journal_path(), serde_json::to_string_pretty(&j).unwrap()).map_err(|e| e.to_string())?;

    // 3. Rename the normal save out of the game's way.
    if has_main {
        if let Err(e) = fs::rename(&live, &live_main) {
            let _ = fs::remove_file(journal_path());
            return Err(format!("Couldn't move your save aside: {e}. Nothing was changed."));
        }
        if bak.is_file() {
            let _ = fs::rename(&bak, &bak_main);
        }
    }

    // 4. Bring in the stored bingo save, if there is one.
    let store = bingo_store();
    if store.join(SAVE).is_file() {
        let r = copy_checked(&store.join(SAVE), &live).and_then(|_| copy_checked(&store.join(SAVE), &bak).map(|_| ()));
        if let Err(e) = r {
            // The files in the save folder are copies of the stored bingo save, possibly half
            // written; remove them so undoing doesn't store them over the good copy.
            let _ = fs::remove_file(&live);
            let _ = fs::remove_file(&bak);
            let _ = swap_out();
            return Err(e);
        }
        touch(&live);
        touch(&bak);
    }
    Ok(())
}

/// Stores the bingo save and puts the normal save back. The game must be closed.
pub fn swap_out() -> Result<(), String> {
    let Some(j) = read_journal() else { return Ok(()) };
    let dir = j.save_dir.clone();
    let (live, bak) = (dir.join(SAVE), dir.join(BAK));
    let (live_main, bak_main) = (dir.join(format!("{SAVE}.main")), dir.join(format!("{BAK}.main")));

    // 1 + 2. Keep the bingo save, then clear it from the save folder.
    let store = bingo_store();
    fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    if live.is_file() {
        copy_checked(&live, &store.join(SAVE))?;
        sys::recycle(&live)?;
    }
    if bak.is_file() {
        let _ = copy_checked(&bak, &store.join(BAK));
        let _ = sys::recycle(&bak);
    }

    // 3. Normal save back where it was, checked against the hash from swap-in.
    if j.main_hash.is_empty() {
        // There was no normal save to begin with.
        return fs::remove_file(journal_path()).map_err(|e| e.to_string());
    }
    if live_main.is_file() {
        fs::rename(&live_main, &live).map_err(|e| format!("Couldn't put your save back: {e}. It's safe at {}", live_main.display()))?;
    } else {
        // The renamed file went missing: use the parked copy.
        copy_checked(&main_park().join(SAVE), &live)?;
    }
    if bak_main.is_file() {
        let _ = fs::rename(&bak_main, &bak);
    } else if main_park().join(BAK).is_file() {
        let _ = copy_checked(&main_park().join(BAK), &bak);
    }
    if hash(&live)? != j.main_hash {
        return Err(format!("Your restored save doesn't match the parked copy. Your original is safe in {}.", main_park().display()));
    }
    // Fresh timestamps so Steam Cloud keeps the normal save as the newest.
    touch(&live);
    touch(&bak);

    // 4. Done.
    fs::remove_file(journal_path()).map_err(|e| e.to_string())
}

/// Starts the next bingo game with a fresh save (old bingo save goes to the Recycle Bin).
pub fn reset_bingo_save() -> Result<(), String> {
    if active() {
        return Err("Close the bingo game first.".into());
    }
    for f in [SAVE, BAK] {
        let p = bingo_store().join(f);
        if p.is_file() {
            sys::recycle(&p)?;
        }
    }
    Ok(())
}

/// Runs only against a throwaway save folder: `cargo test -- --ignored swap`.
#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        fs::write(p, s).unwrap();
    }
    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    #[ignore]
    fn swap_round_trips() {
        let sandbox = std::env::temp_dir().join("bingohub-swap-test");
        let _ = fs::remove_dir_all(&sandbox);
        let saves = sandbox.join("EldenRing").join("76561190000000000");
        fs::create_dir_all(&saves).unwrap();
        std::env::set_var("BINGOHUB_SAVE_DIR", &saves);
        assert_eq!(core::save_dir().unwrap(), saves, "must only ever touch the sandbox");
        let _ = fs::remove_dir_all(data());
        fs::create_dir_all(data()).unwrap();
        let (live, bak) = (saves.join(SAVE), saves.join(BAK));
        write(&live, "MAIN SAVE with 10 characters");
        write(&bak, "MAIN BAK");
        let other_file = saves.join("ER0000.co2");
        write(&other_file, "SEAMLESS");

        // First bingo launch: no bingo save yet, so the game will create a fresh one.
        swap_in().unwrap();
        assert!(active());
        assert!(!live.exists(), "normal save must not be in the game's way");
        assert_eq!(read(&saves.join("ER0000.sl2.main")), "MAIN SAVE with 10 characters");
        assert_eq!(read(&main_park().join(SAVE)), "MAIN SAVE with 10 characters");
        // The game plays and writes its own save.
        write(&live, "BINGO RUN 1");
        write(&bak, "BINGO BAK 1");
        swap_out().unwrap();
        assert!(!active());
        assert_eq!(read(&live), "MAIN SAVE with 10 characters");
        assert_eq!(read(&bak), "MAIN BAK");
        assert_eq!(read(&bingo_store().join(SAVE)), "BINGO RUN 1");
        assert!(!saves.join("ER0000.sl2.main").exists());

        // Second launch brings the bingo save back.
        swap_in().unwrap();
        assert_eq!(read(&live), "BINGO RUN 1");
        write(&live, "BINGO RUN 2");

        // Crash recovery: the hub restarts mid-session and only has the journal.
        swap_out().unwrap();
        assert_eq!(read(&live), "MAIN SAVE with 10 characters");
        assert_eq!(read(&bingo_store().join(SAVE)), "BINGO RUN 2");

        // The renamed save went missing somehow: the parked copy restores it.
        swap_in().unwrap();
        fs::remove_file(saves.join("ER0000.sl2.main")).unwrap();
        swap_out().unwrap();
        assert_eq!(read(&live), "MAIN SAVE with 10 characters");

        // Unrelated files (Seamless Co-op) are never touched.
        assert_eq!(read(&other_file), "SEAMLESS");

        // Start fresh only clears the bingo store.
        reset_bingo_save().unwrap();
        assert!(!has_bingo_save());
        assert_eq!(read(&live), "MAIN SAVE with 10 characters");
        let _ = fs::remove_dir_all(&sandbox);
    }
}
