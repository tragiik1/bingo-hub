//! Self-update from GitHub Releases.
//!
//! A release carries `Bingo-Hub.exe`. The running exe can't be overwritten, but it can be renamed,
//! so updating is: download next to us, rename ourselves to `.old`, move the new file in, restart.
//! The `.old` file is removed on the next start.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// "owner/repo", baked in at build time from `repo.txt` (see build.rs). Empty = updates off.
pub const REPO: &str = env!("BINGOHUB_REPO");
pub const ASSET: &str = "Bingo-Hub.exe";

#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub size: u64,
}

fn curl() -> Command {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or(PathBuf::from(r"C:\Windows"));
    let mut c = Command::new(sys.join("System32").join("curl.exe"));
    c.creation_flags(crate::sys::CREATE_NO_WINDOW);
    c.args(["--silent", "--show-error", "--location", "--fail", "--user-agent", "BingoHub"]);
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

/// Latest release if it's newer than this build.
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
    if !is_newer(&tag, VERSION) {
        return Ok(None);
    }
    let asset = v["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(ASSET)));
    let Some(asset) = asset else { return Ok(None) };
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"].as_str().unwrap_or_default().trim().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
    }))
}

/// Downloads the new exe, swaps it in and starts it. The caller should close the window after.
pub fn install(r: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&new);

    let ok = curl().args(["--max-time", "300", "--output"]).arg(&new).arg(&r.url).status().map(|s| s.success()).unwrap_or(false);
    let bytes = std::fs::read(&new).unwrap_or_default();
    if !ok || bytes.len() < 1024 || &bytes[..2] != b"MZ" || (r.size > 0 && bytes.len() as u64 != r.size) {
        let _ = std::fs::remove_file(&new);
        return Err("The download didn't complete. Check your connection and try again.".into());
    }

    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("Couldn't replace Bingo Hub: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("Couldn't replace Bingo Hub: {e}"));
    }
    Command::new(&exe).arg("--updated").current_dir(exe.parent().unwrap()).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Removes leftovers from a previous update.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        for ext in ["exe.old", "exe.new"] {
            let _ = std::fs::remove_file(exe.with_extension(ext));
        }
    }
}
