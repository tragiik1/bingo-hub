//! Updates from GitHub Releases: the hub itself and the bundled tools.
//!
//! A release carries `Bingo-Hub.exe`, one zip per tool, and `manifest.json` listing a fingerprint
//! (file hashes) for each tool. A tool is updated when its installed files don't match.
//!
//! The running exe can't be overwritten, but it can be renamed, so the hub updates by downloading
//! next to itself, renaming itself to `.old`, moving the new file in and restarting. The `.old`
//! file is removed on the next start. Tool folders are swapped the same way (old folder renamed,
//! new one moved in), so a failed update never leaves a half-written folder behind.

use std::collections::BTreeMap;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::core;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// "owner/repo", baked in at build time from `repo.txt` (see build.rs). Empty = updates off.
pub const REPO: &str = env!("BINGOHUB_REPO");
pub const ASSET: &str = "Bingo-Hub.exe";
const MANIFEST: &str = "manifest.json";

#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct Component {
    pub name: String,
    pub folder: String,
    pub asset: String,
    /// Copy files over the folder instead of replacing it (keeps the user's own files, e.g. boards).
    #[serde(default)]
    pub merge: bool,
    /// Relative path -> SHA-256 of the files that identify this version.
    pub files: BTreeMap<String, String>,
    /// Set when the zip lives in an earlier release (the tool didn't change); else found by `asset`.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize)]
struct Manifest {
    components: Vec<Component>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    /// Set when the hub itself is out of date.
    pub hub: Option<(String, u64)>,
    /// Tools whose installed files don't match this release.
    pub tools: Vec<Component>,
}

impl Release {
    /// "Bingo Board and Randomizer" style summary of what will change.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.hub.is_some() {
            parts.push(format!("Bingo Hub {}", self.version));
        }
        parts.extend(self.tools.iter().map(|t| t.name.clone()));
        match parts.len() {
            0 => String::new(),
            1 => parts.remove(0),
            n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
        }
    }
}

fn curl() -> Command {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or(PathBuf::from(r"C:\Windows"));
    let mut c = Command::new(sys.join("System32").join("curl.exe"));
    c.creation_flags(crate::sys::CREATE_NO_WINDOW);
    c.args(["--silent", "--show-error", "--location", "--fail", "--user-agent", "BingoHub"]);
    c
}

fn tar() -> Command {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or(PathBuf::from(r"C:\Windows"));
    let mut c = Command::new(sys.join("System32").join("tar.exe"));
    c.creation_flags(crate::sys::CREATE_NO_WINDOW);
    c
}

fn parse_version(s: &str) -> Vec<u64> {
    s.trim().trim_start_matches(['v', 'V']).split(['.', '-']).map_while(|p| p.parse().ok()).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

/// Release feed. BINGOHUB_UPDATE_API overrides it for local testing.
fn feed() -> Option<String> {
    std::env::var("BINGOHUB_UPDATE_API").ok().or_else(|| (!REPO.is_empty()).then(|| format!("https://api.github.com/repos/{REPO}/releases/latest")))
}

pub fn enabled() -> bool {
    feed().is_some()
}

fn root() -> PathBuf {
    core::paths().board_dir.parent().map(Path::to_path_buf).unwrap_or_default()
}

fn sha256_hex(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// True if every identifying file is in `dir` with the expected hash.
fn matches(c: &Component, dir: &Path) -> bool {
    c.files.iter().all(|(rel, want)| sha256_hex(&dir.join(rel)).is_some_and(|got| got.eq_ignore_ascii_case(want)))
}

fn up_to_date(c: &Component) -> bool {
    matches(c, &root().join(&c.folder))
}

/// What the latest release would change here, if anything.
pub fn check() -> Result<Option<Release>, String> {
    let Some(url) = feed() else { return Ok(None) };
    let out = curl()
        .args(["--max-time", "15", "--header", "Accept: application/vnd.github+json"])
        .arg(url)
        .output()
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    if !out.status.success() {
        return Err("Couldn't reach GitHub to check for updates.".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "Unexpected reply from GitHub.".to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |name: &str| assets.iter().find(|a| a["name"].as_str() == Some(name)).map(|a| {
        (a["browser_download_url"].as_str().unwrap_or_default().to_string(), a["size"].as_u64().unwrap_or(0))
    });

    let hub = if is_newer(&tag, VERSION) { find(ASSET) } else { None };

    let mut tools = Vec::new();
    if let Some((murl, _)) = find(MANIFEST) {
        let m = curl().args(["--max-time", "15"]).arg(&murl).output().map_err(|e| e.to_string())?;
        if m.status.success() {
            if let Ok(man) = serde_json::from_slice::<Manifest>(&m.stdout) {
                for mut c in man.components {
                    if up_to_date(&c) {
                        continue;
                    }
                    if c.url.is_empty() {
                        let Some((u, s)) = find(&c.asset) else { continue };
                        c.url = u;
                        c.size = s;
                    }
                    tools.push(c);
                }
            }
        }
    }

    if hub.is_none() && tools.is_empty() {
        return Ok(None);
    }
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"].as_str().unwrap_or_default().trim().to_string(),
        hub,
        tools,
    }))
}

fn download(url: &str, size: u64, to: &Path) -> Result<(), String> {
    let _ = fs::remove_file(to);
    let ok = curl().args(["--max-time", "900", "--output"]).arg(to).arg(url).status().map(|s| s.success()).unwrap_or(false);
    let len = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
    if !ok || len == 0 || (size > 0 && len != size) {
        let _ = fs::remove_file(to);
        return Err("A download didn't complete. Check your connection and try again.".into());
    }
    Ok(())
}

/// Which running program would keep this tool's folder locked.
fn running_in(folder: &str) -> Option<&'static str> {
    let mut pc = crate::sys::ProcCache::default();
    pc.refresh();
    match folder {
        "Bingo Board" => pc.pid_of("eldenbingo.exe").map(|_| "the Bingo Board"),
        "Bingo Randomizer" => core::rando_exe()
            .and_then(|e| e.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .and_then(|n| pc.pid_of(&n))
            .map(|_| "the Randomizer"),
        "Debug Tool" => pc.pid_of("elden ring debug tool.exe").map(|_| "the Debug Tool"),
        _ => None,
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dest)?;
        } else {
            fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Downloads and swaps in the given tools. Returns the names that were updated.
pub fn install_tools(tools: &[Component]) -> Result<Vec<String>, String> {
    let root = root();
    let work = core::paths().data_dir.join(".update");
    let mut done = Vec::new();
    for c in tools {
        if let Some(app) = running_in(&c.folder) {
            return Err(format!("Close {app} so it can be updated, then try again."));
        }
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("x")).map_err(|e| e.to_string())?;
        let zip = work.join(&c.asset);
        download(&c.url, c.size, &zip)?;
        let ok = tar().arg("-xf").arg(&zip).arg("-C").arg(work.join("x")).status().map(|s| s.success()).unwrap_or(false);
        let fresh = work.join("x").join(&c.folder);
        if !ok || !fresh.is_dir() || !matches(c, &fresh) {
            return Err(format!("The {} download was damaged. Try again.", c.name));
        }
        let target = root.join(&c.folder);
        if c.merge {
            let copied = copy_tree(&fresh, &target).is_ok()
                || Command::new("cmd")
                    .creation_flags(crate::sys::CREATE_NO_WINDOW)
                    .raw_arg(format!("/s /c \"xcopy /e /y /i /q \"{}\" \"{}\" >nul\"", fresh.display(), target.display()))
                    .status()
                    .is_ok_and(|s| s.success());
            if !copied {
                return Err(format!("Couldn't update {}. Windows blocked writing to the folder.", c.name));
            }
        } else {
            let old = root.join(format!("{}.old", c.folder));
            let _ = fs::remove_dir_all(&old);
            if target.exists() {
                move_path(&target, &old).map_err(|_| format!("Couldn't replace {}. Close anything using it and try again.", c.name))?;
            }
            if let Err(e) = move_path(&fresh, &target) {
                let _ = move_path(&old, &target);
                return Err(format!("Couldn't replace {}: {e}", c.name));
            }
            let _ = fs::remove_dir_all(&old);
        }
        done.push(c.name.clone());
    }
    let _ = fs::remove_dir_all(&work);
    core::unlock_randomizer_output();
    Ok(done)
}

/// Downloads the new hub exe, swaps it in and starts it. The caller should close the window after.
pub fn install_hub(url: &str, size: u64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    download(url, size, &new)?;
    if fs::read(&new).map(|b| b.len() < 1024 || &b[..2] != b"MZ").unwrap_or(true) {
        let _ = fs::remove_file(&new);
        return Err("The download was damaged. Try again.".into());
    }
    let _ = fs::remove_file(&old);
    let swapped = fs::rename(&exe, &old).is_ok() && {
        if fs::rename(&new, &exe).is_ok() {
            true
        } else {
            let _ = fs::rename(&old, &exe);
            false
        }
    };
    if swapped {
        Command::new(&exe).arg("--updated").current_dir(exe.parent().unwrap()).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    // Windows refused (Controlled Folder Access, or antivirus holding the running exe).
    // Hand the swap to the command shell, which those trust, to finish once we've closed.
    deferred_swap(&exe, &new)
}

/// Writes a small batch script that waits for this process to exit, moves `new` over `exe`
/// (retrying while anything still holds it) and starts the hub again.
fn deferred_swap(exe: &Path, new: &Path) -> Result<(), String> {
    let pid = std::process::id();
    let (exe_s, new_s) = (exe.display().to_string(), new.display().to_string());
    // Windows tools by full path, so nothing else on PATH can stand in for them.
    let sys32 = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()) + r"\System32";
    let script = format!(
        "@echo off\r\n\
         for /l %%i in (1,1,30) do (\r\n\
           \"{sys32}\\tasklist.exe\" /fi \"PID eq {pid}\" | \"{sys32}\\find.exe\" \"{pid}\" >nul || goto swap\r\n\
           \"{sys32}\\ping.exe\" -n 2 127.0.0.1 >nul\r\n\
         )\r\n\
         :swap\r\n\
         for /l %%i in (1,1,15) do (\r\n\
           move /y \"{new_s}\" \"{exe_s}\" >nul 2>&1 && goto run\r\n\
           \"{sys32}\\ping.exe\" -n 2 127.0.0.1 >nul\r\n\
         )\r\n\
         start \"\" \"{exe_s}\"\r\n\
         goto end\r\n\
         :run\r\n\
         start \"\" \"{exe_s}\" --updated\r\n\
         :end\r\n\
         (goto) 2>nul & del \"%~f0\"\r\n"
    );
    let path = std::env::temp_dir().join("bingo-hub-update.cmd");
    fs::write(&path, script).map_err(|e| format!("Couldn't prepare the update: {e}"))?;
    let mut c = Command::new("cmd");
    c.creation_flags(crate::sys::CREATE_NO_WINDOW).raw_arg(format!("/c \"{}\"", path.display()));
    c.spawn().map_err(|e| format!("Couldn't start the update: {e}"))?;
    Ok(())
}

/// Rename that falls back to the command shell's `move` when Windows refuses the hub itself.
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) => {
            let ok = Command::new("cmd")
                .creation_flags(crate::sys::CREATE_NO_WINDOW)
                .raw_arg(format!("/s /c \"move /y \"{}\" \"{}\" >nul\"", from.display(), to.display()))
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok && to.exists() && !from.exists() { Ok(()) } else { Err(e) }
        }
    }
}

/// Removes leftovers from a previous update.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        for ext in ["exe.old", "exe.new"] {
            let _ = fs::remove_file(exe.with_extension(ext));
        }
    }
    let _ = fs::remove_dir_all(core::paths().data_dir.join(".update"));
}
