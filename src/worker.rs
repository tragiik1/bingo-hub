//! Background poller. Everything slow lives here so the UI thread only ever paints.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use crate::core::{self, Backup, Check, Settings};
use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum GameState {
    #[default]
    NotRunning,
    Starting,
    Modded,
    Vanilla,
    Unknown,
}

#[derive(Clone, PartialEq, Default)]
pub struct Snapshot {
    pub game: GameState,
    pub game_pid: Option<u32>,
    pub board: Option<u32>,
    pub rando: Option<u32>,
    pub debug: Option<u32>,
    pub steam: bool,
    pub regulation: Option<SystemTime>,
    pub live_save: Option<SystemTime>,
    pub backups: Vec<Backup>,
    pub game_exe: Option<PathBuf>,
    pub rando_run: Option<core::RandoRun>,
    /// Automatic race graces in the current game.
    pub graces: crate::graces::Status,
    /// The separate bingo save is in the save folder right now.
    pub bingo_save: bool,
    /// Starting classes for the last randomize.
    pub classes: Option<crate::classes::Spoiler>,
    /// The race graces hotkey, one of `sys::hotkey`.
    pub hotkey: u8,
}

pub struct Shared {
    pub snap: Mutex<Snapshot>,
    pub settings: Mutex<Settings>,
    /// Window has focus: poll faster.
    pub focused: AtomicBool,
    /// Set to wake the poller right away (after launching something).
    pub poke: (Mutex<bool>, Condvar),
    /// The race graces hotkey, one of `sys::hotkey`.
    pub hotkey: AtomicU8,
}

impl Shared {
    pub fn new(settings: Settings) -> Self {
        Self {
            snap: Mutex::new(Snapshot::default()),
            settings: Mutex::new(settings),
            focused: AtomicBool::new(true),
            poke: (Mutex::new(false), Condvar::new()),
            hotkey: AtomicU8::new(sys::hotkey::OFF),
        }
    }

    /// Re-scan now, and keep scanning quickly for a few seconds while the tool starts.
    pub fn wake(&self) {
        *self.poke.0.lock().unwrap() = true;
        self.poke.1.notify_one();
    }
}

/// Results coming back to the UI thread from background work.
pub enum Msg {
    Toast(String),
    Error(String),
    Checks(Vec<Check>),
    BoardsChanged,
    GameLocated(PathBuf),
    /// (result, user asked for it)
    Update(Result<Option<crate::update::Release>, String>, bool),
    UpdateFailed(String),
    ToolsUpdated(Vec<String>),
    /// (title, explanation), e.g. for a Randomizer or game crash Windows just logged.
    Alert(String, String),
    Restart,
    /// A manual race graces attempt (button or hotkey) on this game.
    Graces(u32, Result<crate::graces::Outcome, String>),
}

pub fn spawn(shared: Arc<Shared>, tx: Sender<Msg>, ctx: egui::Context) {
    let (s, t, c) = (shared.clone(), tx.clone(), ctx.clone());
    std::thread::Builder::new().name("hotkey".into()).spawn(move || hotkey(s, t, c)).expect("hotkey thread");
    std::thread::Builder::new().name("poller".into()).spawn(move || run(shared, tx, ctx)).expect("poller thread");
}

/// Ctrl+F9 in game sets the race graces, the same as the button. Works from the name screen on.
/// The result is heard straight away and shown in the hub once it's looked at.
fn hotkey(shared: Arc<Shared>, tx: Sender<Msg>, ctx: egui::Context) {
    let modded_pid = || {
        let s = shared.snap.lock().unwrap();
        s.game_pid.filter(|_| s.game == GameState::Modded)
    };
    sys::hotkey_loop(
        || modded_pid().is_some(),
        &shared.hotkey,
        || {
            let Some(pid) = modded_pid() else { return sys::chime(false) };
            let r = crate::graces::apply_now(pid, false);
            sys::chime(matches!(r, Ok(crate::graces::Outcome::Applied(_) | crate::graces::Outcome::AlreadySet)));
            let _ = tx.send(Msg::Graces(pid, r));
            ctx.request_repaint();
        },
    );
}

fn run(shared: Arc<Shared>, tx: Sender<Msg>, ctx: egui::Context) {
    let p = core::paths();
    let rando_file = || core::rando_exe().and_then(|e| e.file_name().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
    let mut rando_name = rando_file();
    let (mut prev_rando, mut prev_game): (Option<u32>, Option<u32>) = (None, None);
    let mut last_crash = String::new();
    let mut grace_game: Option<(u32, Result<crate::graces::Game, String>, Instant)> = None;
    let mut grace_checked = Instant::now();
    let mut graces_status = crate::graces::Status::Idle;
    let mut saw_game_since_swap = false;
    let mut prev_state = GameState::NotRunning;
    // This game ran with the bingo mod on the normal save (not the separate bingo save).
    let mut bingo_on_normal = false;
    let mut online_checked: Option<u32> = None;
    let mut spoiler_time: Option<SystemTime> = None;
    let mut classes: Option<crate::classes::Spoiler> = None;
    // Race code inputs, recomputed only when one of them changes.
    let mut exe_key: Option<(PathBuf, u64, Option<SystemTime>)> = None;
    let mut exe_hash = String::new();
    let mut run_key: Option<(Option<SystemTime>, Option<SystemTime>, String)> = None;
    let mut rando_run: Option<core::RandoRun> = None;
    let mut modded: HashMap<u32, bool> = HashMap::new();
    let mut last_reg = core::mtime(&p.regulation);
    let mut reg_changed_at: Option<Instant> = None;
    let mut backups_sig: Option<SystemTime> = None;
    let mut backups: Vec<Backup> = Vec::new();
    let mut game_exe = None;
    let mut tick: u64 = 0;
    let mut procs = sys::ProcCache::default();
    let mut fast_until = Instant::now();
    core::unlock_randomizer_output();

    loop {
        let settings = shared.settings.lock().unwrap().clone();
        procs.refresh();
        let pid_of = |n: &str| procs.pid_of(n);

        let game_pid = pid_of("eldenring.exe");
        let game = match game_pid {
            None => GameState::NotRunning,
            Some(pid) => match modded.get(&pid) {
                Some(true) => GameState::Modded,
                Some(false) => GameState::Vanilla,
                None => match sys::has_module(pid, "modengine2.dll") {
                    Some(true) => {
                        modded.insert(pid, true);
                        GameState::Modded
                    }
                    Some(false) if sys::process_age(pid).unwrap_or(999.0) > 45.0 => {
                        modded.insert(pid, false);
                        GameState::Vanilla
                    }
                    Some(false) => GameState::Starting,
                    // Anti-cheat blocks inspecting the game, and bingo launches never start it.
                    None if pid_of("easyanticheat_eos.exe").or_else(|| pid_of("easyanticheat.exe")).is_some() => GameState::Vanilla,
                    None => GameState::Unknown,
                },
            },
        };

        // The Randomizer just wrote regulation.bin: once it's stable, back up the save.
        let reg = core::mtime(&p.regulation);
        if reg != last_reg {
            last_reg = reg;
            reg_changed_at = Some(Instant::now());
        }
        if reg_changed_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(3)) {
            reg_changed_at = None;
            let mut msg = String::from("Randomized");
            if settings.auto_backup_on_randomize && game_pid.is_none() {
                match core::create_backup("after randomize", true, settings.keep_auto_backups) {
                    Ok(Some(_)) => msg += ", save backed up",
                    Ok(None) => {}
                    Err(e) => msg = format!("Randomized. Backup failed: {e}"),
                }
            }
            let _ = tx.send(Msg::Toast(msg));
            ctx.request_repaint();
        }

        // Backups are only re-listed when the folder changes.
        let sig = core::mtime(&p.backups_dir);
        if sig != backups_sig || tick % 20 == 0 {
            backups_sig = sig;
            backups = core::list_backups();
        }
        if tick % 15 == 0 {
            game_exe = core::find_game_exe(settings.game_exe_override.as_deref());
            rando_name = rando_file(); // its file name changes when the Randomizer is updated
        }
        let rando_pid = if rando_name.is_empty() { None } else { pid_of(&rando_name) };
        if rando_pid.is_some() {
            core::unlock_randomizer_output(); // a few stat calls; keeps a read-only flag from crashing it
        }

        // Race code: the Randomizer's last run, the build that made it and the game patch it read.
        if tick % 3 == 0 {
            if let Some(exe) = core::rando_exe() {
                let meta = std::fs::metadata(&exe).ok();
                let key = (exe.clone(), meta.as_ref().map(|m| m.len()).unwrap_or(0), meta.and_then(|m| m.modified().ok()));
                if exe_key.as_ref() != Some(&key) {
                    exe_hash = core::file_hash_hex(&exe).unwrap_or_default();
                    exe_key = Some(key);
                }
            }
            let game_dir = game_exe.as_ref().and_then(|g| g.parent().map(|d| d.to_path_buf()));
            let seed_time = core::mtime(&core::last_seed_file());
            let key = (seed_time, game_dir.as_ref().and_then(|d| core::mtime(&d.join("regulation.bin"))), exe_hash.clone());
            if run_key.as_ref() != Some(&key) {
                // After a game update the Randomizer's text copy is stale (it never refreshes it
                // itself), which changes what seeds give from player to player. Drop it so the next
                // randomize copies the current text, like everyone else's.
                if let Some(d) = game_dir.as_deref().filter(|_| rando_pid.is_none()) {
                    core::drop_stale_rando_text(d);
                }
                rando_run = core::rando_run(&exe_hash, game_dir.as_deref());
                run_key = Some(key);
                // A new successful run: remember which game version it worked with.
                let secs = seed_time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                if rando_run.is_some() && secs > settings.rando_game_version_time() {
                    if let Some(v) = game_exe.as_ref().and_then(|g| sys::file_version(g)) {
                        let mut s = shared.settings.lock().unwrap();
                        s.rando_game_version = v;
                        s.rando_run_time = secs;
                        s.save();
                    }
                }
            }
        }

        // A tool just closed: if Windows logged a crash for it, explain it.
        let rando_closed = prev_rando.is_some() && rando_pid.is_none();
        let game_closed = prev_game.is_some() && game_pid.is_none();
        if rando_closed || game_closed {
            std::thread::sleep(Duration::from_millis(1500)); // the log entry lands just after the exit
            let found = if rando_closed { crate::crash::randomizer(&rando_name, 60) } else { crate::crash::game(60, game_exe.as_deref().and_then(|g| g.parent())) };
            if let Some(c) = found.filter(|c| c.when != last_crash) {
                last_crash = c.when.clone();
                let _ = tx.send(Msg::Alert(c.title, c.body));
                ctx.request_repaint();
            }
        }
        prev_rando = rando_pid;

        // Separate bingo save: put the normal save back once the bingo game has closed. Also runs
        // on start-up, so a crash or a closed hub can't leave the bingo save in place, and when the
        // record of a swap was lost but the parked normal save is still there.
        if crate::bingosave::active() || crate::bingosave::adopt_orphan() {
            if game_pid.is_some() {
                saw_game_since_swap = true;
            } else if saw_game_since_swap || crate::bingosave::active_for().unwrap_or(0) > 90 {
                saw_game_since_swap = false;
                let _ = tx.send(match crate::bingosave::swap_out() {
                    Ok(()) => Msg::Toast("Your normal save is back".into()),
                    Err(e) => Msg::Error(format!("Couldn't swap your normal save back.\n\n{e}")),
                });
                ctx.request_repaint();
            }
        }

        // Starting classes from the spoiler log, re-read when it changes.
        let st = core::mtime(&crate::classes::log_path());
        if st != spoiler_time {
            spoiler_time = st;
            classes = crate::classes::load();
        }

        // Race graces. On the separate bingo save every character is a bingo character, so any fresh
        // one gets them. On any other save only a brand-new character does, which covers the name
        // screen (where the Cheat Engine script was run) and the first moments after Finish.
        match (game, game_pid) {
            (GameState::Modded, Some(pid)) if settings.auto_graces => {
                if grace_checked.elapsed() >= Duration::from_secs(2) {
                    grace_checked = Instant::now();
                    let reopen = match &grace_game {
                        Some((p, Ok(_), _)) => *p != pid,
                        Some((p, Err(_), at)) => *p != pid || at.elapsed() > Duration::from_secs(30),
                        None => true,
                    };
                    if reopen {
                        grace_game = Some((pid, crate::graces::Game::open(pid), Instant::now()));
                    }
                    use crate::graces::{Outcome, Status};
                    let limit = if crate::bingosave::active() { crate::graces::FRESH_LIMIT } else { crate::graces::NEW_LIMIT };
                    graces_status = match &grace_game {
                        Some((_, Ok(g), _)) => match g.apply(Some(limit)) {
                            Outcome::Applied(_) => {
                                sys::chime(true);
                                let _ = tx.send(Msg::Toast("Race graces set".into()));
                                Status::Set
                            }
                            Outcome::AlreadySet => Status::Set,
                            Outcome::NotLoaded => Status::Waiting,
                            Outcome::NotFresh(n) => Status::Skipped(n),
                            Outcome::WriteFailed => Status::Error("The game didn't take the graces. Try Set graces again.".into()),
                        },
                        Some((_, Err(e), _)) => Status::Error(e.clone()),
                        None => Status::Idle,
                    };
                }
            }
            (GameState::NotRunning, _) => {
                grace_game = None;
                graces_status = crate::graces::Status::Idle;
            }
            _ => {}
        }

        // A game just started, from the hub or anywhere else (the Randomizer's Bingo! button):
        // back up the save before a character is loaded. Identical to the hub's pre-launch backup
        // when launched from here, so that one is skipped as a duplicate.
        if let Some(pid) = game_pid.filter(|_| prev_game.is_none()) {
            let fresh_start = sys::process_age(pid).is_some_and(|a| a < 60.0);
            if fresh_start && settings.auto_backup_on_launch && !crate::bingosave::active() {
                if let Err(e) = core::create_backup("game started", true, settings.keep_auto_backups) {
                    let _ = tx.send(Msg::Toast(format!("Couldn't back up your save: {e}")));
                }
            }
        }

        // Bingo on the normal save. Auto backups rotate out, so the first bingo game after the save
        // was played some other way keeps a "before bingo" copy that never does. When the game
        // closes, remember what bingo left the save as, so launching it normally gets a warning.
        let live_hash = || core::live_save().and_then(|s| core::file_hash_hex(&s));
        if game == GameState::Modded && prev_state != GameState::Modded && !crate::bingosave::active() {
            bingo_on_normal = true;
            let just_started = game_pid.and_then(sys::process_age).is_some_and(|a| a < 120.0);
            if just_started && live_hash().is_some_and(|h| h != settings.bingo_played_hash) {
                if let Err(e) = core::create_backup(core::BEFORE_BINGO, false, settings.keep_auto_backups) {
                    let _ = tx.send(Msg::Toast(format!("Couldn't back up your save: {e}")));
                }
            }
        }
        if game_closed && std::mem::take(&mut bingo_on_normal) {
            if let Some(h) = live_hash() {
                let mut s = shared.settings.lock().unwrap();
                s.bingo_played_hash = h;
                s.save();
            }
        }
        if let Some(pid) = game_pid.filter(|&p| game == GameState::Vanilla && online_checked != Some(p) && sys::process_age(p).is_some_and(|a| a > 45.0)) {
            online_checked = Some(pid);
            if !settings.bingo_played_hash.is_empty() && live_hash().as_ref() == Some(&settings.bingo_played_hash) {
                let restore = match core::newest_before_bingo() {
                    Some(b) => format!("restore \"{}\" ({}) from the Saves page", b.label, core::when(b.when)),
                    None => "restore a backup from before your bingo runs from the Saves page".into(),
                };
                let _ = tx.send(Msg::Alert(
                    "This save was last used for bingo".into(),
                    format!("Elden Ring is running without the bingo mod on a save that still has your bingo characters. Randomized gear and stats can get a character flagged online.\n\nBefore playing online, close the game and {restore}."),
                ));
                ctx.request_repaint();
            }
        }
        prev_state = game;
        prev_game = game_pid;

        let snap = Snapshot {
            game,
            game_pid,
            board: pid_of("eldenbingo.exe"),
            rando: rando_pid,
            debug: pid_of("elden ring debug tool.exe"),
            steam: pid_of("steam.exe").is_some(),
            regulation: reg,
            live_save: core::live_save().and_then(|s| core::mtime(&s)),
            backups: backups.clone(),
            game_exe: game_exe.clone(),
            rando_run: rando_run.clone(),
            graces: graces_status.clone(),
            bingo_save: crate::bingosave::active(),
            classes: classes.clone(),
            hotkey: shared.hotkey.load(Ordering::Relaxed),
        };
        {
            let mut cur = shared.snap.lock().unwrap();
            if *cur != snap {
                *cur = snap;
                ctx.request_repaint();
            }
        }
        modded.retain(|pid, _| Some(*pid) == game_pid);
        tick += 1;
        // Scanning every process is the only real cost, so pace it: quick while something is starting
        // or the window is focused, slow in the background.
        let wait = if game == GameState::Starting || Instant::now() < fast_until {
            500
        } else if shared.focused.load(Ordering::Relaxed) {
            1500
        } else {
            4000
        };
        let (lock, cv) = &shared.poke;
        let mut poked = lock.lock().unwrap();
        if !*poked {
            poked = cv.wait_timeout(poked, Duration::from_millis(wait)).unwrap().0;
        }
        if *poked {
            fast_until = Instant::now() + Duration::from_secs(8);
        }
        *poked = false;
    }
}
