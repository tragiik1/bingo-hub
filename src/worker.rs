//! Background poller. Everything slow lives here so the UI thread only ever paints.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

pub struct Shared {
    pub snap: Mutex<Snapshot>,
    pub settings: Mutex<Settings>,
    /// Window has focus: poll faster.
    pub focused: AtomicBool,
    /// Set to wake the poller right away (after launching something).
    pub poke: (Mutex<bool>, Condvar),
}

impl Shared {
    pub fn new(settings: Settings) -> Self {
        Self { snap: Mutex::new(Snapshot::default()), settings: Mutex::new(settings), focused: AtomicBool::new(true), poke: (Mutex::new(false), Condvar::new()) }
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
    Restart,
}

pub fn spawn(shared: Arc<Shared>, tx: Sender<Msg>, ctx: egui::Context) {
    std::thread::Builder::new().name("poller".into()).spawn(move || run(shared, tx, ctx)).expect("poller thread");
}

fn run(shared: Arc<Shared>, tx: Sender<Msg>, ctx: egui::Context) {
    let p = core::paths();
    let rando_name = core::rando_exe().and_then(|e| e.file_name().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
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
        }
        let rando_pid = if rando_name.is_empty() { None } else { pid_of(&rando_name) };
        if rando_pid.is_some() {
            core::unlock_randomizer_output(); // a few stat calls; keeps a read-only flag from crashing it
        }

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
