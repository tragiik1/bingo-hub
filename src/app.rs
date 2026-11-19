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
    Restore(Backup),
    Delete(Backup),
    Debug,
    StartSteam,
    DeleteRun(usize),
}

enum PromptFor {
    Backup,
    Rename(Backup),
}

enum Modal {
    Confirm { title: String, body: String, yes: String, then: Confirmed },
    Alert { title: String, body: String },
    Prompt { title: String, label: String, value: String, then: PromptFor },
    Run { idx: Option<usize>, run: Run, date: String },
    Update(Release),
}

struct Practice {
    board: usize,
    path: PathBuf,
    n: usize,
    cells: Vec<usize>,
    marked: Vec<bool>,
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
                    sys::focus_pid(pid);
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
                self.modal = Some(Modal::Run { idx: None, date: now.format("%Y-%m-%d %H:%M").to_string(), run });
            }
            Act::EditRun(i) => {
                if let Some(r) = self.runs.get(i) {
                    let date = r.date_local().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                    self.modal = Some(Modal::Run { idx: Some(i), run: r.clone(), date });
                }
            }
            Act::DeleteRun(i) => {
                if let Some(r) = self.runs.get(i) {
                    let d = r.date_local().map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default();
                    self.confirm("Delete run", format!("Delete the run from {d}?"), "Delete", Confirmed::DeleteRun(i));
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
            Confirmed::ResetBingoSave => match crate::bingosave::reset_bingo_save() {
                Ok(()) => self.toast(ctx, "Next bingo game starts with a fresh save"),
                Err(e) => self.alert("Couldn't reset", e),
            },
            Confirmed::DeleteRun(i) => {
                if i < self.runs.len() {
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

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        ui::text_at(ui, pos2(r.left() + 24.0, r.top() + TITLEBAR_H / 2.0 + 4.0), Align2::LEFT_CENTER, TITLE, ui::strong(), c::TEXT);

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
                        GameState::Starting => (Lead::Num("3"), "Starting".into(), c::MUTED),
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
                        if ui::button(ui, if running { "Show" } else { "Launch" }, Kind::Primary, running || reg.is_some()).clicked() {
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
        egui::ScrollArea::vertical().id_salt("classes").auto_shrink([false, false]).show(ui, |ui| {
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
                let gap = 12.0;
                let w = ((ui.available_width() - gap) / 2.0).floor();
                for pair in sp.classes.chunks(2) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for class in pair {
                            ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
                                ui.set_width(w);
                                class_card(ui, class);
                            });
                        }
                    });
                    ui.add_space(gap);
                }
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
                    Modal::Run { idx, .. } => if idx.is_some() { "Edit run".into() } else { "New run".into() },
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
                Modal::Run { idx, mut run, date } => {
                    if let Ok(n) = NaiveDateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M") {
                        if let Some(d) = Local.from_local_datetime(&n).single() {
                            run.date = d.to_rfc3339();
                        }
                    }
                    match idx {
                        Some(i) if i < self.runs.len() => self.runs[i] = run,
                        _ => self.runs.insert(0, run),
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

fn class_card(ui: &mut Ui, class: &crate::classes::Class) {
    ui::card(ui, |ui| {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&class.name).font(ui::strong()).color(c::TEXT));
                ui.add_space(6.0);
                for it in &class.items {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&it.name, 0.0, egui::TextFormat::simple(ui::body(), c::TEXT));
                    if let Some(n) = it.count {
                        job.append(&format!("  ×{n}"), 0.0, egui::TextFormat::simple(ui::small(), c::MUTED));
                    }
                    if let Some(l) = it.level {
                        job.append(&format!("   needs lv {l}"), 0.0, egui::TextFormat::simple(ui::small(), c::WARN));
                    }
                    ui.label(job);
                }
                ui.add_space(8.0);
                let short = |k: &str| match k {
                    "Vigor" => "VIG", "Mind" => "MND", "Endurance" => "END", "Strength" => "STR",
                    "Dexterity" => "DEX", "Intelligence" => "INT", "Faith" => "FAI", "Arcane" => "ARC",
                    _ => "",
                };
                for row in class.stats.chunks(4) {
                    let mut job = egui::text::LayoutJob::default();
                    for (k, v) in row {
                        job.append(short(k), 0.0, egui::TextFormat::simple(ui::small(), c::FAINT));
                        job.append(&format!(" {v:<3}   "), 0.0, egui::TextFormat::simple(ui::small(), c::TEXT));
                    }
                    ui.label(job);
                }
            });
        });
        ui.add_space(10.0);
    });
}
