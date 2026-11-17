//! Files, settings, saves, boards, run log and setup checks. No UI here.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sys;

// ---------------------------------------------------------------- paths

pub struct Paths {
    pub board_dir: PathBuf,
    pub board_exe: PathBuf,
    pub rando_dir: PathBuf,
    pub debug_exe: PathBuf,
    pub me2_dir: PathBuf,
    pub me2_launcher: PathBuf,
    pub me2_config: PathBuf,
    pub regulation: PathBuf,
    pub data_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub boards_dir: PathBuf,
    pub downloads: PathBuf,
}

pub fn paths() -> &'static Paths {
    static P: OnceLock<Paths> = OnceLock::new();
    P.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into());
        let root = exe_dir
            .ancestors()
            .find(|d| d.join("Bingo Board").is_dir() || d.join("Bingo Randomizer").is_dir())
            .map(Path::to_path_buf)
            .unwrap_or(exe_dir);
        let rando_dir = root.join("Bingo Randomizer");
        let me2_dir = rando_dir.join("Resources").join("ME2");
        let env_dir = |k: &str, d: PathBuf| std::env::var_os(k).map(PathBuf::from).unwrap_or(d);
        let p = Paths {
            board_dir: root.join("Bingo Board"),
            board_exe: root.join("Bingo Board").join("EldenBingo.exe"),
            debug_exe: root.join("Debug Tool").join("Elden Ring Debug Tool.exe"),
            me2_launcher: me2_dir.join("modengine2_launcher.exe"),
            me2_config: me2_dir.join("config_bingo.toml"),
            regulation: me2_dir.join("bingo").join("regulation.bin"),
            data_dir: root.join("Data"),
            backups_dir: env_dir("BINGOHUB_BACKUP_DIR", root.join("Save Backups")),
            boards_dir: root.join("Boards"),
            downloads: std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join("Downloads")).unwrap_or_default(),
            me2_dir,
            rando_dir,
        };
        // v1 kept its data in "Bingo Hub\Data".
        let legacy = p.boards_dir.parent().unwrap().join("Bingo Hub").join("Data");
        if !p.data_dir.exists() && legacy.is_dir() {
            let _ = fs::rename(&legacy, &p.data_dir);
        }
        for d in [&p.data_dir, &p.backups_dir, &p.boards_dir] {
            let _ = fs::create_dir_all(d);
        }
        p
    })
}

/// The Randomizer's exe. Its name changes between builds (BingoRandomizer0.91.exe, S2_Style_1.0.exe),
/// so take whatever exe sits in the folder root; the newest wins if there's more than one.
pub fn rando_exe() -> Option<PathBuf> {
    fs::read_dir(&paths().rando_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file() && e.file_name().to_string_lossy().to_lowercase().ends_with(".exe"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// The Randomizer's last run, as recorded in its Cache\LastSeed.json.
#[derive(Clone, PartialEq, Debug)]
pub struct RandoRun {
    pub seed: String,
    /// Short code that matches only when seed, Randomizer build and game patch all match.
    pub code: String,
}

pub fn last_seed_file() -> PathBuf {
    paths().rando_dir.join("Cache").join("LastSeed.json")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn file_hash_hex(p: &Path) -> Option<String> {
    hash(p).ok().map(|h| hex(&h))
}

/// Reads LastSeed.json and derives the race code from the inputs that decide the randomization:
/// the seed it used, the Randomizer build (`exe_hash`) and the game's own regulation.bin.
pub fn rando_run(exe_hash: &str, game_dir: Option<&Path>) -> Option<RandoRun> {
    if !paths().regulation.is_file() {
        return None;
    }
    let text = fs::read_to_string(last_seed_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let seed = v["Seed"].as_str()?.to_string();
    let game_reg = file_hash_hex(&game_dir?.join("regulation.bin")).unwrap_or_default();
    let digest = Sha256::digest(format!("{exe_hash}|{seed}|{game_reg}").as_bytes());
    Some(RandoRun { seed, code: hex(&digest[..3]).to_uppercase() })
}

/// Writes through a temporary file and a rename, so a crash mid-write leaves the old file intact.
pub fn write_atomic(p: &Path, data: impl AsRef<[u8]>) -> io::Result<()> {
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, p)
}

pub fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn to_local(t: SystemTime) -> DateTime<Local> {
    t.into()
}

pub fn find_game_exe(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(o) = override_path.filter(|o| Path::new(o).is_file()) {
        return Some(PathBuf::from(o));
    }
    let mut libs = Vec::new();
    if let Some(steam) = sys::steam_path() {
        if let Ok(vdf) = fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
            for line in vdf.lines() {
                let t = line.trim();
                if t.starts_with("\"path\"") {
                    if let Some(v) = t.split('"').nth(3) {
                        libs.push(PathBuf::from(v.replace("\\\\", "\\")));
                    }
                }
            }
        }
        libs.insert(0, steam);
    }
    libs.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    for lib in libs {
        let exe = lib.join("steamapps").join("common").join("ELDEN RING").join("Game").join("eldenring.exe");
        if exe.is_file() {
            return Some(exe);
        }
    }
    let bp = board_config_get("GamePath")?;
    let exe = Path::new(&bp).join("eldenring.exe");
    exe.is_file().then_some(exe)
}

/// The Steam-ID save folder with the most recently written ER0000.sl2.
pub fn save_dir() -> Option<PathBuf> {
    if let Some(t) = std::env::var_os("BINGOHUB_SAVE_DIR") {
        return Some(PathBuf::from(t));
    }
    let root = PathBuf::from(std::env::var_os("APPDATA")?).join("EldenRing");
    fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.path().join("ER0000.sl2").is_file())
        .max_by_key(|e| mtime(&e.path().join("ER0000.sl2")))
        .map(|e| e.path())
}

pub fn live_save() -> Option<PathBuf> {
    save_dir().map(|d| d.join("ER0000.sl2"))
}

// ---------------------------------------------------------------- settings (same file and keys as v1)

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Settings {
    pub seed: String,
    pub seed_set_utc: String,
    pub auto_backup_on_launch: bool,
    pub auto_backup_on_randomize: bool,
    pub keep_auto_backups: u32,
    pub debug_tool_as_admin: bool,
    /// Set the race graces automatically once a bingo character has loaded.
    pub auto_graces: bool,
    /// Bingo launches use their own save file instead of the normal one.
    pub separate_save: bool,
    /// Setup warnings the user dismissed ("title|detail"); they come back if the detail changes.
    pub dismissed: Vec<String>,
    pub debug_warning_accepted: bool,
    pub game_exe_override: Option<String>,
    pub last_page: String,
    /// Game version at the Randomizer's last successful run, and when that run was (unix secs).
    pub rando_game_version: String,
    pub rando_run_time: u64,
    /// SHA-256 of the normal save as the last bingo game on it left it ("" = none since).
    pub bingo_played_hash: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seed: String::new(),
            seed_set_utc: String::new(),
            auto_backup_on_launch: true,
            auto_backup_on_randomize: true,
            keep_auto_backups: 10,
            debug_tool_as_admin: false,
            auto_graces: true,
            separate_save: false,
            dismissed: Vec::new(),
            debug_warning_accepted: false,
            game_exe_override: None,
            last_page: "Race".into(),
            rando_game_version: String::new(),
            rando_run_time: 0,
            bingo_played_hash: String::new(),
        }
    }
}

impl Settings {
    fn file() -> PathBuf {
        paths().data_dir.join("hub.json")
    }
    pub fn load() -> Self {
        fs::read_to_string(Self::file()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = write_atomic(&Self::file(), s);
        }
    }
    pub fn rando_game_version_time(&self) -> u64 {
        self.rando_run_time
    }

}

/// Parses ISO timestamps written by either version. Naive times are UTC if `naive_utc`, else local.
pub fn parse_time(s: &str, naive_utc: bool) -> Option<DateTime<Local>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Local));
    }
    let n = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    if n.and_utc().timestamp() <= -62_000_000_000 {
        return None; // 0001-01-01
    }
    if naive_utc { Some(Utc.from_utc_datetime(&n).with_timezone(&Local)) } else { Local.from_local_datetime(&n).single() }
}

// ---------------------------------------------------------------- Bingo Board's user.config

/// The Board keeps one settings folder per version it has run as. Use the one for the installed
/// version (or the closest older one), so swapping the Board to an older release still works.
fn board_user_config() -> Option<PathBuf> {
    let parse = |s: &str| -> Vec<u32> {
        let mut v: Vec<u32> = s.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        v.resize(4, 0);
        v
    };
    let installed = sys::file_version(&paths().board_exe).map(|s| parse(&s));
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("EldenBingo");
    let mut all: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for app in fs::read_dir(root).ok()?.flatten() {
        if !app.file_name().to_string_lossy().starts_with("EldenBingo") {
            continue;
        }
        for v in fs::read_dir(app.path()).into_iter().flatten().flatten() {
            let cfg = v.path().join("user.config");
            if cfg.is_file() {
                all.push((parse(&v.file_name().to_string_lossy()), cfg));
            }
        }
    }
    all.sort_by(|a, b| b.0.cmp(&a.0));
    let fitting = installed.and_then(|i| all.iter().position(|(v, _)| *v <= i));
    let idx = fitting.unwrap_or(0);
    (idx < all.len()).then(|| all.swap_remove(idx).1)
}

/// Byte range of a setting's value inside the XML, plus whether it's a self-closing <value />.
fn find_setting(xml: &str, name: &str) -> Option<(usize, usize, bool)> {
    let at = xml.find(&format!("<setting name=\"{name}\""))?;
    let rest = &xml[at..];
    let v = rest.find("<value")?;
    let after = at + v + "<value".len();
    let close = xml[after..].find('>')? + after;
    if xml[..close].ends_with('/') {
        return Some((at + v, close + 1, true));
    }
    let end = xml[close..].find("</value>")? + close;
    Some((close + 1, end, false))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn board_config_get(name: &str) -> Option<String> {
    let files = [board_user_config(), Some(paths().board_dir.join("EldenBingo.dll.config"))];
    for f in files.into_iter().flatten() {
        let Ok(xml) = fs::read_to_string(&f) else { continue };
        if let Some((a, b, empty)) = find_setting(&xml, name) {
            return Some(if empty { String::new() } else { xml_unescape(&xml[a..b]) });
        }
    }
    None
}

pub fn board_config_set(name: &str, value: &str, board_running: bool) -> Result<(), String> {
    if board_running {
        return Err("Close the Bingo Board first. It overwrites its settings when it exits.".into());
    }
    // Before the Board's first run there's no user.config yet; its defaults file works the same.
    let f = board_user_config().unwrap_or_else(|| paths().board_dir.join("EldenBingo.dll.config"));
    let xml = fs::read_to_string(&f).map_err(|e| e.to_string())?;
    let (a, b, empty) = find_setting(&xml, name).ok_or(format!("The Bingo Board has no \"{name}\" setting."))?;
    let v = xml_escape(value);
    let new = if empty { format!("{}<value>{}</value>{}", &xml[..a], v, &xml[b..]) } else { format!("{}{}{}", &xml[..a], v, &xml[b..]) };
    fs::write(&f, new).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- saves

const STAMP: &str = "%Y-%m-%d %H-%M-%S";
const AUTO: &str = "[auto]";

#[derive(Clone, PartialEq, Debug)]
pub struct Backup {
    pub path: PathBuf,
    pub label: String,
    pub when: NaiveDateTime,
    pub auto: bool,
    pub size: u64,
}

pub fn list_backups() -> Vec<Backup> {
    let mut v: Vec<Backup> = fs::read_dir(&paths().backups_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("sl2")))
        .map(|e| {
            let path = e.path();
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let meta = e.metadata().ok();
            let (when, mut label) = match stem.get(..19).and_then(|s| NaiveDateTime::parse_from_str(s, STAMP).ok()) {
                Some(t) => (t, stem[19..].trim().to_string()),
                None => (meta.as_ref().and_then(|m| m.modified().ok()).map(|t| to_local(t).naive_local()).unwrap_or_default(), stem.clone()),
            };
            let auto = label.starts_with(AUTO);
            if auto {
                label = label[AUTO.len()..].trim().to_string();
            }
            Backup { path, label, when, auto, size: meta.map(|m| m.len()).unwrap_or(0) }
        })
        .collect();
    v.sort_by(|a, b| b.when.cmp(&a.when));
    v
}

fn sanitize(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(60).collect();
    if s.trim().is_empty() { "backup".into() } else { s.trim().to_string() }
}

/// Opens a file other processes (the game) may have open for writing.
fn open_shared(p: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(0x7).open(p)
}

fn hash(p: &Path) -> io::Result<Vec<u8>> {
    let mut f = open_shared(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().to_vec())
}

fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    ma.len() == mb.len() && matches!((hash(a), hash(b)), (Ok(x), Ok(y)) if x == y)
}

/// Copies the live save into Save Backups. Auto backups are skipped if nothing changed.
pub fn create_backup(label: &str, auto: bool, keep: u32) -> Result<Option<PathBuf>, String> {
    let live = live_save().ok_or("Couldn't find your Elden Ring save (ER0000.sl2).")?;
    if auto {
        if let Some(newest) = list_backups().first() {
            if same_content(&live, &newest.path) {
                return Ok(None);
            }
        }
    }
    let name = format!("{} {}{}.sl2", Local::now().format(STAMP), if auto { "[auto] " } else { "" }, sanitize(label));
    let dest = paths().backups_dir.join(name);
    // Copied under another extension first: a half-written .sl2 would be listed and restorable.
    let part = dest.with_extension("part");
    let copied = open_shared(&live).and_then(|mut src| io::copy(&mut src, &mut File::create(&part)?)).and_then(|_| fs::rename(&part, &dest));
    if let Err(e) = copied {
        let _ = fs::remove_file(&part);
        return Err(e.to_string());
    }
    if auto {
        for old in list_backups().into_iter().filter(|b| b.auto).skip(keep.max(1) as usize) {
            let _ = sys::recycle(&old.path);
        }
    }
    Ok(Some(dest))
}

/// Label of the kept (never rotated) backup made when a bingo game first runs on a save that was
/// played some other way since, so there's always a clean copy to go back to before playing online.
pub const BEFORE_BINGO: &str = "before bingo";

pub fn newest_before_bingo() -> Option<Backup> {
    list_backups().into_iter().find(|b| !b.auto && b.label == BEFORE_BINGO)
}

pub fn restore_backup(b: &Backup, keep: u32) -> Result<(), String> {
    let live = live_save().ok_or("Couldn't find your Elden Ring save folder.")?;
    create_backup("before restore", true, keep)?;
    for target in [live.clone(), live.with_extension("sl2.bak")] {
        fs::copy(&b.path, &target).map_err(|e| e.to_string())?;
        // Fresh timestamp so Steam Cloud treats the restored file as newest.
        if let Ok(f) = OpenOptions::new().write(true).open(&target) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
    Ok(())
}

pub fn rename_backup(b: &Backup, label: &str) -> Result<(), String> {
    let dest = paths().backups_dir.join(format!("{} {}.sl2", b.when.format(STAMP), sanitize(label)));
    if dest == b.path {
        return Ok(());
    }
    if dest.exists() {
        return Err("A backup with that name already exists.".into());
    }
    fs::rename(&b.path, dest).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- boards

#[derive(Clone)]
pub struct Goal {
    pub name: String,
}

#[derive(Clone)]
pub struct Board {
    pub path: PathBuf,
    pub name: String,
    pub goals: Vec<Goal>,
}

pub fn pretty(file: &Path) -> String {
    let mut s = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(i) = s.rfind(" (") {
        if s.ends_with(')') && s[i + 2..s.len() - 1].chars().all(|c| c.is_ascii_digit()) {
            s.truncate(i);
        }
    }
    let s = s.replace(['-', '_'], " ");
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if c.is_ascii_digit() && prev.is_some_and(|p| p.is_alphabetic()) {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            cs.next().map(|f| f.to_uppercase().chain(cs.flat_map(|c| c.to_lowercase())).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn load_board(p: &Path) -> Option<Board> {
    if fs::metadata(p).ok()?.len() > 2_000_000 {
        return None;
    }
    let text = fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let goals: Vec<Goal> = v
        .as_array()?
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.trim().to_string();
            Some(Goal { name })
        })
        .collect();
    (!goals.is_empty()).then(|| Board { path: p.to_path_buf(), name: pretty(p), goals })
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .collect()
}

pub fn list_boards() -> Vec<Board> {
    let mut v: Vec<Board> = json_files(&paths().boards_dir).iter().filter_map(|p| load_board(p)).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn file_hash(p: &Path) -> Option<Vec<u8>> {
    fs::read(p).ok().map(|b| Sha256::digest(b).to_vec())
}

fn library_hashes() -> Vec<Vec<u8>> {
    json_files(&paths().boards_dir).iter().filter_map(|p| file_hash(p)).collect()
}

fn download_candidates() -> Vec<PathBuf> {
    json_files(&paths().downloads).into_iter().filter(|p| load_board(p).is_some()).collect()
}

pub fn pending_in_downloads() -> usize {
    let have = library_hashes();
    let mut seen = Vec::new();
    for h in download_candidates().iter().filter_map(|p| file_hash(p)) {
        if !have.contains(&h) && !seen.contains(&h) {
            seen.push(h);
        }
    }
    seen.len()
}

/// Copies a goal-list into the library. False if invalid or already there.
pub fn import_board(src: &Path) -> bool {
    if load_board(src).is_none() {
        return false;
    }
    let Some(h) = file_hash(src) else { return false };
    if library_hashes().contains(&h) {
        return false;
    }
    let mut base = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or("board".into());
    if let Some(i) = base.rfind(" (") {
        if base.ends_with(')') {
            base.truncate(i);
        }
    }
    let dir = &paths().boards_dir;
    let mut dest = dir.join(format!("{base}.json"));
    let mut i = 2;
    while dest.exists() {
        dest = dir.join(format!("{base} {i}.json"));
        i += 1;
    }
    fs::copy(src, dest).is_ok()
}

pub fn import_from_downloads() -> usize {
    download_candidates().iter().filter(|p| import_board(p)).count()
}

// ---------------------------------------------------------------- run log (same file and keys as v1)

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Run {
    pub date: String,
    pub seed: String,
    pub board: String,
    pub opponent: String,
    pub result: String,
    pub score: String,
    pub time: String,
    pub notes: String,
    /// Race code at the time of the run.
    pub code: String,
}

impl Run {
    pub fn date_local(&self) -> Option<DateTime<Local>> {
        parse_time(&self.date, false)
    }
}

fn runs_file() -> PathBuf {
    paths().data_dir.join("runs.json")
}

pub fn load_runs() -> Vec<Run> {
    let Ok(text) = fs::read_to_string(runs_file()) else { return vec![] };
    let mut v: Vec<Run> = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|_| {
        // Unreadable: keep a copy, since the next save would overwrite it with an empty log.
        let _ = fs::copy(runs_file(), runs_file().with_extension("bad.json"));
        vec![]
    });
    v.sort_by_key(|r| std::cmp::Reverse(r.date_local()));
    v
}

pub fn save_runs(runs: &[Run]) {
    if let Ok(s) = serde_json::to_string_pretty(runs) {
        let _ = write_atomic(&runs_file(), s);
    }
}

pub fn runs_csv(runs: &[Run]) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("\u{feff}Date,Opponent,Result,Squares,Time,Board,Seed,Notes\r\n");
    for r in runs {
        let d = r.date_local().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
        out += &[q(&d), q(&r.opponent), q(&r.result), q(&r.score), q(&r.time), q(&r.board), q(&r.seed), q(&r.notes)].join(",");
        out += "\r\n";
    }
    out
}

// ---------------------------------------------------------------- setup checks

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Bad,
    Warn,
    Info,
    Ok,
}

#[derive(Clone, PartialEq)]
pub enum Fix {
    OpenUrl(String),
    LocateGame,
    FixBoardPath,
    OpenRandomizer,
    StartSteam,
    OpenFolder(PathBuf),
    BackupNow,
    ImportDownloads,
}

#[derive(Clone)]
pub struct Check {
    pub level: Level,
    pub title: String,
    pub detail: String,
    pub fix: Option<(String, Fix)>,
}

impl Check {
    pub fn is_issue(&self) -> bool {
        matches!(self.level, Level::Bad | Level::Warn) || self.level == Level::Info && self.fix.is_some()
    }
}

fn desktop8_installed() -> bool {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or(PathBuf::from(r"C:\Program Files"));
    fs::read_dir(pf.join("dotnet").join("shared").join("Microsoft.WindowsDesktop.App"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("8."))
}

/// The Randomizer deletes and rewrites `Resources\ME2\bingo` on every run, and its delete fails
/// ("Access to the path … is denied") if any folder in there is marked read-only. Clear the flag.
pub fn unlock_randomizer_output() {
    fn walk(p: &Path) {
        let Ok(meta) = fs::metadata(p) else { return };
        let mut perm = meta.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = fs::set_permissions(p, perm);
        }
        if meta.is_dir() {
            for e in fs::read_dir(p).into_iter().flatten().flatten() {
                walk(&e.path());
            }
        }
    }
    walk(&paths().me2_dir.join("bingo"));
}

pub fn run_checks(settings: &Settings, steam_running: bool) -> Vec<Check> {
    let p = paths();
    let mut r = Vec::new();
    let mut add = |level, title: &str, detail: String, fix: Option<(&str, Fix)>| {
        r.push(Check { level, title: title.into(), detail, fix: fix.map(|(a, b)| (a.to_string(), b)) })
    };
    let ver = |p: &Path| sys::file_version(p).unwrap_or_else(|| "Installed".into());
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });

    if p.board_exe.is_file() {
        add(Level::Ok, "Bingo Board", ver(&p.board_exe), None);
    } else {
        add(Level::Bad, "Bingo Board missing", "EldenBingo.exe isn't in the Bingo Board folder".into(), None);
    }
    if desktop8_installed() {
        add(Level::Ok, ".NET 8 Desktop Runtime", "Installed".into(), None);
    } else {
        add(Level::Bad, ".NET 8 Desktop Runtime missing", "The Bingo Board needs it".into(), Some(("Download", Fix::OpenUrl("https://dotnet.microsoft.com/download/dotnet/8.0".into()))));
    }
    match rando_exe() {
        Some(e) => add(Level::Ok, "Randomizer", e.file_stem().unwrap_or_default().to_string_lossy().into_owned(), None),
        None => add(Level::Bad, "Randomizer missing", "No exe in the Bingo Randomizer folder".into(), None),
    }
    match (p.me2_launcher.is_file(), fs::read_to_string(&p.me2_config)) {
        (true, Ok(cfg)) if cfg.contains("path = \"bingo\"") => add(Level::Ok, "ModEngine 2", "Configured".into(), None),
        (true, Ok(_)) => add(Level::Warn, "ModEngine 2", "config_bingo.toml doesn't point at the bingo folder".into(), None),
        _ => add(Level::Bad, "ModEngine 2 missing", p.me2_dir.display().to_string(), None),
    }
    match mtime(&p.regulation) {
        Some(t) => add(Level::Ok, "Regulation", format!("Randomized {}", ago(to_local(t))), None),
        None => add(Level::Warn, "Not randomized yet", "Run the Randomizer once".into(), Some(("Open Randomizer", Fix::OpenRandomizer))),
    }
    if p.debug_exe.is_file() {
        add(Level::Ok, "Debug Tool", ver(&p.debug_exe), None);
    }

    match find_game_exe(settings.game_exe_override.as_deref()) {
        None => add(Level::Bad, "Elden Ring not found", "Not in any Steam library".into(), Some(("Locate", Fix::LocateGame))),
        Some(game) => {
            let dir = game.parent().unwrap().to_path_buf();
            let game_ver = ver(&game);
            if !settings.rando_game_version.is_empty() && settings.rando_game_version != game_ver {
                add(
                    Level::Warn,
                    "Elden Ring updated since your last randomize",
                    format!("{} to {}. If the Randomizer starts crashing, it may need an update.", settings.rando_game_version, game_ver),
                    None,
                );
            }
            add(Level::Ok, "Elden Ring", game_ver, None);
            if let Some(bp) = board_config_get("GamePath").filter(|s| !s.is_empty()) {
                let same = bp.trim_end_matches('\\').eq_ignore_ascii_case(dir.to_string_lossy().trim_end_matches('\\'));
                if same {
                    add(Level::Ok, "Board game path", "Matches install".into(), None);
                } else {
                    add(Level::Warn, "Board has the wrong game path", bp, Some(("Fix", Fix::FixBoardPath)));
                }
            }
            const VANILLA: [&str; 6] = ["amd_ags_x64.dll", "bink2w64.dll", "eossdk-win64-shipping.dll", "oo2core_6_win64.dll", "oo2core_8_win64.dll", "steam_api64.dll"];
            let extras: Vec<String> = fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .filter(|n| n.ends_with(".dll") && !VANILLA.contains(&n.as_str()))
                .collect();
            if !extras.is_empty() {
                let mods = fs::read_dir(dir.join("mods")).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".dll")).count();
                let only_loader = extras.len() == 1 && extras[0] == "dinput8.dll" && mods == 0;
                let names = extras.iter().map(|n| if n == "dinput8.dll" { "dinput8.dll (mod loader)".to_string() } else { n.clone() }).collect::<Vec<_>>().join(", ");
                add(if only_loader { Level::Ok } else { Level::Warn }, "Extra DLLs in the game folder", format!("{names}. They load in bingo runs too."), Some(("Open folder", Fix::OpenFolder(dir.clone()))));
            }
            if dir.join("start_protected_game.exe.original").is_file() {
                add(Level::Info, "Steam's Play button", "Doesn't load the bingo mod. Launch from here instead.".into(), None);
            }
        }
    }

    if steam_running {
        add(Level::Ok, "Steam", "Running".into(), None);
    } else {
        add(Level::Warn, "Steam isn't running", "Needed to launch the game".into(), Some(("Start Steam", Fix::StartSteam)));
    }

    if live_save().is_some() {
        let b = list_backups();
        match b.first() {
            None => add(Level::Warn, "No save backups", "Back up before your first run".into(), Some(("Back up", Fix::BackupNow))),
            Some(n) => add(Level::Ok, "Save backups", format!("{}, newest {}", plural(b.len(), "backup"), when(n.when).to_lowercase()), None),
        }
    } else {
        add(Level::Warn, "No save file", "Start the game once to create ER0000.sl2".into(), None);
    }

    let pending = pending_in_downloads();
    if pending > 0 {
        add(Level::Info, &format!("{} in Downloads", plural(pending, "board file")), "Not in your boards yet".into(), Some(("Import", Fix::ImportDownloads)));
    }
    r
}

// ---------------------------------------------------------------- time formatting

pub fn ago(t: DateTime<Local>) -> String {
    let s = (Local::now() - t).num_seconds();
    match s {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ if s < 60 * 86_400 => format!("{} days ago", s / 86_400),
        _ => t.format("%-d %b %Y").to_string(),
    }
}

pub fn when(t: NaiveDateTime) -> String {
    let today = Local::now().date_naive();
    if t.date() == today {
        format!("Today, {}", t.format("%H:%M"))
    } else if Some(t.date()) == today.pred_opt() {
        format!("Yesterday, {}", t.format("%H:%M"))
    } else if t.format("%Y").to_string() == today.format("%Y").to_string() {
        t.format("%-d %b, %H:%M").to_string()
    } else {
        t.format("%-d %b %Y").to_string()
    }
}
