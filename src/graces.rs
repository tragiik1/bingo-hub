//! Race graces: sets the group's standard graces on a fresh bingo character and clears the rest,
//! by flipping the game's event flags directly (what the "Grace Unlocker Bingo" and "Remove
//! Unwanted Graces Bingo" Cheat Engine scripts did).
//!
//! Event flags live in the game's CSEventFlagMan (a.k.a. CSFD4VirtualMemoryFlag). A flag id is
//! split into a block (id / divisor) and an index; blocks sit in a std::map keyed by block number,
//! and each flag is one bit, big-endian within its byte.
//!
//! Only used on bingo launches (ModEngine 2 loaded, EAC off). Automatic setting never touches a
//! character that has been played (see NEW_LIMIT / FRESH_LIMIT), and only writes once the flags
//! read the same twice, 2 s apart, so a real character caught mid-load (briefly reading as having
//! no graces) is left alone. The manual button and hotkey ask before going past the limit.

use std::collections::HashMap;
use std::ffi::c_void;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Threading::*;

/// The race graces.
pub const WANTED: [(u32, &str); 11] = [
    (76154, "Ailing Village Outskirts"),
    (76413, "Inner Aeonia"),
    (76203, "Scenic Isle"),
    (76225, "Ruined Labyrinth"),
    (76303, "Altus Highway Junction"),
    (76353, "Road of Iniquity"),
    (76521, "Snow Valley Ruins Overlook"),
    (71222, "Siofra River Bank"),
    (71216, "Lake of Rot Shoreside"),
    (76551, "Inner Consecrated Snowfield"),
    (71504, "Haligtree Roots"),
];

/// Every grace flag the table handles (315); the ones not in WANTED are cleared.
pub const ALL: [u32; 315] = [
    82001, 71190, 76101, 76100, 76111, 76120, 76103, 76104, 76105, 76106, 76108, 76110, 76113, 76114, 76116, 76119, 73002, 73004, 73103,
    73115, 73100, 73117, 73201, 71800, 71801, 76102, 71002, 71001, 76118, 76117, 73011, 73410, 73412, 76150, 76151, 76152, 76153, 76154,
    76155, 76156, 76162, 76157, 76158, 76159, 76160, 76161, 73001, 73000, 73101, 73102, 73200, 71008, 71003, 71004, 71005, 71006, 71007,
    71000, 76200, 76202, 76201, 76204, 76217, 76223, 76222, 76221, 76244, 76206, 76203, 76205, 76225, 76216, 76224, 76237, 76234, 76236,
    76219, 76245, 76226, 76247, 76218, 76215, 76220, 76243, 76242, 76210, 76233, 76214, 76231, 76230, 76212, 76213, 76232, 76211, 76241,
    76227, 73106, 76238, 73005, 73006, 73105, 73421, 76228, 76229, 73202, 73003, 73104, 73420, 76235, 73422, 76208, 76240, 76207, 76239,
    76209, 73900, 73901, 73902, 76252, 76251, 76250, 71402, 71401, 71400, 71403, 76300, 76303, 76301, 76306, 76302, 76304, 76305, 76307,
    76321, 76320, 76313, 76308, 73205, 76322, 73204, 73118, 73119, 73008, 73012, 76350, 76356, 76351, 73009, 76352, 76357, 76353, 73107,
    76354, 76355, 73109, 73007, 73013, 73010, 76314, 73430, 73432, 76311, 76310, 76312, 76309, 73431, 71606, 71605, 71604, 71603, 71600,
    71607, 71601, 71602, 71103, 71102, 71101, 71100, 71109, 71108, 71107, 71105, 71104, 73500, 73502, 73504, 73503, 73501, 71125, 71122,
    71120, 71121, 71123, 71124, 71900, 76403, 76405, 76404, 76415, 76418, 76402, 76401, 76414, 76416, 76400, 76409, 76411, 73120, 73015,
    76420, 76410, 73207, 73121, 76417, 73014, 73257, 76419, 73208, 76422, 73016, 76406, 76407, 76412, 76413, 76454, 73440, 73441, 73110,
    76452, 76450, 76456, 76453, 73460, 76451, 76455, 73111, 73451, 73450, 76500, 76502, 73020, 76503, 76522, 76524, 76523, 76505, 76504,
    76521, 73122, 76520, 76501, 76507, 76509, 76508, 76510, 76506, 73018, 73017, 76653, 73112, 76550, 73019, 76551, 76652, 73211, 71506,
    71505, 71507, 71508, 71503, 71502, 71504, 71500, 71501, 71310, 71303, 71304, 71306, 71302, 71308, 71309, 71307, 71301, 71300, 71305,
    71213, 71212, 71211, 71240, 71210, 71214, 71215, 71219, 71218, 71216, 71271, 71224, 71225, 71220, 71221, 71226, 71227, 71222, 71270,
    71223, 71250, 71252, 71253, 71251, 71235, 71233, 71232, 71230, 71231, 71234,
];

/// A character with more graces than this isn't a fresh bingo character: leave it alone.
pub const FRESH_LIMIT: usize = 40;

/// Off the separate bingo save, automatic setting only touches a character this new: at most the
/// tutorial graces. A brand-new character (name screen onwards) has none, and anything played
/// longer may be someone's real playthrough.
pub const NEW_LIMIT: usize = 3;

/// Instructions that load the CSEventFlagMan pointer: `mov rdi, [rip+X]` followed by a null check.
const PATTERNS: [&str; 2] = ["48 8B 3D ?? ?? ?? ?? 48 85 FF 74 ?? 48 8B 49", "48 8B 3D ?? ?? ?? ?? 48 85 FF ?? ?? 32 C0 E9"];

/// `Game::open` error while the game is still starting or closing (retry shortly).
pub const NOT_READY: &str = "The game is still starting. Graces can be set once a character is loaded.";

pub struct Game {
    h: HANDLE,
    /// Static address that holds the CSEventFlagMan pointer.
    flag_man: usize,
}

impl Drop for Game {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.h) };
    }
}

#[derive(Debug)]
pub enum Outcome {
    /// Applied; how many other graces were cleared.
    Applied(usize),
    AlreadySet,
    /// Character already has this many graces.
    NotFresh(usize),
    /// No character loaded yet (flags not readable).
    NotLoaded,
    /// The flags were written but didn't read back as set.
    WriteFailed,
}

/// What automatic setting is doing in the current game, for the UI.
#[derive(Clone, PartialEq, Default, Debug)]
pub enum Status {
    #[default]
    Idle,
    /// Game running, no character readable yet.
    Waiting,
    Set,
    /// Left alone: the character already has this many graces.
    Skipped(usize),
    Error(String),
}

fn parse_pattern(p: &str) -> Vec<Option<u8>> {
    p.split_whitespace().map(|b| if b.starts_with('?') { None } else { u8::from_str_radix(b, 16).ok() }).collect()
}

fn find_all(hay: &[u8], pat: &[Option<u8>]) -> Vec<usize> {
    let first = pat[0].unwrap();
    let mut out = Vec::new();
    let mut i = 0;
    while i + pat.len() <= hay.len() {
        if hay[i] == first && pat.iter().enumerate().all(|(k, b)| b.is_none_or(|v| hay[i + k] == v)) {
            out.push(i);
        }
        i += 1;
    }
    out
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    hay.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect()
}

/// Target of a RIP-relative instruction with its disp32 at byte 3 and length 7.
fn rip_target(section: usize, code: &[u8], at: usize) -> usize {
    let rel = i32::from_le_bytes(code[at + 3..at + 7].try_into().unwrap()) as isize;
    (section + at + 7).wrapping_add_signed(rel)
}

fn module(pid: u32, name: &str) -> Option<(usize, usize)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut m: MODULEENTRY32W = std::mem::zeroed();
        m.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut found = None;
        if Module32FirstW(snap, &mut m) != 0 {
            loop {
                let n = m.szModule.iter().position(|&c| c == 0).unwrap_or(m.szModule.len());
                if String::from_utf16_lossy(&m.szModule[..n]).eq_ignore_ascii_case(name) {
                    found = Some((m.modBaseAddr as usize, m.modBaseSize as usize));
                    break;
                }
                if Module32NextW(snap, &mut m) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

impl Game {
    fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
        if addr == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        let mut got = 0usize;
        let ok = unsafe { ReadProcessMemory(self.h, addr as *const c_void, buf.as_mut_ptr() as *mut c_void, len, &mut got) } != 0;
        (ok && got == len).then_some(buf)
    }
    fn u8(&self, a: usize) -> Option<u8> {
        self.read_bytes(a, 1).map(|b| b[0])
    }
    fn u32(&self, a: usize) -> Option<u32> {
        self.read_bytes(a, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn ptr(&self, a: usize) -> Option<usize> {
        self.read_bytes(a, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize).filter(|&p| p != 0)
    }
    fn write_u8(&self, a: usize, v: u8) -> bool {
        let mut n = 0usize;
        unsafe { WriteProcessMemory(self.h, a as *const c_void, &v as *const u8 as *const c_void, 1, &mut n) != 0 && n == 1 }
    }

    /// Opens the game and locates the event flag manager.
    pub fn open(pid: u32) -> Result<Game, String> {
        let access = PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION;
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            return Err(if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
                "Windows blocked access to the game. If Steam or Elden Ring runs as administrator, run Bingo Hub as administrator too."
            } else {
                "Couldn't access the game. Graces only work on bingo launches (anti-cheat off)."
            }
            .into());
        }
        let mut g = Game { h, flag_man: 0 };
        // Finding the flag manager means scanning the game's code, so remember it per game process.
        static FOUND: std::sync::Mutex<Option<(u32, usize)>> = std::sync::Mutex::new(None);
        let cached = *FOUND.lock().unwrap();
        if let Some((_, addr)) = cached.filter(|c| c.0 == pid) {
            g.flag_man = addr;
            if g.looks_valid() {
                return Ok(g);
            }
        }
        let (base, _) = module(pid, "eldenring.exe").ok_or("Couldn't read the game's memory.")?;

        // Sections. The game has two named .text (the second is anti-tamper code), so go by
        // flags: code = executable sections, strings = the first read-only data section.
        let hdr = g.read_bytes(base, 0x1000).ok_or("Couldn't read the game's memory.")?;
        let nt = u32::from_le_bytes(hdr[0x3C..0x40].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(hdr[nt + 6..nt + 8].try_into().unwrap()) as usize;
        let opt = u16::from_le_bytes(hdr[nt + 20..nt + 22].try_into().unwrap()) as usize;
        let mut code_secs = Vec::new();
        let mut rdata = None;
        for i in 0..count {
            let s = nt + 24 + opt + i * 40;
            let vsize = u32::from_le_bytes(hdr[s + 8..s + 12].try_into().unwrap()) as usize;
            let va = u32::from_le_bytes(hdr[s + 12..s + 16].try_into().unwrap()) as usize;
            let flags = u32::from_le_bytes(hdr[s + 36..s + 40].try_into().unwrap());
            if flags & 0x2000_0000 != 0 {
                code_secs.push((base + va, vsize));
            } else if &hdr[s..s + 6] == b".rdata" && rdata.is_none() {
                rdata = Some((base + va, vsize));
            }
        }

        let mut candidates: Vec<(usize, &str)> = Vec::new();
        let rdata = rdata.and_then(|(a, n)| Some((a, g.read_region(a, n)?)));
        // The name the game's own code uses for this singleton.
        let names: Vec<usize> = match &rdata {
            Some((a, bytes)) => {
                let wide: Vec<u8> = "CSEventFlagMan".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
                let mut v: Vec<usize> = find_bytes(bytes, b"CSEventFlagMan").into_iter().map(|o| a + o).collect();
                v.extend(find_bytes(bytes, &wide).into_iter().map(|o| a + o));
                v
            }
            None => vec![],
        };

        for (start, len) in code_secs {
            let Some(code) = g.read_region(start, len) else { continue };
            // 1. Code that references the "CSEventFlagMan" name (its null-singleton error path)
            //    loads the singleton's static just before: `mov reg, [rip+static]`.
            if !names.is_empty() {
                for i in 0..code.len().saturating_sub(7) {
                    let is_lea = (code[i] == 0x48 || code[i] == 0x4C) && code[i + 1] == 0x8D && code[i + 2] & 0xC7 == 0x05;
                    if !is_lea {
                        continue;
                    }
                    let t = rip_target(start, &code, i);
                    if !names.iter().any(|&n| n >= t && n - t < 64) {
                        continue;
                    }
                    for back in 1..0x60.min(i) {
                        let p = i - back;
                        if code[p] == 0x48 && code[p + 1] == 0x8B && code[p + 2] & 0xC7 == 0x05 {
                            candidates.push((rip_target(start, &code, p), "name reference"));
                            break;
                        }
                    }
                }
            }
            // 2. Known byte patterns (older method, kept as a fallback).
            for p in PATTERNS {
                for hit in find_all(&code, &parse_pattern(p)) {
                    candidates.push((rip_target(start, &code, hit), "pattern"));
                }
            }
        }

        candidates.dedup();
        let mut report = String::new();
        let mut best: Option<usize> = None;
        for (c, how) in &candidates {
            g.flag_man = *c;
            let vmf = g.ptr(*c);
            let divisor = vmf.and_then(|v| g.u32(v + 0x1C));
            let ok = g.looks_valid();
            let block = vmf.and_then(|v| g.u32(v + 0x20));
            report += &format!("{c:#x} via {how}: object {:?} divisor {:?} block {:?} valid {ok}\n", vmf.map(|v| format!("{v:#x}")), divisor, block);
            if ok && best.is_none() {
                best = Some(*c);
            }
        }
        if let Some(b) = best {
            g.flag_man = b;
            *FOUND.lock().unwrap() = Some((pid, b));
            return Ok(g);
        }
        // While the game starts up or shuts down the manager doesn't exist yet: its static holds
        // null. That's "not yet", not "this game version is different".
        if candidates.iter().any(|(c, _)| g.read_bytes(*c, 8).is_some_and(|b| b.iter().all(|&x| x == 0))) {
            return Err(NOT_READY.into());
        }
        let _ = std::fs::write(crate::core::paths().data_dir.join("graces-debug.txt"), format!("{} candidates\n{report}", candidates.len()));
        Err("Couldn't find graces in this game version. Use Cheat Engine for now.".into())
    }

    /// Reads a whole section in 1 MB chunks (unreadable chunks come back as zeros).
    fn read_region(&self, start: usize, len: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        const CHUNK: usize = 1 << 20;
        let mut off = 0;
        let mut any = false;
        while off < len {
            let n = CHUNK.min(len - off);
            match self.read_bytes(start + off, n) {
                Some(b) => {
                    any = true;
                    out.extend(b)
                }
                None => out.extend(std::iter::repeat_n(0, n)),
            }
            off += n;
        }
        any.then_some(out)
    }

    /// Strict check before anything is written: flags come in blocks of 1000 (125 bytes),
    /// and the block tree must exist.
    fn looks_valid(&self) -> bool {
        let Some(vmf) = self.ptr(self.flag_man) else { return false };
        self.u32(vmf + 0x1C) == Some(1000) && self.u32(vmf + 0x20) == Some(125) && self.ptr(vmf + 0x38).is_some()
    }

    /// Address and bit mask of a flag, if its block is loaded.
    fn flag(&self, id: u32, cache: &mut HashMap<u32, Option<usize>>) -> Option<(usize, u8)> {
        let vmf = self.ptr(self.flag_man)?;
        let divisor = self.u32(vmf + 0x1C)?;
        let (block, index) = (id / divisor, id % divisor);
        let block_addr = *cache.entry(block).or_insert_with(|| self.block_address(vmf, block));
        let addr = block_addr? + (index / 8) as usize;
        Some((addr, 1u8 << (7 - index % 8)))
    }

    fn block_address(&self, vmf: usize, block: u32) -> Option<usize> {
        let tree = self.ptr(vmf + 0x38)?;
        let mut node = self.ptr(tree + 0x8)?;
        for _ in 0..64 {
            if self.u8(node + 0x19)? != 0 {
                return None; // reached a nil leaf
            }
            let nb = self.u32(node + 0x20)?;
            if nb == block {
                let mode = self.u32(node + 0x28)?;
                let raw = self.read_bytes(node + 0x30, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize)?;
                return match mode {
                    1 => {
                        let size = self.u32(vmf + 0x20)? as usize;
                        let blocks = self.ptr(vmf + 0x28)?;
                        Some(blocks + (raw & 0xFFFF_FFFF) * size)
                    }
                    2 if raw != 0 => Some(raw),
                    _ => None,
                };
            }
            node = if nb < block { self.ptr(node + 0x10)? } else { self.ptr(node)? };
        }
        None
    }

    fn get(&self, id: u32, cache: &mut HashMap<u32, Option<usize>>) -> Option<bool> {
        let (a, bit) = self.flag(id, cache)?;
        Some(self.u8(a)? & bit != 0)
    }

    fn set(&self, id: u32, on: bool, cache: &mut HashMap<u32, Option<usize>>) -> bool {
        let Some((a, bit)) = self.flag(id, cache) else { return false };
        let Some(v) = self.u8(a) else { return false };
        let nv = if on { v | bit } else { v & !bit };
        nv == v || self.write_u8(a, nv)
    }

    /// Which graces are on right now (`None` while no character is readable). Automatic setting
    /// only writes once two of these agree, so a character that's still loading isn't touched.
    pub fn snapshot(&self) -> Option<Vec<bool>> {
        let mut cache = HashMap::new();
        ALL.iter().map(|&id| self.get(id, &mut cache)).collect()
    }

    /// Applies the race graces. Characters with more than `limit` graces are left alone; `None`
    /// applies regardless.
    pub fn apply(&self, limit: Option<usize>) -> Outcome {
        let mut cache = HashMap::new();
        let wanted: Vec<u32> = WANTED.iter().map(|w| w.0).collect();
        let states: Vec<Option<bool>> = wanted.iter().map(|&id| self.get(id, &mut cache)).collect();
        if states.iter().any(Option::is_none) {
            return Outcome::NotLoaded;
        }
        if states.iter().all(|s| *s == Some(true)) {
            return Outcome::AlreadySet;
        }
        let have = ALL.iter().filter(|&&id| self.get(id, &mut cache) == Some(true)).count();
        if limit.is_some_and(|l| have > l) {
            return Outcome::NotFresh(have);
        }
        wanted.iter().for_each(|&id| { self.set(id, true, &mut cache); });
        let off = ALL.iter().filter(|id| !wanted.contains(id)).filter(|&&id| self.get(id, &mut cache) == Some(true) && self.set(id, false, &mut cache)).count();
        if !wanted.iter().all(|&id| self.get(id, &mut cache) == Some(true)) {
            return Outcome::WriteFailed;
        }
        Outcome::Applied(off)
    }
}

/// One-shot apply for the button and hotkey. Without `force` it uses the same limit as automatic
/// setting, so one key press can't wipe a real character on the normal save; past the limit the
/// hub asks first. `force` skips the check.
pub fn apply_now(pid: u32, force: bool) -> Result<Outcome, String> {
    let limit = if crate::bingosave::active() { FRESH_LIMIT } else { NEW_LIMIT };
    Ok(Game::open(pid)?.apply(if force { None } else { Some(limit) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_matching() {
        let hay = [0x90, 0x48, 0x8B, 0x3D, 1, 2, 3, 4, 0x48, 0x85, 0xFF, 0x74, 9, 0x48, 0x8B, 0x49];
        assert_eq!(find_all(&hay, &parse_pattern(PATTERNS[0])), vec![1]);
    }

    /// Against a running bingo game, reads only (nothing is written):
    /// `BINGOHUB_GAME_PID=<pid> cargo test -- --ignored live_read --nocapture`
    #[test]
    #[ignore]
    fn live_read() {
        let pid: u32 = std::env::var("BINGOHUB_GAME_PID").unwrap().parse().unwrap();
        let g = Game::open(pid).unwrap();
        assert!(g.looks_valid());
        let mut cache = HashMap::new();
        let wanted: Vec<Option<bool>> = WANTED.iter().map(|w| g.get(w.0, &mut cache)).collect();
        let on = ALL.iter().filter(|&&id| g.get(id, &mut cache) == Some(true)).count();
        println!("flag manager {:#x}; wanted {wanted:?}; graces on {on}", g.flag_man);
    }

    #[test]
    fn wanted_are_known_graces() {
        assert!(WANTED.iter().all(|(id, _)| ALL.contains(id)));
    }
}
