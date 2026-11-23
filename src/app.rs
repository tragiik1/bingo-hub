//! Window, pages and actions.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CursorIcon, Frame, Id, Key, Layout, Order, Rect, ResizeDirection, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, ViewportCommand,
};

use crate::core::{self, Backup, Board, Check, Fix, Level, Run, Settings};
use crate::sys;
use crate::update::{self, Release};
use crate::ui::{self, c, col, col_r, Caption, Kind, Lead};
use crate::worker::{self, GameState, Msg, Shared, Snapshot};

const TITLE: &str = "Bingo Hub";
const TITLEBAR_H: f32 = 40.0;
const SIDEBAR_W: f32 = 212.0;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Race,
    Classes,
    Saves,
    Boards,
    Runs,
    Setup,
}

impl Page {
    const ALL: [Page; 6] = [Page::Race, Page::Classes, Page::Saves, Page::Boards, Page::Runs, Page::Setup];
    fn key(self) -> &'static str {
        match self {
            Page::Race => "Race",
            Page::Classes => "Classes",
            Page::Saves => "Saves",
            Page::Boards => "Boards",
            Page::Runs => "Runs",
            Page::Setup => "Setup",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Page::Runs => "Run log",
            p => p.key(),
        }
    }
    fn from_key(k: &str) -> Page {
        Page::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(k)).unwrap_or(Page::Race)
    }
}

enum Act {
    Nav(Page),
    OpenBoard,
    OpenRando,
    OpenDebug,
    Launch,
    BackupPrompt,
    Restore(Backup),
    Rename(Backup),
    Delete(Backup),
    OpenFolder(PathBuf),
    ImportDownloads,
    AddBoards,
    SetDefault(PathBuf),
    Practice,
    NewRun,
    EditRun(usize),
    DeleteRun(usize),
    Export,
    Fix(Fix),
    Toast(String),
    ApplyGraces,
    Dismiss(String),
    Undismiss(String),
    ResetBingoSave,
    CheckUpdate,
    ShowUpdate,
    InstallUpdate,
    /// Close a game that's stuck in the background with no window.
    EndStuckGame,
}

#[derive(Clone, PartialEq)]
enum Upd {
    Idle,
    Checking,
    Current,
    Available(Release),
    Installing,
    Failed(String),
}

enum Confirmed {
    ResetBingoSave,
    /// Set race graces on this game's character even though it has many already.
    ForceGraces(u32),
    /// Close this game process, which had no window.
    EndGame(u32),
    Restore(Backup),
    Delete(Backup),
    Debug,
    StartSteam,
    /// The run as it was when Delete was pressed (a race can be added to the log meanwhile).
    DeleteRun(Run),
}

enum PromptFor {
    Backup,
    Rename(Backup),
}

enum Modal {
    Confirm { title: String, body: String, yes: String, then: Confirmed },
    Alert { title: String, body: String },
    Prompt { title: String, label: String, value: String, then: PromptFor },
    /// `orig` is the run being edited (None = new), found again by value when saving, since a
    /// race can be added to the log while the dialog is open.
    Run { orig: Option<Run>, run: Run, date: String },
    Update(Release),
}

struct Practice {
    board: usize,
    path: PathBuf,
    n: usize,
    cells: Vec<usize>,
    marked: Vec<bool>,
}

#[derive(Default)]
struct Spoiler {
    open: bool,
    query: String,
    groups: std::collections::HashSet<String>,
}

pub struct App {
    shared: Arc<Shared>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    hwnd: isize,
    page: Page,
    toast: Option<(String, f64)>,
    modal: Option<Modal>,
    seed: String,
    save_sel: Option<PathBuf>,
    boards: Vec<Board>,
    boards_loaded: bool,
    board_sel: usize,
    default_board: Option<String>,
    query: String,
    practice: Option<Practice>,
    runs: Vec<Run>,
    run_sel: Option<usize>,
    checks: Option<Vec<Check>>,
    checks_busy: bool,
    badge: (usize, usize),
    upd: Upd,
    prev_game: GameState,
    show_dismissed: bool,
    /// Classes page spoiler: closed every time the hub starts, so nobody sees it by accident.
    spoiler: Spoiler,
    /// Started right after a hub update: finish any tool updates without asking again.
    auto_tools: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        ui::setup(&cc.egui_ctx);
        let settings = Settings::load();
        let args: Vec<String> = std::env::args().collect();
        let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).map(|k| Page::from_key(k)).unwrap_or(Page::from_key(&settings.last_page));
        let shared = Arc::new(Shared::new(settings.clone()));
        let (tx, rx) = channel();
        worker::spawn(shared.clone(), tx.clone(), cc.egui_ctx.clone());
        let mut app = App {
            shared,
            tx,
            rx,
            hwnd: 0,
            page,
            toast: None,
            modal: None,
            seed: settings.seed.clone(),
            save_sel: None,
            boards: vec![],
            boards_loaded: false,
            board_sel: 0,
            default_board: None,
            query: String::new(),
            practice: None,
            runs: core::load_runs(),
            run_sel: None,
            checks: None,
            checks_busy: false,
            badge: (0, 0),
            upd: Upd::Idle,
            prev_game: GameState::NotRunning,
            show_dismissed: false,
            spoiler: Spoiler::default(),
            auto_tools: args.iter().any(|a| a == "--updated"),
        };
        if args.iter().any(|a| a == "--updated") {
            app.toast = Some((format!("Updated to {}", update::VERSION), 0.0));
        }
        // Check for updates shortly after start, then every six hours.
        if update::enabled() {
            let (tx, ctx) = (app.tx.clone(), cc.egui_ctx.clone());
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let _ = tx.send(Msg::Update(update::check(), false));
                ctx.request_repaint();
                std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
            });
        }
        app.run_checks(&cc.egui_ctx);
        app.load_boards();
        app
    }

    fn settings(&self) -> Settings {
        self.shared.settings.lock().unwrap().clone()
    }

    fn set(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.shared.settings.lock().unwrap();
        f(&mut s);
        s.save();
    }

    fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.toast = Some((text.into(), ctx.input(|i| i.time)));
    }

    fn alert(&mut self, title: &str, body: impl Into<String>) {
        self.modal = Some(Modal::Alert { title: title.into(), body: body.into() });
    }

    fn confirm(&mut self, title: &str, body: impl Into<String>, yes: &str, then: Confirmed) {
        self.modal = Some(Modal::Confirm { title: title.into(), body: body.into(), yes: yes.into(), then });
    }

    /// Run a closure on a worker thread; it reports back through `Msg`.
    fn bg(&self, ctx: &egui::Context, f: impl FnOnce(&Sender<Msg>) + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            f(&tx);
            ctx.request_repaint();
        });
    }

    fn run_checks(&mut self, ctx: &egui::Context) {
        if self.checks_busy {
            return;
        }
        self.checks_busy = true;
        let settings = self.settings();
        let known = self.shared.snap.lock().unwrap().steam;
        self.bg(ctx, move |tx| {
            let mut pc = sys::ProcCache::default();
            let steam = known || {
                pc.refresh();
                pc.pid_of("steam.exe").is_some()
            };
            let _ = tx.send(Msg::Checks(core::run_checks(&settings, steam)));
        });
    }

    fn update_badge(&mut self) {
        let dismissed = self.settings().dismissed;
        let live: Vec<&Check> = self.checks.iter().flatten().filter(|c| !dismissed.contains(&dismiss_key(c))).collect();
        self.badge = (live.iter().filter(|c| c.level == Level::Bad).count(), live.iter().filter(|c| c.level == Level::Warn).count());
    }

    /// Bingo game just started: add it to the run log (unless it's a relaunch of the same race).
    fn auto_log_run(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let (seed, code) = match &snap.rando_run {
            Some(r) => (r.seed.clone(), r.code.clone()),
            None => (self.settings().seed.trim().to_string(), String::new()),
        };
        let recent = self.runs.first().is_some_and(|r| {
            r.result.is_empty() && r.seed == seed && r.date_local().is_some_and(|d| (Local::now() - d).num_hours() < 6)
        });
        if recent {
            return;
        }
        let board = self.default_board.clone().map(|d| core::pretty(&PathBuf::from(d))).unwrap_or_default();
        self.runs.insert(0, Run { date: Local::now().to_rfc3339(), seed, code, board, ..Default::default() });
        core::save_runs(&self.runs);
        self.toast(ctx, "Race added to your run log");
    }

    fn load_boards(&mut self) {
        self.boards = core::list_boards();
        self.default_board = core::board_config_get("LastBingoFile").filter(|s| !s.is_empty());
        self.board_sel = self.board_sel.min(self.boards.len().saturating_sub(1));
        self.boards_loaded = true;
        // Boards re-sort when one is added; a card dealt from a board that changed is closed.
        if let Some(pr) = &mut self.practice {
            match self.boards.iter().position(|b| b.path == pr.path && pr.cells.iter().all(|&g| g < b.goals.len())) {
                Some(i) => {
                    pr.board = i;
                    self.board_sel = i; // "Deal again" deals from the selected board
                }
                None => self.practice = None,
            }
        }
    }

    fn is_default(&self, b: &Board) -> bool {
        self.default_board.as_ref().is_some_and(|d| PathBuf::from(d) == b.path || core::pretty(&PathBuf::from(d)) == b.name)
    }

    fn start_practice(&mut self) {
        let Some(b) = self.boards.get(self.board_sel) else { return };
        let n = core::board_config_get("GS_BoardSize").and_then(|s| s.parse().ok()).filter(|n| (3..=8).contains(n)).unwrap_or(5usize);
        let mut idx: Vec<usize> = (0..b.goals.len()).collect();
        shuffle(&mut idx);
        idx.truncate(n * n);
        self.practice = Some(Practice { board: self.board_sel, path: b.path.clone(), n, marked: vec![false; idx.len()], cells: idx });
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Toast(t) => self.toast(ctx, t),
                Msg::Error(e) => self.alert("Something went wrong", e),
                Msg::Checks(c) => {
                    self.checks = Some(c);
                    self.update_badge();
                    self.checks_busy = false;
                }
                Msg::BoardsChanged => {
                    self.load_boards();
                    self.run_checks(ctx);
                }
                Msg::Update(r, manual) => {
                    let auto = std::mem::take(&mut self.auto_tools);
                    self.upd = match r {
                        Ok(Some(rel)) if auto && rel.hub.is_none() && !rel.tools.is_empty() => {
                            self.upd = Upd::Available(rel);
                            self.install_update(ctx);
                            continue;
                        }
                        Ok(Some(rel)) => Upd::Available(rel),
                        Ok(None) => {
                            if manual {
                                self.toast(ctx, "You're on the latest version");
                            }
                            Upd::Current
                        }
                        Err(e) if manual => Upd::Failed(e),
                        Err(_) => Upd::Idle,
                    };
                }
                Msg::UpdateFailed(e) => {
                    self.upd = Upd::Failed(e.clone());
                    self.alert("Update failed", e);
                }
                Msg::Restart => ctx.send_viewport_cmd(ViewportCommand::Close),
                Msg::Alert(title, body) => self.alert(&title, body),
                Msg::ToolsUpdated(names) => {
                    self.upd = Upd::Current;
                    self.toast(ctx, format!("Updated {}", names.join(", ")));
                    self.load_boards();
                    self.run_checks(ctx);
                }
                Msg::GameLocated(p) => {
                    self.set(|s| s.game_exe_override = Some(p.display().to_string()));
                    self.run_checks(ctx);
                }
                Msg::Graces(pid, r) => self.graces_result(ctx, pid, r),
            }
        }
    }

    fn graces_result(&mut self, ctx: &egui::Context, pid: u32, r: Result<crate::graces::Outcome, String>) {
        use crate::graces::Outcome;
        match r {
            Ok(Outcome::Applied(off)) => self.toast(ctx, format!("Race graces set{}", if off > 0 { format!(", {off} others removed") } else { String::new() })),
            Ok(Outcome::AlreadySet) => self.toast(ctx, "Race graces are already set"),
            Ok(Outcome::NotLoaded) => self.alert("Race graces", "No character yet. Press it again on the name screen (where you set the keepsake) or once you've loaded in."),
            Ok(Outcome::NotFresh(n)) => {
                let backup = if self.settings().auto_backup_on_launch { " Your save was backed up when the game started." } else { "" };
                self.confirm(
                    "Race graces",
                    format!("This character already has {n} graces, so it may not be a bingo character.\n\nSet the race graces anyway? Its other graces are removed, and the game saves that.{backup}"),
                    "Set graces",
                    Confirmed::ForceGraces(pid),
                )
            }
            Ok(Outcome::WriteFailed) => self.alert("Race graces", "The game didn't take the graces. Try again once you're in game; if it keeps failing, use Cheat Engine for now."),
            Err(e) => self.alert("Race graces", e),
        }
    }

    // ------------------------------------------------------------ actions

    fn open_tool(&mut self, pid: Option<u32>, exe: Option<PathBuf>, admin: bool) {
        self.shared.wake();
        if pid.is_some_and(sys::focus_pid) {
            return;
        }
        let Some(exe) = exe.filter(|e| e.is_file()) else {
            return self.alert("Not found", "That tool isn't in your tools folder.");
        };
        if let Err(e) = sys::launch(&exe, &[], admin, false) {
            if !e.is_empty() {
                self.alert("Couldn't open", e);
            }
        }
    }

    fn act(&mut self, ctx: &egui::Context, a: Act, snap: &Snapshot) {
        let p = core::paths();
        match a {
            Act::Nav(pg) => {
                self.page = pg;
                self.set(|s| s.last_page = pg.key().into());
                if pg == Page::Setup {
                    self.run_checks(ctx);
                }
                if pg == Page::Boards {
                    self.load_boards();
                }
            }
            Act::OpenBoard => self.open_tool(snap.board, Some(p.board_exe.clone()), false),
            Act::OpenRando => {
                core::unlock_randomizer_output();
                let seed = self.settings().seed.trim().to_string();
                if !seed.is_empty() {
                    ctx.copy_text(seed.clone());
                    self.toast(ctx, format!("Seed {seed} copied. Paste it into the Randomizer."));
                }
                self.open_tool(snap.rando, core::rando_exe(), false);
            }
            Act::OpenDebug => {
                if self.settings().debug_warning_accepted {
                    let admin = self.settings().debug_tool_as_admin;
                    self.open_tool(snap.debug, Some(p.debug_exe.clone()), admin);
                } else {
                    self.confirm("Debug Tool", "For practice only. Don't use it during a race, and don't take a save you've used it on online.", "Open", Confirmed::Debug);
                }
            }
            Act::Launch => {
                if let Some(pid) = snap.game_pid {
                    if !sys::focus_pid(pid) && snap.game_stuck {
                        return self.act(ctx, Act::EndStuckGame, snap);
                    }
                } else if !p.regulation.is_file() {
                    self.alert("Randomize first", "Open the Randomizer, enter the seed and press Randomize.");
                } else if !p.me2_launcher.is_file() {
                    self.alert("ModEngine 2 is missing", p.me2_launcher.display().to_string());
                } else if !snap.steam {
                    self.confirm("Steam isn't running", "ModEngine 2 needs Steam to launch the game.", "Start Steam", Confirmed::StartSteam);
                } else {
                    let s = self.settings();
                    self.toast(ctx, "Launching Elden Ring");
                    self.shared.wake();
                    self.bg(ctx, move |tx| {
                        if s.auto_backup_on_launch {
                            if let Err(e) = core::create_backup("before launch", true, s.keep_auto_backups) {
                                let _ = tx.send(Msg::Error(format!("Backup failed, so the game wasn't launched.\n\n{e}\n\nTurn off \"Back up save when the game starts\" in Setup to launch anyway.")));
                                return;
                            }
                        }
                        if s.separate_save {
                            if let Err(e) = crate::bingosave::swap_in() {
                                let _ = tx.send(Msg::Error(format!("Couldn't switch to your bingo save, so the game wasn't launched. Your normal save is untouched.\n\n{e}")));
                                return;
                            }
                        }
                        // Same command as launchmod_bingo.bat (the Randomizer's Bingo! button).
                        if let Err(e) = sys::launch(&core::paths().me2_launcher, &["-t", "er", "-c", ".\\config_bingo.toml"], false, true) {
                            let _ = crate::bingosave::swap_out();
                            let _ = tx.send(Msg::Error(e));
                        }
                    });
                }
            }
            Act::BackupPrompt => {
                self.modal = Some(Modal::Prompt { title: "Back up save".into(), label: "Name".into(), value: format!("Backup {}", Local::now().format("%-d %b")), then: PromptFor::Backup });
            }
            Act::Restore(b) => {
                if snap.game_pid.is_some() {
                    self.alert("Close the game first", "Elden Ring has the save open.");
                } else if snap.bingo_save {
                    self.alert("Wait a moment", "Your bingo save is still in the save folder. The hub puts your normal save back a few seconds after the game closes; restore after that.");
                } else {
                    self.confirm("Restore save", format!("Replace your current save with \"{}\" ({})?\n\nYour current save is backed up first.", b.label, core::when(b.when)), "Restore", Confirmed::Restore(b));
                }
            }
            Act::Rename(b) => self.modal = Some(Modal::Prompt { title: "Rename backup".into(), label: "Name".into(), value: b.label.clone(), then: PromptFor::Rename(b) }),
            Act::Delete(b) => self.confirm("Delete backup", format!("Move \"{}\" to the Recycle Bin?", b.label), "Delete", Confirmed::Delete(b)),
            Act::OpenFolder(d) => sys::open_folder(&d),
            Act::ImportDownloads => self.bg(ctx, |tx| {
                let n = core::import_from_downloads();
                let _ = tx.send(Msg::Toast(if n == 0 { "Nothing new in Downloads".into() } else { format!("Imported {n} board{}", if n == 1 { "" } else { "s" }) }));
                let _ = tx.send(Msg::BoardsChanged);
            }),
            Act::AddBoards => {
                let owner = self.hwnd;
                self.bg(ctx, move |tx| {
                    let files = sys::open_files(owner, "Add boards", &[("Goal lists (*.json)", "*.json")], true);
                    if files.is_empty() {
                        return;
                    }
                    let ok = files.iter().filter(|f| core::import_board(f)).count();
                    let skipped = files.len() - ok;
                    let _ = tx.send(Msg::Toast(format!("Added {ok}{}", if skipped > 0 { format!(", skipped {skipped} (duplicate or not a goal list)") } else { String::new() })));
                    let _ = tx.send(Msg::BoardsChanged);
                });
            }
            Act::SetDefault(path) => match core::board_config_set("LastBingoFile", &path.display().to_string(), snap.board.is_some()) {
                Ok(()) => {
                    self.toast(ctx, format!("{} is the Board's default", core::pretty(&path)));
                    self.load_boards();
                }
                Err(e) => self.alert("Couldn't set default", e),
            },
            Act::Practice => self.start_practice(),
            Act::NewRun => {
                let board = self.default_board.clone().map(|d| core::pretty(&PathBuf::from(d))).unwrap_or_default();
                let now = Local::now();
                let run = Run { date: now.to_rfc3339(), seed: self.settings().seed, board, ..Default::default() };
                self.modal = Some(Modal::Run { orig: None, date: now.format("%Y-%m-%d %H:%M").to_string(), run });
            }
            Act::EditRun(i) => {
                if let Some(r) = self.runs.get(i) {
                    let date = r.date_local().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                    self.modal = Some(Modal::Run { orig: Some(r.clone()), run: r.clone(), date });
                }
            }
            Act::DeleteRun(i) => {
                if let Some(r) = self.runs.get(i).cloned() {
                    let d = r.date_local().map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default();
                    self.confirm("Delete run", format!("Delete the run from {d}?"), "Delete", Confirmed::DeleteRun(r));
                }
            }
            Act::Export => {
                let owner = self.hwnd;
                let csv = core::runs_csv(&self.runs);
                self.bg(ctx, move |tx| {
                    let name = format!("bingo-runs-{}.csv", Local::now().format("%Y-%m-%d"));
                    if let Some(path) = sys::save_file(owner, "Export run log", &[("CSV (*.csv)", "*.csv")], &name, "csv") {
                        let _ = tx.send(match std::fs::write(&path, csv) {
                            Ok(()) => Msg::Toast("Exported".into()),
                            Err(e) => Msg::Error(e.to_string()),
                        });
                    }
                });
            }
            Act::Toast(t) => self.toast(ctx, t),
            Act::Dismiss(k) => {
                self.set(|s| s.dismissed.push(k));
                self.update_badge();
            }
            Act::Undismiss(k) => {
                self.set(|s| s.dismissed.retain(|d| *d != k));
                self.update_badge();
            }
            Act::ResetBingoSave => self.confirm(
                "New bingo save",
                "Your next bingo game starts with an empty save. The current bingo save goes to the Recycle Bin. Your normal save isn't touched.",
                "Start fresh",
                Confirmed::ResetBingoSave,
            ),
            Act::ApplyGraces => {
                if let (GameState::Modded, Some(pid)) = (snap.game, snap.game_pid) {
                    self.bg(ctx, move |tx| {
                        let _ = tx.send(Msg::Graces(pid, crate::graces::apply_now(pid, false)));
                    });
                } else {
                    self.alert("Race graces", "Launch a bingo game first. Graces can be set from the name screen onwards.");
                }
            }
            Act::CheckUpdate => {
                self.upd = Upd::Checking;
                self.bg(ctx, |tx| {
                    let _ = tx.send(Msg::Update(update::check(), true));
                });
            }
            Act::ShowUpdate => {
                if let Upd::Available(r) = &self.upd {
                    self.modal = Some(Modal::Update(r.clone()));
                }
            }
            Act::InstallUpdate => self.install_update(ctx),
            Act::EndStuckGame => {
                if let (true, Some(pid)) = (snap.game_stuck, snap.game_pid) {
                    self.confirm(
                        "Close Elden Ring",
                        "Elden Ring is still running in the background with no window, usually left over from a crash or a slow exit. It stops a new game from starting.

Close it?",
                        "Close it",
                        Confirmed::EndGame(pid),
                    );
                }
            }
            Act::Fix(f) => {
                match f {
                    Fix::OpenUrl(u) => sys::shell_open(&u),
                    Fix::OpenFolder(d) => sys::open_folder(&d),
                    Fix::OpenRandomizer => return self.act(ctx, Act::OpenRando, snap),
                    Fix::BackupNow => return self.act(ctx, Act::BackupPrompt, snap),
                    Fix::ImportDownloads => return self.act(ctx, Act::ImportDownloads, snap),
                    Fix::StartSteam => start_steam(),
                    Fix::FixBoardPath => {
                        let dir = snap.game_exe.as_ref().and_then(|e| e.parent()).map(|d| d.display().to_string());
                        match dir.map(|d| core::board_config_set("GamePath", &d, snap.board.is_some())) {
                            Some(Ok(())) => self.toast(ctx, "Board game path updated"),
                            Some(Err(e)) => self.alert("Couldn't update", e),
                            None => self.alert("Elden Ring not found", "Locate the game first."),
                        }
                    }
                    Fix::LocateGame => {
                        let owner = self.hwnd;
                        self.bg(ctx, move |tx| {
                            if let Some(p) = sys::open_files(owner, "Find eldenring.exe", &[("eldenring.exe", "eldenring.exe")], false).into_iter().next() {
                                let _ = tx.send(Msg::GameLocated(p));
                            }
                        });
                    }
                }
                self.run_checks(ctx);
            }
        }
    }

    /// Tools first (they don't need a restart), then the hub, which restarts itself.
    fn install_update(&mut self, ctx: &egui::Context) {
        let Upd::Available(r) = self.upd.clone() else { return };
        self.upd = Upd::Installing;
        self.bg(ctx, move |tx| {
            if !r.tools.is_empty() {
                match update::install_tools(&r.tools) {
                    Err(e) => {
                        let _ = tx.send(Msg::UpdateFailed(e));
                        return;
                    }
                    Ok(names) if r.hub.is_none() => {
                        let _ = tx.send(Msg::ToolsUpdated(names));
                        return;
                    }
                    Ok(_) => {}
                }
            }
            if let Some((url, size)) = &r.hub {
                let _ = tx.send(match update::install_hub(url, *size) {
                    Ok(()) => Msg::Restart,
                    Err(e) => Msg::UpdateFailed(e),
                });
            }
        });
    }

    fn confirmed(&mut self, ctx: &egui::Context, then: Confirmed, snap: &Snapshot) {
        let keep = self.settings().keep_auto_backups;
        match then {
            Confirmed::Restore(b) => self.bg(ctx, move |tx| {
                let _ = tx.send(match core::restore_backup(&b, keep) {
                    Ok(()) => Msg::Toast(format!("Restored \"{}\"", b.label)),
                    Err(e) => Msg::Error(e),
                });
            }),
            Confirmed::Delete(b) => {
                if let Err(e) = sys::recycle(&b.path) {
                    self.alert("Couldn't delete", e);
                }
            }
            Confirmed::Debug => {
                self.set(|s| s.debug_warning_accepted = true);
                self.act(ctx, Act::OpenDebug, snap);
            }
            Confirmed::StartSteam => start_steam(),
            Confirmed::ForceGraces(pid) => {
                if snap.game_pid == Some(pid) {
                    self.bg(ctx, move |tx| {
                        let _ = tx.send(Msg::Graces(pid, crate::graces::apply_now(pid, true)));
                    });
                } else {
                    self.alert("Race graces", "That game has closed. Nothing was changed.");
                }
            }
            Confirmed::EndGame(pid) => {
                // Only the same process, and only if it still has no window.
                if snap.game_pid != Some(pid) || sys::has_window(pid) {
                    self.alert("Close Elden Ring", "It isn't stuck any more, so nothing was closed.");
                } else {
                    match sys::end_process(pid) {
                        Ok(()) => {
                            self.toast(ctx, "Closed Elden Ring");
                            self.shared.wake();
                        }
                        Err(e) => self.alert("Couldn't close Elden Ring", e),
                    }
                }
            }
            Confirmed::ResetBingoSave => match crate::bingosave::reset_bingo_save() {
                Ok(()) => self.toast(ctx, "Next bingo game starts with a fresh save"),
                Err(e) => self.alert("Couldn't reset", e),
            },
            Confirmed::DeleteRun(run) => {
                if let Some(i) = self.runs.iter().position(|r| *r == run) {
                    self.runs.remove(i);
                    core::save_runs(&self.runs);
                    self.run_sel = None;
                }
            }
        }
    }
}

fn start_steam() {
    match sys::steam_path().map(|s| s.join("steam.exe")).filter(|e| e.is_file()) {
        Some(exe) => {
            let _ = sys::launch(&exe, &[], false, false);
        }
        None => sys::shell_open("steam://open/main"),
    }
}

fn shuffle(v: &mut [usize]) {
    let mut x = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u64) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

fn dismiss_key(c: &Check) -> String {
    format!("{}|{}", c.title, c.detail)
}


// ================================================================ frame

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        c::BG.to_normalized_gamma_f32()
    }

    /// Debug builds only: BINGOHUB_HOVER="x,y" puts a pretend pointer there, so a screenshot can show
    /// hover states without touching the real mouse.
    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some((x, y)) = std::env::var("BINGOHUB_HOVER").ok().and_then(|v| v.split_once(',').map(|(a, b)| (a.trim().parse().ok(), b.trim().parse().ok()))) {
            if let (Some(x), Some(y)) = (x, y) {
                raw.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Debug builds only: BINGOHUB_SHOT=path.png saves the window's own frame after BINGOHUB_SHOT_AFTER seconds (3) and exits.
        #[cfg(debug_assertions)]
        if let Ok(path) = std::env::var("BINGOHUB_SHOT") {
            static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            ctx.request_repaint();
            let after: f64 = std::env::var("BINGOHUB_SHOT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0);
            if ctx.input(|i| i.time) > after && !SENT.swap(true, std::sync::atomic::Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = shot {
                let f = std::fs::File::create(&path).unwrap();
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.size[0] as u32, img.size[1] as u32);
                enc.set_color(png::ColorType::Rgba);
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                enc.write_header().unwrap().write_image_data(&bytes).unwrap();
                std::process::exit(0);
            }
        }
        if self.hwnd == 0 {
            self.hwnd = sys::own_window(TITLE);
            sys::style_window(self.hwnd);
        }
        let focused = ctx.input(|i| i.focused);
        if self.shared.focused.swap(focused, std::sync::atomic::Ordering::Relaxed) != focused && focused {
            self.shared.wake();
        }
        self.drain(ctx);
        let snap = self.shared.snap.lock().unwrap().clone();
        let mut acts: Vec<Act> = Vec::new();
        if snap.game == GameState::Modded && self.prev_game != GameState::Modded {
            self.auto_log_run(ctx, &snap);
        }
        self.prev_game = snap.game;

        // Maximized borderless windows hang off-screen by the frame width: pad by exactly that.
        let [ol, ot, or, ob] = if maximized(ctx) {
            let ppp = ctx.pixels_per_point();
            sys::maximized_overhang(self.hwnd).map(|v| (v / ppp).round().min(60.0) as i8)
        } else {
            [0; 4]
        };
        let side_margin = egui::Margin { left: ol, right: 0, top: ot, bottom: ob };
        let main_margin = egui::Margin { left: 0, right: or, top: ot, bottom: ob };
        egui::SidePanel::left("side").exact_width(SIDEBAR_W + ol as f32).resizable(false).show_separator_line(false).frame(Frame::NONE.fill(c::SIDEBAR).inner_margin(side_margin)).show(ctx, |ui| {
            self.sidebar(ui, &snap, &mut acts);
        });
        egui::CentralPanel::default().frame(Frame::NONE.fill(c::BG).inner_margin(main_margin)).show(ctx, |ui| {
            let full = ui.max_rect();
            titlebar(ui, Rect::from_min_size(full.min, vec2(full.width(), TITLEBAR_H)), true);
            let content = Rect::from_min_max(pos2(full.left(), full.top() + TITLEBAR_H), full.max);
            ui.scope_builder(UiBuilder::new().max_rect(content), |ui| match self.page {
                Page::Race => self.race(ui, &snap, &mut acts),
                Page::Saves => self.saves(ui, &snap, &mut acts),
                Page::Boards => self.boards_page(ui, &mut acts),
                Page::Classes => self.classes_page(ui, &snap),
                Page::Runs => self.runs_page(ui, &mut acts),
                Page::Setup => self.setup(ui, &snap, &mut acts),
            });
            self.toast_ui(ui, content);
        });

        if self.page == Page::Boards {
            let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
            if !dropped.is_empty() {
                let ok = dropped.iter().filter(|p| core::import_board(p)).count();
                self.toast(ctx, format!("Added {ok}"));
                self.load_boards();
            }
        }

        self.modal_ui(ctx, &snap, &mut acts);
        for a in acts {
            self.act(ctx, a, &snap);
        }
        resize_edges(ctx);
    }
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Draggable strip; the right one also carries the window buttons.
fn titlebar(ui: &mut Ui, rect: Rect, buttons: bool) {
    let ctx = ui.ctx().clone();
    let drag = ui.interact(rect, Id::new(("drag", rect.left() as i32)), Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(&ctx)));
    }
    if !buttons {
        return;
    }
    let w = 46.0;
    let close = Rect::from_min_size(pos2(rect.right() - w, rect.top()), vec2(w, rect.height()));
    let max = close.translate(vec2(-w, 0.0));
    let min = max.translate(vec2(-w, 0.0));
    if ui::caption_button(ui, min, Caption::Min).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    let is_max = maximized(&ctx);
    if ui::caption_button(ui, max, if is_max { Caption::Restore } else { Caption::Max }).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_max));
    }
    if ui::caption_button(ui, close, Caption::Close).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

/// Borderless window: resize from the outer 5px.
fn resize_edges(ctx: &egui::Context) {
    if maximized(ctx) {
        return;
    }
    let r = ctx.content_rect();
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let m = 5.0;
    let (l, rt, t, b) = (p.x < r.left() + m, p.x > r.right() - m, p.y < r.top() + m, p.y > r.bottom() - m);
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => Some((ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest)),
        (_, true, true, _) => Some((ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast)),
        (true, _, _, true) => Some((ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest)),
        (_, true, _, true) => Some((ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast)),
        (true, ..) => Some((ResizeDirection::West, CursorIcon::ResizeWest)),
        (_, true, ..) => Some((ResizeDirection::East, CursorIcon::ResizeEast)),
        (_, _, true, _) => Some((ResizeDirection::North, CursorIcon::ResizeNorth)),
        (.., true) => Some((ResizeDirection::South, CursorIcon::ResizeSouth)),
        _ => None,
    };
    if let Some((d, icon)) = dir {
        ctx.set_cursor_icon(icon);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
        }
    }
}

/// Page content area with the standard margins; `max_w` caps the readable width.
fn page_area<R>(ui: &mut Ui, max_w: Option<f32>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = ui.max_rect();
    let left = r.left() + ui::PAD_X;
    let right = match max_w {
        Some(w) => (left + w).min(r.right() - ui::PAD_X),
        None => r.right() - ui::PAD_X,
    };
    let inner = Rect::from_min_max(pos2(left, r.top() + 14.0), pos2(right, r.bottom() - 28.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), add).inner
}

fn toolbar(ui: &mut Ui, acts: &mut Vec<Act>, left: impl FnOnce(&mut Ui, &mut Vec<Act>), right: impl FnOnce(&mut Ui, &mut Vec<Act>)) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, 32.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        left(ui, acts);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            right(ui, acts);
        });
    });
}

impl App {
    // ------------------------------------------------------------ sidebar

    fn sidebar(&mut self, ui: &mut Ui, snap: &Snapshot, acts: &mut Vec<Act>) {
        let r = ui.max_rect();
        ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0_f32, c::DIVIDER));
        titlebar(ui, Rect::from_min_size(r.min, vec2(r.width() - 1.0, TITLEBAR_H)), false);
        // Logo: a bingo card filling in. It only moves while the hub has focus, so it never costs
        // anything while the game is being played; otherwise it rests on a finished row.
        let (ctx, painter) = (ui.ctx().clone(), ui.painter().clone());
        let (now, focused) = ctx.input(|i| (i.time, i.focused));
        if focused {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        let mark = pos2(r.left() + 16.0, r.top() + 9.0);
        crate::art::logo(&painter, mark, 1.0, if focused { now } else { crate::art::REST_T });
        ui::wordmark(ui, pos2(mark.x + 32.0 + 9.0, mark.y + 16.0));

        let mut y = r.top() + 70.0;
        for pg in Page::ALL {
            let item = Rect::from_min_size(pos2(r.left() + 12.0, y), vec2(r.width() - 25.0, 34.0));
            let badge = if pg == Page::Setup && self.badge.0 + self.badge.1 > 0 { Some(((self.badge.0 + self.badge.1).to_string(), if self.badge.0 > 0 { c::BAD } else { c::WARN })) } else { None };
            if nav_item(ui, item, pg.label(), self.page == pg, badge, ("nav", pg.key())).clicked() && self.page != pg {
                acts.push(Act::Nav(pg));
            }
            y += 36.0;
        }

        // Tools, pinned to the bottom.
        let tools: [(&str, String, Color32, Act); 4] = [
            ("Bingo Board", if snap.board.is_some() { "Open".into() } else { String::new() }, c::MUTED, Act::OpenBoard),
            ("Randomizer", if snap.rando.is_some() { "Open".into() } else { String::new() }, c::MUTED, Act::OpenRando),
            match snap.game {
                GameState::Modded => ("Elden Ring", "Modded".into(), c::GOOD, Act::Launch),
                GameState::Vanilla => ("Elden Ring", "No mod".into(), c::BAD, Act::Launch),
                GameState::Starting => ("Elden Ring", "Starting".into(), c::MUTED, Act::Launch),
                GameState::Unknown => ("Elden Ring", "Running".into(), c::WARN, Act::Launch),
                GameState::NotRunning => ("Elden Ring", String::new(), c::MUTED, Act::Launch),
            },
            ("Debug Tool", if snap.debug.is_some() { "Open".into() } else { String::new() }, c::MUTED, Act::OpenDebug),
        ];
        let mut y = r.bottom() - 20.0 - tools.len() as f32 * 30.0;
        let notice = match &self.upd {
            Upd::Available(rel) => Some(("Update available", if rel.hub.is_some() { format!("Version {}", rel.version) } else { rel.summary() }, true)),
            Upd::Installing => Some(("Updating", "Restarting shortly".to_string(), false)),
            _ => None,
        };
        if let Some((head, sub, clickable)) = notice {
            let card = Rect::from_min_size(pos2(r.left() + 12.0, y - 46.0 - 58.0), vec2(r.width() - 25.0, 52.0));
            let resp = ui.interact(card, Id::new("update_card"), if clickable { Sense::click() } else { Sense::hover() });
            let t = ui.ctx().animate_bool_with_time(resp.id, clickable && resp.hovered(), 0.08);
            ui.painter().rect_filled(card, 8.0, c::SURFACE.lerp_to_gamma(c::RAISED, t));
            ui.painter().rect_stroke(card, 8.0, Stroke::new(1.0_f32, c::ACCENT.gamma_multiply(0.45)), StrokeKind::Inside);
            ui::text_at(ui, pos2(card.left() + 12.0, card.top() + 17.0), Align2::LEFT_CENTER, head, ui::strong(), c::TEXT);
            ui::text_at(ui, pos2(card.left() + 12.0, card.top() + 36.0), Align2::LEFT_CENTER, &sub, ui::small(), c::ACCENT);
            if !clickable {
                ui::spinner(ui, pos2(card.right() - 20.0, card.center().y));
            }
            if clickable && resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                acts.push(Act::ShowUpdate);
            }
        }
        ui::text_at(ui, pos2(r.left() + 24.0, y - 16.0), Align2::LEFT_CENTER, "Tools", ui::small(), c::FAINT);
        for (name, state, color, act) in tools {
            let item = Rect::from_min_size(pos2(r.left() + 12.0, y), vec2(r.width() - 25.0, 30.0));
            let resp = ui.interact(item, Id::new(("tool", name)), Sense::click());
            let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
            if t > 0.0 {
                ui.painter().rect_filled(item, 6.0, c::SURFACE.gamma_multiply(t));
            }
            ui::text_at(ui, pos2(item.left() + 12.0, item.center().y), Align2::LEFT_CENTER, name, ui::small(), c::MUTED.lerp_to_gamma(c::TEXT, t));
            ui::text_at(ui, pos2(item.right() - 12.0, item.center().y), Align2::RIGHT_CENTER, &state, ui::small(), color);
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                acts.push(act);
            }
            y += 30.0;
        }
    }

    // ------------------------------------------------------------ race

    fn race(&mut self, ui: &mut Ui, snap: &Snapshot, acts: &mut Vec<Act>) {
        let settings = self.settings();
        egui::ScrollArea::vertical().id_salt("race").auto_shrink([false, false]).show(ui, |ui| {
            page_area(ui, Some(820.0), |ui| {
                ui::title(ui, "Race");
                ui::section(ui, "Seed", 0.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let changed = ui::field(ui, "seed", &mut self.seed, "Optional: checks everyone used the same seed", 340.0, 60).changed();
                    ui.add_space(8.0);
                    if ui::button(ui, "Copy", Kind::Ghost, !self.seed.trim().is_empty()).clicked() {
                        ui.ctx().copy_text(self.seed.trim().to_string());
                        self.toast = Some(("Seed copied".into(), ui.ctx().input(|i| i.time)));
                    }
                    if changed {
                        let seed = self.seed.trim().to_string();
                        self.set(|s| {
                            s.seed = seed;
                            s.seed_set_utc = Utc::now().to_rfc3339();
                        });
                    }
                });

                ui::section(ui, "Checklist", 30.0);
                let reg = snap.regulation.map(core::to_local);
                let seed = settings.seed.trim().to_string();
                ui::card(ui, |ui| {
                    let (lead, detail) = if snap.board.is_some() { (Lead::Check, "Open") } else { (Lead::Num("1"), "Not open") };
                    ui::row(ui, lead, "Bingo Board", detail, c::MUTED, false, false, |ui| {
                        if ui::button(ui, if snap.board.is_some() { "Show" } else { "Open" }, Kind::Secondary, true).clicked() {
                            acts.push(Act::OpenBoard);
                        }
                    });

                    // With the Randomizer's own record we know the exact seed; otherwise fall back to timestamps.
                    let (lead, detail, color, code) = match (&snap.rando_run, reg) {
                        (_, None) => (Lead::Num("2"), "Not randomized yet".to_string(), c::MUTED, None),
                        (Some(run), Some(_)) if !seed.is_empty() && run.seed.trim() != seed => (
                            Lead::Dot(c::WARN),
                            format!("Randomized with \"{}\", not \"{}\". Randomize again.", run.seed, seed),
                            c::WARN,
                            None,
                        ),
                        (Some(run), Some(_)) => (
                            Lead::Check,
                            format!("Seed \"{}\"   ·   race code {}", run.seed, run.code),
                            c::MUTED,
                            Some(format!("Race code {} (seed \"{}\")", run.code, run.seed)),
                        ),
                        (None, Some(t)) => (Lead::Check, format!("Randomized {}", core::when(t.naive_local()).to_lowercase()), c::MUTED, None),
                    };
                    ui::row(ui, lead, "Randomize", &detail, color, false, true, |ui| {
                        if ui::button(ui, if snap.rando.is_some() { "Show" } else { "Open" }, Kind::Secondary, true).clicked() {
                            acts.push(Act::OpenRando);
                        }
                        if let Some(line) = &code {
                            if ui::button(ui, "Copy code", Kind::Ghost, true).clicked() {
                                ui.ctx().copy_text(line.clone());
                                acts.push(Act::Toast("Race code copied".into()));
                            }
                        }
                    });

                    let normal_save_in_bingo = snap.game == GameState::Modded && settings.separate_save && !snap.bingo_save;
                    let (lead, detail, color) = match snap.game {
                        _ if snap.game_stuck => (Lead::Dot(c::WARN), "Still running in the background with no window, so a new game can't start.".to_string(), c::WARN),
                        _ if normal_save_in_bingo => (
                            Lead::Dot(c::WARN),
                            "Using your normal save. Close the game and launch from here to use your bingo save.".to_string(),
                            c::WARN,
                        ),
                        GameState::Modded => {
                            let mut d = String::from("Running with the bingo mod");
                            if snap.bingo_save {
                                d += "   ·   bingo save";
                            }
                            if snap.graces == crate::graces::Status::Set {
                                d += "   ·   race graces set";
                            } else if snap.hotkey == sys::hotkey::ON {
                                d += &format!("   ·   {} in game sets race graces", sys::HOTKEY);
                            }
                            (Lead::Check, d, c::MUTED)
                        }
                        GameState::Starting => (Lead::Spin, "Starting".into(), c::MUTED),
                        GameState::Vanilla => (Lead::Dot(c::BAD), "Running without the mod. Close it and launch from here.".into(), c::BAD),
                        GameState::Unknown => (Lead::Dot(c::WARN), "Running, mod not confirmed".into(), c::MUTED),
                        GameState::NotRunning => (
                            Lead::Num("3"),
                            if reg.is_none() { "Randomize first".into() } else if snap.steam { "Ready".into() } else { "Steam isn't running".into() },
                            c::MUTED,
                        ),
                    };
                    let running = snap.game != GameState::NotRunning;
                    ui::row(ui, lead, "Launch Elden Ring", &detail, color, false, true, |ui| {
                        if snap.game_stuck {
                            if ui::button(ui, "Close it", Kind::Primary, true).clicked() {
                                acts.push(Act::EndStuckGame);
                            }
                        } else if ui::button(ui, if running { "Show" } else { "Launch" }, Kind::Primary, running || reg.is_some()).clicked() {
                            acts.push(Act::Launch);
                        }
                    });
                });

                // Saves at a glance; backups happen by themselves.
                ui.add_space(14.0);
                let mut parts: Vec<String> = Vec::new();
                if settings.separate_save {
                    parts.push(if snap.bingo_save { "Bingo save in use".into() } else { "Separate bingo save on".into() });
                }
                parts.push(match snap.backups.first() {
                    Some(b) => format!("Last backup {}", core::when(b.when).to_lowercase()),
                    None => "No backups yet".into(),
                });
                ui::sub(ui, &parts.join("   ·   "));
            });
        });
    }

    // ------------------------------------------------------------ classes

    fn classes_page(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let mut area = egui::ScrollArea::vertical().id_salt("classes").auto_shrink([false, false]);
        // Debug builds only: BINGOHUB_SCROLL=n opens the page scrolled down, for screenshots.
        #[cfg(debug_assertions)]
        if let Some(n) = std::env::var("BINGOHUB_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
            area = area.vertical_scroll_offset(n);
        }
        area.show(ui, |ui| {
            page_area(ui, Some(1000.0), |ui| {
                ui::title(ui, "Classes");
                let Some(sp) = &snap.classes else {
                    ui::sub(ui, "Randomize to see this seed's starting classes.");
                    return;
                };
                ui.add_space(-20.0);
                let stale = snap.rando_run.as_ref().is_some_and(|r| r.seed != sp.seed);
                ui::sub(ui, &format!("Seed \"{}\"{}", sp.seed, if stale { "   ·   from an earlier randomize" } else { "" }));
                ui.add_space(20.0);
                let gap = 14.0;
                let w = ((ui.available_width() - gap) / 2.0).floor();
                for pair in sp.classes.chunks(2) {
                    // Both cards in a row get the same number of item slots, so they line up.
                    let slots = pair.iter().map(|c| c.items.len()).max().unwrap_or(0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for class in pair {
                            class_card(ui, class, w, slots);
                        }
                    });
                    ui.add_space(gap);
                }
                if !sp.swaps.is_empty() {
                    ui.add_space(10.0);
                    spoiler(ui, &sp.swaps, &mut self.spoiler);
                }
                // Room under the last card so the window edge doesn't sit on it.
                ui.add_space(36.0);
            });
        });
    }

    // ------------------------------------------------------------ saves

    fn saves(&mut self, ui: &mut Ui, snap: &Snapshot, acts: &mut Vec<Act>) {
        page_area(ui, None, |ui| {
            ui::title(ui, "Saves");
            ui.add_space(-20.0);
            let sub = match snap.live_save {
                None => "No save file found. Start the game once to create it.".to_string(),
                Some(t) => format!(
                    "ER0000.sl2, last written {}{}",
                    core::when(core::to_local(t).naive_local()).to_lowercase(),
                    if snap.game_pid.is_some() { ". Close the game before restoring." } else { "" }
                ),
            };
            ui::sub(ui, &sub);
            ui.add_space(20.0);

            let sel = self.save_sel.as_ref().and_then(|p| snap.backups.iter().position(|b| &b.path == p));
            let selected = sel.map(|i| snap.backups[i].clone());
            toolbar(
                ui,
                acts,
                |ui, acts| {
                    if ui::button(ui, "Back up", Kind::Primary, true).clicked() {
                        acts.push(Act::BackupPrompt);
                    }
                    for (label, f) in [("Restore", Act::Restore as fn(Backup) -> Act), ("Rename", Act::Rename), ("Delete", Act::Delete)] {
                        if ui::button(ui, label, Kind::Secondary, selected.is_some()).clicked() {
                            acts.push(f(selected.clone().unwrap()));
                        }
                    }
                },
                |ui, acts| {
                    if ui::button(ui, "Open folder", Kind::Ghost, true).clicked() {
                        acts.push(Act::OpenFolder(core::paths().backups_dir.clone()));
                    }
                },
            );
            ui.add_space(16.0);

            let rect = ui.available_rect_before_wrap();
            let b = &snap.backups;
            let out = ui::table(ui, "saves", rect, &[col("Name", 5.0), col("Created", 2.0), col_r("Size", 1.0)], b.len(), sel, "No backups yet", |r, ci| match ci {
                0 => (b[r].label.clone(), if b[r].auto { c::MUTED } else { c::TEXT }),
                1 => (core::when(b[r].when), c::MUTED),
                _ => (format!("{:.1} MB", b[r].size as f64 / 1_048_576.0), c::MUTED),
            });
            if let Some(i) = out.clicked {
                self.save_sel = Some(b[i].path.clone());
            }
            if let Some(i) = out.double {
                acts.push(Act::Restore(b[i].clone()));
            }
            if let Some(s) = selected {
                if self.modal.is_none() && ui.input(|i| i.key_pressed(Key::Delete)) {
                    acts.push(Act::Delete(s));
                }
            }
        });
    }

    // ------------------------------------------------------------ boards

    fn boards_page(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        if !self.boards_loaded {
            self.load_boards();
        }
        page_area(ui, None, |ui| {
            ui::title(ui, "Boards");
            let sel = self.boards.get(self.board_sel).cloned();

            if let Some(pr) = &mut self.practice {
                let board = &self.boards[pr.board];
                let mut close = false;
                let mut deal = false;
                toolbar(
                    ui,
                    acts,
                    |ui, _| {
                        deal = ui::button(ui, "Deal again", Kind::Secondary, true).clicked();
                        if ui::button(ui, "Copy", Kind::Ghost, true).clicked() {
                            let text: String = pr.cells.iter().enumerate().map(|(i, g)| format!("{}{}  {}\n", (b'A' + (i % pr.n) as u8) as char, i / pr.n + 1, board.goals[*g].name)).collect();
                            ui.ctx().copy_text(text);
                        }
                    },
                    |ui, _| close = ui::button(ui, "Close", Kind::Ghost, true).clicked(),
                );
                ui.add_space(16.0);
                let area = ui.available_rect_before_wrap();
                let side = area.width().min(area.height());
                let grid = Rect::from_min_size(area.min, vec2(area.width(), side));
                let gap = 4.0;
                let cw = (grid.width() - gap * (pr.n as f32 - 1.0)) / pr.n as f32;
                let ch = (grid.height() - gap * (pr.n as f32 - 1.0)) / pr.n as f32;
                for (i, g) in pr.cells.iter().enumerate() {
                    let (x, y) = ((i % pr.n) as f32, (i / pr.n) as f32);
                    let r = Rect::from_min_size(pos2(grid.left() + x * (cw + gap), grid.top() + y * (ch + gap)), vec2(cw, ch));
                    let resp = ui.interact(r, Id::new(("cell", i)), Sense::click());
                    if resp.clicked() {
                        pr.marked[i] = !pr.marked[i];
                    }
                    let on = ui.ctx().animate_bool_with_time(resp.id, pr.marked[i], 0.12);
                    let hov = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.08);
                    ui.painter().rect_filled(r, 6.0, c::SURFACE.lerp_to_gamma(c::RAISED, hov * 0.6).lerp_to_gamma(c::ACCENT, on));
                    let color = c::TEXT.lerp_to_gamma(c::ON_ACCENT, on);
                    let mut job = egui::text::LayoutJob::simple(board.goals[*g].name.clone(), ui::body(), color, cw - 24.0);
                    job.halign = Align::Center;
                    let gal = ui.fonts_mut(|f| f.layout_job(job));
                    ui.painter().galley(pos2(r.center().x, r.center().y - gal.size().y / 2.0), gal, color);
                    resp.on_hover_cursor(CursorIcon::PointingHand);
                }
                if close {
                    self.practice = None;
                } else if deal {
                    self.start_practice();
                }
                return;
            }

            let is_default = sel.as_ref().is_some_and(|b| self.is_default(b));
            toolbar(
                ui,
                acts,
                |ui, acts| {
                    if ui::button(ui, "Import from Downloads", Kind::Secondary, true).clicked() {
                        acts.push(Act::ImportDownloads);
                    }
                    if ui::button(ui, "Add file", Kind::Secondary, true).clicked() {
                        acts.push(Act::AddBoards);
                    }
                },
                |ui, acts| {
                    if ui::button(ui, if is_default { "Board default" } else { "Set as default" }, Kind::Ghost, sel.is_some() && !is_default).clicked() {
                        acts.push(Act::SetDefault(sel.clone().unwrap().path));
                    }
                    if ui::button(ui, "Practice card", Kind::Ghost, sel.is_some()).clicked() {
                        acts.push(Act::Practice);
                    }
                    // The Board's own file picker needs this folder; open it to copy the path.
                    if ui::button(ui, "Open folder", Kind::Ghost, true).clicked() {
                        acts.push(Act::OpenFolder(core::paths().boards_dir.clone()));
                    }
                },
            );
            ui.add_space(16.0);

            let area = ui.available_rect_before_wrap();
            let split = (area.width() * 0.34).round();
            let left = Rect::from_min_max(area.min, pos2(area.left() + split - 6.0, area.bottom()));
            let right = Rect::from_min_max(pos2(area.left() + split + 6.0, area.top()), area.max);
            let n = self.boards.len();
            ui::text_at(ui, pos2(left.left() + 1.0, left.top() + 16.0), Align2::LEFT_CENTER, &format!("{n} board{}", if n == 1 { "" } else { "s" }), ui::small(), c::MUTED);
            ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(right.min, vec2(right.width(), 32.0))), |ui| {
                ui::field(ui, "goal_search", &mut self.query, "Search goals", right.width(), 100);
            });

            let boards = &self.boards;
            let out = ui::table(ui, "boards", Rect::from_min_max(pos2(left.left(), left.top() + 44.0), left.max), &[col("Board", 5.0), col_r("Goals", 1.4)], n, Some(self.board_sel), "No boards yet", |r, ci| {
                if ci == 0 { (boards[r].name.clone(), c::TEXT) } else { (boards[r].goals.len().to_string(), c::MUTED) }
            });
            if let Some(i) = out.clicked {
                self.board_sel = i;
            }

            let q = self.query.trim().to_lowercase();
            let goals: Vec<(usize, &str)> = sel_goals(boards.get(self.board_sel), &q);
            ui::table(ui, "goals", Rect::from_min_max(pos2(right.left(), right.top() + 44.0), right.max), &[col_r("", 0.9), col("Goal", 10.0)], goals.len(), None, if q.is_empty() { "" } else { "No matching goals" }, |r, ci| {
                if ci == 0 { ((goals[r].0 + 1).to_string(), c::FAINT) } else { (goals[r].1.to_string(), c::TEXT) }
            });
        });
    }

    // ------------------------------------------------------------ runs

    fn runs_page(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        page_area(ui, None, |ui| {
            ui::title(ui, "Run log");
            ui.add_space(-20.0);
            let [w, l, d] = ["Win", "Loss", "Draw"].map(|k| self.runs.iter().filter(|r| r.result == k).count());
            let decided = w + l + d;
            let stats = if self.runs.is_empty() {
                "No runs yet".to_string()
            } else {
                let mut s = format!("{} run{}      {w}W  {l}L  {d}D", self.runs.len(), if self.runs.len() == 1 { "" } else { "s" });
                if decided > 0 {
                    s += &format!("      {:.0}% won", 100.0 * w as f64 / decided as f64);
                }
                s
            };
            ui::sub(ui, &stats);
            ui.add_space(20.0);
            let sel = self.run_sel.filter(|&i| i < self.runs.len());
            toolbar(
                ui,
                acts,
                |ui, acts| {
                    if ui::button(ui, "New run", Kind::Primary, true).clicked() {
                        acts.push(Act::NewRun);
                    }
                    if ui::button(ui, "Edit", Kind::Secondary, sel.is_some()).clicked() {
                        acts.push(Act::EditRun(sel.unwrap()));
                    }
                    if ui::button(ui, "Delete", Kind::Secondary, sel.is_some()).clicked() {
                        acts.push(Act::DeleteRun(sel.unwrap()));
                    }
                },
                |ui, acts| {
                    if ui::button(ui, "Export CSV", Kind::Ghost, !self.runs.is_empty()).clicked() {
                        acts.push(Act::Export);
                    }
                },
            );
            ui.add_space(16.0);
            let rect = ui.available_rect_before_wrap();
            let runs = &self.runs;
            let cols = [col("Date", 1.3), col("Opponent", 1.5), col("Result", 1.0), col("Squares", 0.9), col("Time", 0.8), col("Board", 1.7), col("Seed", 1.2), col("Notes", 2.4)];
            let out = ui::table(ui, "runs", rect, &cols, runs.len(), sel, "Press New run after a race", |r, ci| {
                let run = &runs[r];
                match ci {
                    0 => (run.date_local().map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default(), c::MUTED),
                    1 => (run.opponent.clone(), c::TEXT),
                    2 => (run.result.clone(), match run.result.as_str() { "Win" => c::GOOD, "Loss" => c::BAD, _ => c::MUTED }),
                    3 => (run.score.clone(), c::TEXT),
                    4 => (run.time.clone(), c::TEXT),
                    5 => (run.board.clone(), c::MUTED),
                    6 => (if run.code.is_empty() { run.seed.clone() } else { format!("{}  ({})", run.seed, run.code) }, c::MUTED),
                    _ => (run.notes.clone(), c::TEXT),
                }
            });
            if let Some(i) = out.clicked {
                self.run_sel = Some(i);
            }
            if let Some(i) = out.double {
                acts.push(Act::EditRun(i));
            }
        });
    }

    // ------------------------------------------------------------ setup

    fn setup(&mut self, ui: &mut Ui, snap: &Snapshot, acts: &mut Vec<Act>) {
        let mut s = self.settings();
        let before = s.clone();
        egui::ScrollArea::vertical().id_salt("setup").auto_shrink([false, false]).show(ui, |ui| {
            page_area(ui, Some(820.0), |ui| {
                ui::title(ui, "Setup");
                match &self.checks {
                    None => ui::sub(ui, "Checking…"),
                    Some(checks) => {
                        let dismissed_keys = s.dismissed.clone();
                        let is_dismissed = |c: &Check| dismissed_keys.contains(&dismiss_key(c));
                        let mut issues: Vec<&Check> = checks.iter().filter(|c| c.is_issue() && !is_dismissed(c)).collect();
                        issues.sort_by_key(|c| c.level);
                        let hidden: Vec<&Check> = checks.iter().filter(|c| c.is_issue() && is_dismissed(c)).collect();
                        let ok: Vec<&Check> = checks.iter().filter(|c| !c.is_issue()).collect();
                        if !issues.is_empty() {
                            ui::section(ui, "Needs attention", 0.0);
                            ui::card(ui, |ui| {
                                for (i, ch) in issues.iter().enumerate() {
                                    let col = match ch.level { Level::Bad => c::BAD, Level::Warn => c::WARN, _ => c::MUTED };
                                    ui::row(ui, Lead::Dot(col), &ch.title, &ch.detail, c::MUTED, false, i > 0, |ui| {
                                        if ch.level != Level::Bad && ui::button(ui, "Dismiss", Kind::Ghost, true).clicked() {
                                            acts.push(Act::Dismiss(dismiss_key(ch)));
                                        }
                                        if let Some((label, fix)) = &ch.fix {
                                            if ui::button(ui, label, Kind::Secondary, true).clicked() {
                                                acts.push(Act::Fix(fix.clone()));
                                            }
                                        }
                                    });
                                }
                            });
                        }
                        if !hidden.is_empty() {
                            ui.add_space(8.0);
                            let label = if self.show_dismissed { "Hide dismissed".to_string() } else { format!("Show {} dismissed", hidden.len()) };
                            if ui::button(ui, &label, Kind::Ghost, true).clicked() {
                                self.show_dismissed = !self.show_dismissed;
                            }
                            if self.show_dismissed {
                                ui.add_space(8.0);
                                ui::card(ui, |ui| {
                                    for (i, ch) in hidden.iter().enumerate() {
                                        ui::row(ui, Lead::Dot(c::FAINT), &ch.title, &ch.detail, c::MUTED, false, i > 0, |ui| {
                                            if ui::button(ui, "Restore", Kind::Ghost, true).clicked() {
                                                acts.push(Act::Undismiss(dismiss_key(ch)));
                                            }
                                        });
                                    }
                                });
                            }
                        }
                        ui::section(ui, "Checks", if issues.is_empty() { 0.0 } else { 30.0 });
                        ui::card(ui, |ui| {
                            for (i, ch) in ok.iter().enumerate() {
                                let lead = if ch.level == Level::Ok { Lead::Check } else { Lead::Dot(c::FAINT) };
                                ui::row(ui, lead, &ch.title, &ch.detail, c::MUTED, true, i > 0, |ui| {
                                    if let Some((label, fix)) = &ch.fix {
                                        if ui::button(ui, label, Kind::Ghost, true).clicked() {
                                            acts.push(Act::Fix(fix.clone()));
                                        }
                                    }
                                });
                            }
                        });
                    }
                }
                ui::section(ui, "Settings", 30.0);
                ui::card(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.vertical(|ui| {
                            ui::toggle(ui, &mut s.auto_backup_on_launch, "Back up save when the game starts");
                            ui::toggle(ui, &mut s.auto_backup_on_randomize, "Back up save after randomizing");
                            ui::toggle(ui, &mut s.debug_tool_as_admin, "Run Debug Tool as administrator");
                        });
                    });
                    ui.add_space(8.0);
                });

                ui::section(ui, "Bingo save", 30.0);
                let save_status = if snap.bingo_save {
                    "In use by the bingo game. Your normal save comes back when it closes.".to_string()
                } else if !s.separate_save {
                    "Off: bingo runs share your normal save file".to_string()
                } else if crate::bingosave::has_bingo_save() {
                    "Ready. Launch from the hub to use it.".to_string()
                } else {
                    "Your first bingo launch starts a fresh save".to_string()
                };
                ui::card(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui::toggle(ui, &mut s.separate_save, "Use a separate save for bingo (launch from the hub)");
                    });
                    ui::row(ui, Lead::None, "Status", &save_status, c::MUTED, true, true, |ui| {
                        if s.separate_save && crate::bingosave::has_bingo_save() && !snap.bingo_save && ui::button(ui, "Start fresh", Kind::Ghost, true).clicked() {
                            acts.push(Act::ResetBingoSave);
                        }
                    });
                });

                ui::section(ui, "Race graces", 30.0);
                use crate::graces::Status;
                let status = match &snap.graces {
                    _ if !s.auto_graces => format!("Automatic setting is off. Use Set graces or {} in game.", sys::HOTKEY),
                    Status::Idle if s.separate_save => "Set automatically on fresh characters in your bingo save".to_string(),
                    Status::Idle => "Set automatically on new characters, from the name screen on".to_string(),
                    Status::Waiting => "Waiting for a character".to_string(),
                    Status::Set => "Set for this character".to_string(),
                    Status::Skipped(n) => format!("Left alone: this character already has {n} graces. Set graces asks before changing it."),
                    Status::Error(e) => e.clone(),
                };
                let hotkey = match snap.hotkey {
                    sys::hotkey::TAKEN => format!("{} is taken by another program. Use the Set graces button.", sys::HOTKEY),
                    _ => format!("{} sets them on the current character while a bingo game runs", sys::HOTKEY),
                };
                ui::card(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui::toggle(ui, &mut s.auto_graces, "Set race graces on new bingo characters");
                    });
                    ui::row(ui, Lead::None, "Status", &status, c::MUTED, true, true, |ui| {
                        if ui::button(ui, "Set graces", Kind::Ghost, snap.game == GameState::Modded).clicked() {
                            acts.push(Act::ApplyGraces);
                        }
                    });
                    ui::row(ui, Lead::None, "Hotkey", &hotkey, if snap.hotkey == sys::hotkey::TAKEN { c::WARN } else { c::MUTED }, true, true, |_| {});
                });

                ui::section(ui, "About", 30.0);
                let (detail, color) = match &self.upd {
                    _ if !update::enabled() => ("Updates aren't set up in this build".to_string(), c::MUTED),
                    Upd::Idle => (String::new(), c::MUTED),
                    Upd::Checking => ("Checking for updates…".into(), c::MUTED),
                    Upd::Current => ("Up to date".into(), c::MUTED),
                    Upd::Available(r) => (format!("Update available: {}", r.summary()), c::ACCENT),
                    Upd::Installing => ("Updating…".into(), c::MUTED),
                    Upd::Failed(e) => (e.clone(), c::WARN),
                };
                let upd = self.upd.clone();
                ui::card(ui, |ui| {
                    ui::row(ui, Lead::None, &format!("Bingo Hub {}", update::VERSION), &detail, color, true, false, |ui| {
                        if !update::enabled() {
                            return;
                        }
                        match upd {
                            Upd::Available(_) => {
                                if ui::button(ui, "Update", Kind::Primary, true).clicked() {
                                    acts.push(Act::ShowUpdate);
                                }
                            }
                            Upd::Installing => {}
                            _ => {
                                if ui::button(ui, "Check for updates", Kind::Ghost, upd != Upd::Checking).clicked() {
                                    acts.push(Act::CheckUpdate);
                                }
                            }
                        }
                    });
                });
                ui.add_space(8.0);
            });
        });
        if s != before {
            self.set(|x| *x = s);
        }
    }

    // ------------------------------------------------------------ overlays

    fn toast_ui(&mut self, ui: &mut Ui, area: Rect) {
        let Some((text, start)) = &self.toast else { return };
        let now = ui.ctx().input(|i| i.time);
        let age = now - start;
        const LIFE: f64 = 4.0;
        if age > LIFE {
            self.toast = None;
            return;
        }
        let a = ((age / 0.15).min(1.0) * ((LIFE - age) / 0.3).min(1.0)) as f32;
        let g = ui::galley(ui, text, ui::small(), c::TEXT);
        let size = vec2(g.size().x + 28.0, 34.0);
        let r = Rect::from_min_size(pos2(area.left() + ui::PAD_X, area.bottom() - 24.0 - size.y + (1.0 - a) * 6.0), size);
        let p = ui.painter();
        p.rect_filled(r, 8.0, c::RAISED.gamma_multiply(a));
        p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, c::BORDER.gamma_multiply(a)), StrokeKind::Inside);
        p.galley_with_override_text_color(pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g, c::TEXT.gamma_multiply(a));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(if a < 1.0 { 16 } else { 250 }));
    }

    fn modal_ui(&mut self, ctx: &egui::Context, snap: &Snapshot, acts: &mut Vec<Act>) {
        let Some(modal) = &mut self.modal else { return };
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("scrim")).order(Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(150));
        });

        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let mut close = esc;
        let mut ok = false;
        let width = if matches!(modal, Modal::Run { .. }) { 520.0 } else { 420.0 };

        egui::Area::new(Id::new("modal")).order(Order::Foreground).anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0)).show(ctx, |ui| {
            Frame::NONE.fill(c::SIDEBAR).stroke(Stroke::new(1.0_f32, c::BORDER)).corner_radius(10).inner_margin(24).show(ui, |ui| {
                ui.set_width(width);
                let heading = match modal {
                    Modal::Confirm { title, .. } | Modal::Alert { title, .. } | Modal::Prompt { title, .. } => title.clone(),
                    Modal::Run { orig, .. } => if orig.is_some() { "Edit run".into() } else { "New run".into() },
                    Modal::Update(_) => "Update available".into(),
                };
                ui.label(egui::RichText::new(heading).font(ui::strong()).color(c::TEXT));
                ui.add_space(12.0);
                let primary = match modal {
                    Modal::Confirm { body, yes, .. } => {
                        ui::paragraph(ui, body, c::MUTED);
                        yes.clone()
                    }
                    Modal::Alert { body, .. } => {
                        ui::paragraph(ui, body, c::MUTED);
                        "OK".into()
                    }
                    Modal::Prompt { label, value, .. } => {
                        ui.label(egui::RichText::new(label.as_str()).font(ui::small()).color(c::MUTED));
                        ui.add_space(8.0);
                        let r = ui::field(ui, "prompt", value, "", width, 60);
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        "Save".into()
                    }
                    Modal::Run { run, date, .. } => {
                        run_form(ui, run, date, width);
                        "Save".into()
                    }
                    Modal::Update(r) => {
                        let restart = if r.hub.is_some() { " The hub restarts by itself when it's done." } else { "" };
                        ui::paragraph(ui, &format!("This updates {}.{restart} Your settings, saves and backups stay as they are.", r.summary()), c::MUTED);
                        if !r.notes.is_empty() {
                            ui.add_space(14.0);
                            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                                ui::paragraph(ui, &r.notes, c::TEXT);
                            });
                        }
                        "Update now".into()
                    }
                };
                ui.add_space(24.0);
                ui.allocate_ui_with_layout(vec2(width, 32.0), Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ui::button(ui, &primary, Kind::Primary, true).clicked() {
                        ok = true;
                    }
                    let cancel = if matches!(modal, Modal::Update(_)) { "Later" } else { "Cancel" };
                    if !matches!(modal, Modal::Alert { .. }) && ui::button(ui, cancel, Kind::Secondary, true).clicked() {
                        close = true;
                    }
                });
            });
        });

        if enter && !ok {
            ok = true;
        }
        if ok {
            let m = self.modal.take().unwrap();
            match m {
                Modal::Alert { .. } => {}
                Modal::Update(_) => acts.push(Act::InstallUpdate),
                Modal::Confirm { then, .. } => self.confirmed(ctx, then, snap),
                Modal::Prompt { value, then, .. } => {
                    let name = value.trim().to_string();
                    if !name.is_empty() {
                        let keep = self.settings().keep_auto_backups;
                        match then {
                            PromptFor::Backup => self.bg(ctx, move |tx| {
                                let _ = tx.send(match core::create_backup(&name, false, keep) {
                                    Ok(_) => Msg::Toast("Save backed up".into()),
                                    Err(e) => Msg::Error(e),
                                });
                            }),
                            PromptFor::Rename(b) => {
                                if let Err(e) = core::rename_backup(&b, &name) {
                                    self.alert("Couldn't rename", e);
                                }
                            }
                        }
                    }
                }
                Modal::Run { orig, mut run, date } => {
                    if let Ok(n) = NaiveDateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M") {
                        if let Some(d) = Local.from_local_datetime(&n).single() {
                            run.date = d.to_rfc3339();
                        }
                    }
                    match orig.and_then(|o| self.runs.iter().position(|r| *r == o)) {
                        Some(i) => self.runs[i] = run,
                        None => self.runs.insert(0, run),
                    }
                    self.runs.sort_by_key(|r| std::cmp::Reverse(r.date_local()));
                    core::save_runs(&self.runs);
                }
            }
        } else if close {
            self.modal = None;
        }
    }
}

fn sel_goals<'a>(b: Option<&'a Board>, q: &str) -> Vec<(usize, &'a str)> {
    b.map(|b| b.goals.iter().enumerate().filter(|(_, g)| q.is_empty() || g.name.to_lowercase().contains(q)).map(|(i, g)| (i, g.name.as_str())).collect()).unwrap_or_default()
}

fn nav_item(ui: &mut Ui, rect: Rect, label: &str, selected: bool, badge: Option<(String, Color32)>, id: (&str, &str)) -> egui::Response {
    let resp = ui.interact(rect, Id::new(id), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    let s = ui.ctx().animate_bool_with_time(resp.id.with("s"), selected, 0.12);
    let fill = Color32::TRANSPARENT.lerp_to_gamma(c::SURFACE, t).lerp_to_gamma(c::RAISED, s);
    if fill.a() > 0 {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let color = c::MUTED.lerp_to_gamma(c::TEXT, t.max(s));
    ui::text_at(ui, pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, label, ui::body(), color);
    if let Some((b, col)) = badge {
        ui::text_at(ui, pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, &b, ui::strong(), col);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn run_form(ui: &mut Ui, run: &mut Run, date: &mut String, width: f32) {
    let half = (width - 12.0) / 2.0;
    let label = |ui: &mut Ui, t: &str| {
        ui.label(egui::RichText::new(t).font(ui::small()).color(c::MUTED));
        ui.add_space(6.0);
    };
    let pair = |ui: &mut Ui, a: (&str, &mut String, &str, &str), b: (&str, &mut String, &str, &str)| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (name, val, id, hint) in [a, b] {
                ui.vertical(|ui| {
                    ui.set_width(half);
                    label(ui, name);
                    ui::field(ui, id, val, hint, half, 120);
                });
            }
        });
        ui.add_space(14.0);
    };
    pair(ui, ("Date", date, "run_date", "YYYY-MM-DD HH:MM"), ("Opponent", &mut run.opponent, "run_opp", ""));

    label(ui, "Result");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for r in ["Win", "Loss", "Draw", "DNF"] {
            let on = run.result == r;
            if ui::button(ui, r, if on { Kind::Primary } else { Kind::Secondary }, true).clicked() {
                run.result = if on { String::new() } else { r.to_string() };
            }
        }
    });
    ui.add_space(14.0);

    pair(ui, ("Squares", &mut run.score, "run_score", "e.g. 13-12"), ("Time", &mut run.time, "run_time", "e.g. 1:42:10"));
    pair(ui, ("Board", &mut run.board, "run_board", ""), ("Seed", &mut run.seed, "run_seed", ""));
    label(ui, "Notes");
    ui::field(ui, "run_notes", &mut run.notes, "", width, 400);
}

const SOMBER: Color32 = Color32::from_rgb(178, 156, 236);
const SMITHING: Color32 = Color32::from_rgb(196, 200, 210);

/// One starting class laid out like a character sheet: name, starting gear, then attributes.
fn class_card(ui: &mut Ui, class: &crate::classes::Class, w: f32, slots: usize) {
    const PAD: f32 = 18.0;
    const TOP: f32 = 18.0 + 24.0 + 10.0;
    const ITEM_H: f32 = 60.0;
    const ICON: f32 = 48.0;
    const STAT_H: f32 = 44.0;
    let stat_rows = class.stats.len().div_ceil(4).max(1);
    let h = TOP + slots as f32 * ITEM_H + 12.0 + 14.0 + stat_rows as f32 * STAT_H + 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter().clone();
    p.rect_filled(rect, 10.0, c::SURFACE);
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, c::BORDER), StrokeKind::Inside);

    let left = rect.left() + PAD;
    let right = rect.right() - PAD;
    let name_font = egui::FontId::new(18.0, egui::FontFamily::Name("sb".into()));
    p.text(pos2(left, rect.top() + 30.0), Align2::LEFT_CENTER, &class.name, name_font, c::TEXT);

    const NUM_W: f32 = 54.0;
    let tag_w = ui::galley(ui, "Smithing", tag_font(), c::TEXT).size().x + 20.0;
    let stats = class_stats(class);
    // Which spell tools the class starts with.
    let tools = class.items.iter().filter_map(|it| crate::weapons::weapon(&it.name)).fold((false, false), |t, w| (t.0 || w.sorcery, t.1 || w.incantation));

    for (n, it) in class.items.iter().enumerate() {
        let info = crate::items::info(&it.name);
        let row = Rect::from_min_size(pos2(left, rect.top() + TOP + n as f32 * ITEM_H), vec2(right - left, ITEM_H));
        let icon = Rect::from_min_size(pos2(left, row.center().y - ICON / 2.0), vec2(ICON, ICON));
        let pick = crate::items::bingo_pick(&it.name);
        p.rect_filled(icon, 8.0, if pick { c::RAISED.lerp_to_gamma(c::ACCENT, 0.12) } else { c::RAISED });
        if pick {
            p.rect_stroke(icon, 8.0, Stroke::new(1.0_f32, c::ACCENT.gamma_multiply(0.55)), StrokeKind::Inside);
        }
        if let Some(tex) = info.and_then(|i| crate::items::texture(ui.ctx(), i.icon)) {
            p.image(tex.id(), icon.shrink(2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        // Upgrade tag on the right; spells and ammo have none.
        let tag = info.and_then(|i| match i.upgrade {
            crate::items::Upgrade::Somber => Some(("Somber", SOMBER)),
            crate::items::Upgrade::Smithing => Some(("Smithing", SMITHING)),
            crate::items::Upgrade::None => None,
        });
        let mut text_right = right;
        if let Some((label, col)) = tag {
            let g = ui::galley(ui, label, tag_font(), col);
            let pill = Rect::from_min_size(pos2(right - g.size().x - 20.0, row.center().y - 11.0), vec2(g.size().x + 20.0, 22.0));
            p.rect_filled(pill, 11.0, col.gamma_multiply(0.13));
            p.galley(pill.center() - g.size() / 2.0, g, col);
            text_right = pill.left() - 12.0;
        }

        // Attack rating (or spell FP) in a fixed column left of the widest tag, so rows line up.
        let d = ItemDetail::new(it, &stats, &tools);
        if let Some((value, label, col)) = d.figure() {
            text_right = right - tag_w - 12.0;
            let cx = text_right - NUM_W / 2.0;
            p.text(pos2(cx, row.center().y - 7.0), Align2::CENTER_CENTER, value, egui::FontId::new(15.0, egui::FontFamily::Name("sb".into())), col);
            p.text(pos2(cx, row.center().y + 10.0), Align2::CENTER_CENTER, label, egui::FontId::new(10.5, egui::FontFamily::Proportional), c::FAINT);
            text_right -= NUM_W + 10.0;
        }

        let x = icon.right() + 14.0;
        // "Bingo pick" badge right after the name.
        let badge = pick.then(|| ui::galley(ui, "BINGO PICK", egui::FontId::new(10.0, egui::FontFamily::Name("sb".into())), c::ACCENT));
        let badge_w = badge.as_ref().map_or(0.0, |g| g.size().x + 14.0 + 8.0);
        let name = ui::truncated(ui, &it.name, ui::body(), c::TEXT, text_right - x - badge_w);
        let name_w = name.size().x;
        let name_y = row.center().y - 19.0;
        p.galley(pos2(x, name_y), name, c::TEXT);
        if let Some(g) = badge {
            let b = Rect::from_min_size(pos2(x + name_w + 8.0, name_y + 1.0), vec2(g.size().x + 14.0, 17.0));
            p.rect_filled(b, 4.0, c::ACCENT.gamma_multiply(0.16));
            p.galley(b.center() - g.size() / 2.0, g, c::ACCENT);
        }

        // Shown in this order, but when space is short the least important go first: warnings stay,
        // then the skill, the weapon type, the buildup. Items are never cut in half.
        let mut parts: Vec<(u8, String, Color32)> = vec![];
        if let Some(i) = info {
            parts.push((2, i.kind.to_string(), c::MUTED));
        }
        if let Some(n) = it.count {
            parts.push((0, format!("×{n}"), c::MUTED));
        }
        if let Some(l) = it.level {
            parts.push((0, format!("Lv {l} to wield"), c::WARN));
        }
        if let Some(why) = d.cant_cast() {
            parts.push((0, why, c::WARN));
        }
        if let Some(w) = d.weapon.filter(|w| !w.skill.is_empty()) {
            parts.push((1, w.skill.to_string(), c::MUTED));
        }
        if let Some((name, v)) = d.buildup().first() {
            parts.push((3, format!("{name} {v}"), c::MUTED));
        }
        let max_w = (text_right - x).max(1.0);
        let layout = |keep: &[bool]| {
            let mut job = egui::text::LayoutJob::default();
            for ((_, text, col), _) in parts.iter().zip(keep).filter(|(_, k)| **k) {
                let sep = if job.text.is_empty() { "" } else { "  ·  " };
                job.append(&format!("{sep}{text}"), 0.0, egui::TextFormat::simple(ui::small(), *col));
            }
            job
        };
        let mut keep = vec![false; parts.len()];
        for level in 0..=3 {
            for i in 0..parts.len() {
                if parts[i].0 != level {
                    continue;
                }
                keep[i] = true;
                let fits = ui.fonts_mut(|f| f.layout_job(layout(&keep))).size().x <= max_w;
                if !fits && keep.iter().filter(|k| **k).count() > 1 {
                    keep[i] = false;
                }
            }
        }
        let mut job = layout(&keep);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let g = ui.fonts_mut(|f| f.layout_job(job));
        p.galley(pos2(x, row.center().y + 3.0), g, c::MUTED);

        // The full breakdown on hover, after a short pause so passing over the cards stays quiet.
        let resp = ui.interact(row, ui.id().with(("class_item", &class.name, n)), Sense::hover());
        if d.has_more() && hover_after(ui, &resp, 0.3) {
            item_detail(ui, &d, &it.name, pos2(x - 8.0, row.bottom() + 2.0));
        }
    }
    let y = rect.top() + TOP + slots as f32 * ITEM_H + 12.0;
    p.hline(left..=right, y, Stroke::new(1.0_f32, c::DIVIDER));

    let short = |k: &str| match k {
        "Vigor" => "VIG",
        "Mind" => "MND",
        "Endurance" => "END",
        "Strength" => "STR",
        "Dexterity" => "DEX",
        "Intelligence" => "INT",
        "Faith" => "FAI",
        "Arcane" => "ARC",
        _ => "",
    };
    // One panel, value over label, hairlines between cells.
    let panel = Rect::from_min_size(pos2(left, y + 14.0), vec2(right - left, stat_rows as f32 * STAT_H));
    p.rect_filled(panel, 8.0, c::SURFACE.lerp_to_gamma(c::BG, 0.6));
    let cell_w = panel.width() / 4.0;
    let line = Stroke::new(1.0_f32, c::DIVIDER);
    for col in 1..4 {
        let x = (panel.left() + col as f32 * cell_w).round();
        p.vline(x, panel.y_range().shrink(9.0), line);
    }
    for row in 1..stat_rows {
        p.hline(panel.x_range().shrink(12.0), (panel.top() + row as f32 * STAT_H).round(), line);
    }
    let value_font = egui::FontId::new(15.0, egui::FontFamily::Name("sb".into()));
    let label_font = egui::FontId::new(10.5, egui::FontFamily::Proportional);
    for (i, (k, v)) in class.stats.iter().enumerate() {
        let cx = panel.left() + (i % 4) as f32 * cell_w + cell_w / 2.0;
        let cy = panel.top() + (i / 4) as f32 * STAT_H + STAT_H / 2.0;
        p.text(pos2(cx, cy - 7.0), Align2::CENTER_CENTER, v.to_string(), value_font.clone(), c::TEXT);
        p.text(pos2(cx, cy + 10.0), Align2::CENTER_CENTER, short(k), label_font.clone(), c::FAINT);
    }
}

fn tag_font() -> egui::FontId {
    egui::FontId::new(12.0, egui::FontFamily::Name("sb".into()))
}

/// Str, Dex, Int, Fai, Arc from the spoiler's stat list.
fn class_stats(class: &crate::classes::Class) -> [u32; 5] {
    let get = |k: &str| class.stats.iter().find(|(n, _)| n == k).map_or(0, |(_, v)| *v);
    [get("Strength"), get("Dexterity"), get("Intelligence"), get("Faith"), get("Arcane")]
}

/// What the Classes page knows about one starting item at its class's stats.
struct ItemDetail {
    kind: &'static str,
    weapon: Option<&'static crate::weapons::Weapon>,
    spell: Option<&'static crate::weapons::Spell>,
    attack: Option<crate::weapons::Attack>,
    bow: bool,
    /// The stats the attack was worked out with (Strength ×1.5 when shown two-handed).
    eff: [u32; 5],
    /// Starts with a staff, a seal.
    tools: (bool, bool),
}

impl ItemDetail {
    fn new(it: &crate::classes::Item, stats: &[u32; 5], tools: &(bool, bool)) -> Self {
        let kind = crate::items::info(&it.name).map_or("", |i| i.kind);
        let weapon = crate::weapons::weapon(&it.name).filter(|_| !matches!(kind, "Arrow" | "Greatarrow" | "Bolt" | "Ballista Bolt"));
        let bow = matches!(kind, "Light Bow" | "Bow" | "Greatbow" | "Ballista");
        let attack = weapon.map(|w| crate::weapons::attack(w, *stats, bow));
        let mut eff = *stats;
        if attack.as_ref().is_some_and(|a| a.two_handed) {
            eff[0] = stats[0] * 3 / 2;
        }
        let spell = matches!(kind, "Sorcery" | "Incantation").then(|| crate::weapons::spell(&it.name)).flatten();
        ItemDetail { kind, weapon, spell, attack, bow, eff, tools: *tools }
    }

    fn short(&self, a: usize) -> bool {
        self.weapon.is_some_and(|w| self.eff[a] < w.req[a])
    }

    /// The number on the right of the row: attack rating, a catalyst's spell scaling, or a spell's FP.
    fn figure(&self) -> Option<(String, &'static str, Color32)> {
        if let Some(a) = &self.attack {
            let col = if (0..5).any(|i| self.short(i)) { c::WARN } else { c::TEXT };
            if let Some(s) = a.spell_scaling {
                return Some((s.to_string(), "SCALING", col));
            }
            if a.total > 0 {
                return Some((a.total.to_string(), if a.two_handed && !self.bow { "2H ATTACK" } else { "ATTACK" }, col));
            }
        }
        let s = self.spell?;
        Some((s.fp.to_string(), "FP", if self.cant_cast().is_some() { c::WARN } else { c::TEXT }))
    }

    /// Why the class can't cast this spell from the start, if it can't.
    fn cant_cast(&self) -> Option<String> {
        let s = self.spell?;
        match self.kind {
            "Sorcery" if !self.tools.0 => return Some("No staff".into()),
            "Incantation" if !self.tools.1 => return Some("No seal".into()),
            _ => {}
        }
        let stat = [(2, "Int"), (3, "Fai"), (4, "Arc")];
        let (i, (_, k)) = stat.iter().enumerate().find(|(i, (a, _))| self.eff[*a] < s.req[*i])?;
        Some(format!("Needs {k} {}", s.req[i]))
    }

    /// Status buildup, e.g. [("Bleed", 45)].
    fn buildup(&self) -> Vec<(&'static str, u32)> {
        self.attack.as_ref().map_or(vec![], |a| (0..7).filter(|&s| a.status[s] > 0).map(|s| (crate::weapons::STATUS[s], a.status[s])).collect())
    }

    fn has_more(&self) -> bool {
        self.attack.is_some() || self.spell.is_some()
    }
}

/// True once the pointer has rested on `resp` for `secs`.
fn hover_after(ui: &Ui, resp: &egui::Response, secs: f64) -> bool {
    let key = resp.id.with("hover_since");
    let now = ui.input(|i| i.time);
    if !resp.hovered() {
        ui.ctx().data_mut(|d| d.remove::<f64>(key));
        return false;
    }
    let since = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    if now - since >= secs {
        return true;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(secs - (now - since)));
    false
}

/// The hover card for a starting item: everything the row has no room for.
fn item_detail(ui: &Ui, d: &ItemDetail, name: &str, pos: egui::Pos2) {
    const KEY_W: f32 = 92.0;
    let row = |ui: &mut Ui, key: &str, parts: &[(String, Color32)]| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        ui::text_at(ui, pos2(r.left(), r.center().y), Align2::LEFT_CENTER, key, ui::small(), c::MUTED);
        let mut job = egui::text::LayoutJob::default();
        for (i, (t, col)) in parts.iter().enumerate() {
            job.append(t, if i == 0 { 0.0 } else { 12.0 }, egui::TextFormat::simple(ui::small(), *col));
        }
        let g = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(pos2(r.left() + KEY_W, r.center().y - g.size().y / 2.0), g, c::TEXT);
    };
    egui::Area::new(ui.id().with("item_detail"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos)
        .constrain(true)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(c::RAISED)
                .stroke(Stroke::new(1.0_f32, c::BORDER))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(268.0);
                    ui.label(egui::RichText::new(name).font(ui::strong()).color(c::TEXT));
                    ui.add_space(8.0);
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0_f32, c::BORDER));
                    ui.add_space(6.0);

                    if let (Some(w), Some(a)) = (d.weapon, &d.attack) {
                        if !w.skill.is_empty() {
                            let mut parts = vec![(w.skill.to_string(), c::TEXT)];
                            if !w.skill_fp.is_empty() {
                                parts.push((format!("{} FP", w.skill_fp), c::MUTED));
                            }
                            row(ui, "Skill", &parts);
                        }
                        if let Some(s) = a.spell_scaling {
                            row(ui, if w.sorcery { "Sorcery" } else { "Incantations" }, &[(format!("{s} scaling"), c::TEXT)]);
                        } else {
                            for t in (0..5).filter(|&t| a.damage[t] > 0) {
                                row(ui, crate::weapons::DAMAGE[t], &[(a.damage[t].to_string(), c::TEXT)]);
                            }
                        }
                        for (s, v) in d.buildup() {
                            row(ui, s, &[(v.to_string(), c::TEXT)]);
                        }
                        let letters: Vec<(String, Color32)> = w.letters().into_iter().map(|(k, l)| (format!("{k} {l}"), c::TEXT)).collect();
                        if !letters.is_empty() {
                            row(ui, "Scaling", &letters);
                        }
                        let req: Vec<(String, Color32)> = (0..5)
                            .filter(|&i| w.req[i] > 0)
                            .map(|i| match d.short(i) {
                                true => (format!("{} {} (+{})", crate::weapons::ATTRS[i], w.req[i], w.req[i] - d.eff[i]), c::WARN),
                                false => (format!("{} {}", crate::weapons::ATTRS[i], w.req[i]), c::TEXT),
                            })
                            .collect();
                        if !req.is_empty() {
                            row(ui, "Requires", &req);
                        }
                        if !w.damage_type.is_empty() && a.spell_scaling.is_none() {
                            row(ui, "Damage", &[(w.damage_type.to_string(), c::TEXT)]);
                        }
                    } else if let Some(s) = d.spell {
                        row(ui, "FP", &[(s.fp.to_string(), c::TEXT)]);
                        row(ui, "Slots", &[(s.slots.to_string(), c::TEXT)]);
                        let req: Vec<(String, Color32)> = [(2, "Int"), (3, "Fai"), (4, "Arc")]
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| s.req[*i] > 0)
                            .map(|(i, (a, k))| match d.eff[*a] < s.req[i] {
                                true => (format!("{k} {} (+{})", s.req[i], s.req[i] - d.eff[*a]), c::WARN),
                                false => (format!("{k} {}", s.req[i]), c::TEXT),
                            })
                            .collect();
                        if !req.is_empty() {
                            row(ui, "Requires", &req);
                        }
                    }
                });
        });
}

/// The seed's swap lists, closed until someone asks for them.
fn spoiler(ui: &mut Ui, swaps: &[crate::classes::Swaps], st: &mut Spoiler) {
    let total: usize = swaps.iter().map(|g| g.pairs.len()).sum();
    ui::card(ui, |ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.09);
        if hover > 0.0 {
            ui.painter().rect_filled(rect.shrink(3.0), 7.0, Color32::TRANSPARENT.lerp_to_gamma(c::RAISED, hover));
        }
        let x = rect.left() + 18.0;
        ui::text_at(ui, pos2(x, rect.top() + 20.0), Align2::LEFT_CENTER, "Spoiler", ui::strong(), c::TEXT);
        let sub = format!("{total} weapon and spell swaps for this seed");
        ui::text_at(ui, pos2(x, rect.top() + 39.0), Align2::LEFT_CENTER, &sub, ui::small(), c::MUTED);
        let col = c::MUTED.lerp_to_gamma(c::TEXT, hover);
        ui::text_at(ui, pos2(rect.right() - 40.0, rect.center().y), Align2::RIGHT_CENTER, if st.open { "Hide" } else { "Show" }, ui::btn_font(), col);
        chevron(ui, pos2(rect.right() - 24.0, rect.center().y), st.open, col);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            st.open = !st.open;
            if !st.open {
                st.query.clear();
                st.groups.clear();
            }
        }
        if !st.open {
            return;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let w = ui.available_width() - 16.0;
            ui::field(ui, "spoiler_find", &mut st.query, "Find an item", w, 60);
        });
        ui.add_space(10.0);
        let q = st.query.trim().to_lowercase();
        let mut shown = 0;
        for g in swaps {
            let pairs: Vec<&(String, String)> =
                g.pairs.iter().filter(|(a, b)| q.is_empty() || a.to_lowercase().contains(&q) || b.to_lowercase().contains(&q)).collect();
            if pairs.is_empty() {
                continue;
            }
            shown += 1;
            // While searching, every group with a match is open.
            let open = !q.is_empty() || st.groups.contains(&g.group);
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.09);
            ui.painter().hline(row.x_range().shrink(16.0), row.top(), Stroke::new(1.0_f32, c::DIVIDER));
            let col = c::MUTED.lerp_to_gamma(c::TEXT, if open { 1.0 } else { hover });
            ui::text_at(ui, pos2(row.left() + 18.0, row.center().y), Align2::LEFT_CENTER, &g.group, ui::body(), col);
            ui::text_at(ui, pos2(row.right() - 40.0, row.center().y), Align2::RIGHT_CENTER, &pairs.len().to_string(), ui::small(), c::FAINT);
            chevron(ui, pos2(row.right() - 24.0, row.center().y), open, col);
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && q.is_empty() && !st.groups.remove(&g.group) {
                st.groups.insert(g.group.clone());
            }
            if !open {
                continue;
            }
            // Two columns of "vanilla → now".
            const LINE: f32 = 26.0;
            let col_gap = 32.0;
            let cw = (ui.available_width() - 36.0 - col_gap) / 2.0;
            let half = pairs.len().div_ceil(2);
            let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), half as f32 * LINE + 12.0), Sense::hover());
            let aw = (cw - 28.0) / 2.0;
            for (i, (a, b)) in pairs.iter().enumerate() {
                let x = area.left() + 18.0 + if i < half { 0.0 } else { cw + col_gap };
                let cy = area.top() + (i % half) as f32 * LINE + LINE / 2.0;
                let ga = ui::truncated(ui, a, ui::small(), c::MUTED, aw);
                ui.painter().galley(pos2(x, cy - ga.size().y / 2.0), ga, c::MUTED);
                ui::text_at(ui, pos2(x + aw + 14.0, cy), Align2::CENTER_CENTER, "→", ui::small(), c::FAINT);
                let gb = ui::truncated(ui, b, ui::small(), c::TEXT, aw);
                ui.painter().galley(pos2(x + aw + 28.0, cy - gb.size().y / 2.0), gb, c::TEXT);
            }
        }
        if shown == 0 {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui::sub(ui, "Nothing matches.");
            });
            ui.add_space(14.0);
        }
    });
}

fn chevron(ui: &Ui, center: egui::Pos2, open: bool, col: Color32) {
    let s = Stroke::new(1.6_f32, col);
    let pts = if open {
        [center + vec2(-4.5, -2.0), center + vec2(0.0, 2.5), center + vec2(4.5, -2.0)]
    } else {
        [center + vec2(-2.0, -4.5), center + vec2(2.5, 0.0), center + vec2(-2.0, 4.5)]
    };
    ui.painter().line_segment([pts[0], pts[1]], s);
    ui.painter().line_segment([pts[1], pts[2]], s);
}
