//! Thin wrappers over the Win32 calls the hub needs.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::ProcessStatus::K32EnumProcesses;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

// ---------------------------------------------------------------- processes

/// Cheap process list: enumerating PIDs costs microseconds, so names are only looked up
/// for processes that weren't there on the previous call.
#[derive(Default)]
pub struct ProcCache {
    names: std::collections::HashMap<u32, String>,
}

impl ProcCache {
    pub fn refresh(&mut self) {
        let mut pids = vec![0u32; 4096];
        let mut bytes = 0u32;
        let ok = unsafe { K32EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } != 0;
        if !ok {
            return;
        }
        pids.truncate(bytes as usize / 4);
        let live: std::collections::HashSet<u32> = pids.iter().copied().collect();
        self.names.retain(|pid, _| live.contains(pid));
        for pid in pids {
            self.names.entry(pid).or_insert_with(|| image_name(pid));
        }
    }

    pub fn pid_of(&self, exe_lower: &str) -> Option<u32> {
        self.names.iter().find(|(_, n)| n.as_str() == exe_lower).map(|(p, _)| *p)
    }
}

/// Lowercase exe file name of a process, or "" if it can't be opened.
fn image_name(pid: u32) -> String {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n) != 0;
        CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..n as usize]);
        full.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

/// Some(true/false) if the module list could be read, None if access was denied.
pub fn has_module(pid: u32, module: &str) -> Option<bool> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut m: MODULEENTRY32W = std::mem::zeroed();
        m.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut found = false;
        if Module32FirstW(snap, &mut m) != 0 {
            loop {
                if from_wide(&m.szModule).eq_ignore_ascii_case(module) {
                    found = true;
                    break;
                }
                if Module32NextW(snap, &mut m) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        Some(found)
    }
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Seconds since the process started.
pub fn process_age(pid: u32) -> Option<f64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
        CloseHandle(h);
        if !ok {
            return None;
        }
        let mut now: FILETIME = std::mem::zeroed();
        GetSystemTimeAsFileTime(&mut now);
        Some(ft(now).saturating_sub(ft(c)) as f64 / 1e7)
    }
}

// ---------------------------------------------------------------- windows

struct Find {
    pid: u32,
    title: Option<Vec<u16>>,
    found: HWND,
}

unsafe extern "system" fn enum_cb(h: HWND, l: LPARAM) -> i32 {
    let f = &mut *(l as *mut Find);
    if IsWindowVisible(h) == 0 || !GetWindow(h, GW_OWNER).is_null() {
        return 1;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, &mut pid);
    if f.pid != 0 && pid != f.pid {
        return 1;
    }
    if let Some(t) = &f.title {
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), 256) as usize;
        if buf[..n] != t[..t.len() - 1] {
            return 1;
        }
    }
    f.found = h;
    0
}

fn find_window(pid: u32, title: Option<&str>) -> HWND {
    let mut f = Find { pid, title: title.map(wide), found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(enum_cb), &mut f as *mut Find as LPARAM) };
    f.found
}

fn bring_to_front(h: HWND) {
    unsafe {
        ShowWindow(h, if IsIconic(h) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(h);
    }
}

/// Brings the main window of a process to the front. False if it has no visible window.
pub fn focus_pid(pid: u32) -> bool {
    let h = find_window(pid, None);
    if h.is_null() {
        return false;
    }
    bring_to_front(h);
    true
}

pub fn focus_titled(title: &str) {
    let h = find_window(0, Some(title));
    if !h.is_null() {
        bring_to_front(h);
    }
}

/// Our own top-level window (as an integer so it can cross threads).
pub fn own_window(title: &str) -> isize {
    find_window(std::process::id(), Some(title)) as isize
}

/// Rounded corners, dark mode and the standard window shadow for our borderless window.
pub fn style_window(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let h = hwnd as HWND;
    unsafe {
        let round: i32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(h, 33, &round as *const i32 as *const c_void, 4);
        let dark: i32 = 1;
        DwmSetWindowAttribute(h, 20, &dark as *const i32 as *const c_void, 4);
        let m = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
        DwmExtendFrameIntoClientArea(h, &m);
    }
}

/// How far a maximized window sticks out past the monitor's work area, in physical pixels
/// (left, top, right, bottom). Windows does this to hide the resize frame.
pub fn maximized_overhang(hwnd: isize) -> [f32; 4] {
    if hwnd == 0 {
        return [0.0; 4];
    }
    unsafe {
        let h = hwnd as HWND;
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(h, &mut r);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return [0.0; 4];
        }
        let w = mi.rcWork;
        [(w.left - r.left).max(0) as f32, (w.top - r.top).max(0) as f32, (r.right - w.right).max(0) as f32, (r.bottom - w.bottom).max(0) as f32]
    }
}

pub fn single_instance(name: &str) -> bool {
    unsafe {
        let n = wide(name);
        let h = CreateMutexW(std::ptr::null(), 1, n.as_ptr());
        if h.is_null() {
            return false;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return false;
        }
        true // handle intentionally kept for the life of the process
    }
}

// ---------------------------------------------------------------- hotkey and sounds

pub const HOTKEY: &str = "Ctrl+F9";

/// Hotkey state shared with the UI.
pub mod hotkey {
    pub const OFF: u8 = 0;
    pub const ON: u8 = 1;
    /// Another program already has the key.
    pub const TAKEN: u8 = 2;
}

/// Calls `on_press` each time the global hotkey is pressed. The key is only registered while
/// `active()` is true, so other programs keep Ctrl+F9 the rest of the time. Never returns.
pub fn hotkey_loop(active: impl Fn() -> bool, state: &std::sync::atomic::AtomicU8, mut on_press: impl FnMut()) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, VK_F9};
    unsafe {
        // Thread timer: WM_TIMER lands in this thread's queue, so no window is needed.
        SetTimer(std::ptr::null_mut(), 0, 500, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            match msg.message {
                WM_HOTKEY => on_press(),
                WM_TIMER => {
                    let now = state.load(Ordering::Relaxed);
                    let want = active();
                    if want && now != hotkey::ON {
                        let ok = RegisterHotKey(std::ptr::null_mut(), 1, MOD_CONTROL | MOD_NOREPEAT, VK_F9 as u32) != 0;
                        state.store(if ok { hotkey::ON } else { hotkey::TAKEN }, Ordering::Relaxed);
                    } else if !want && now != hotkey::OFF {
                        if now == hotkey::ON {
                            UnregisterHotKey(std::ptr::null_mut(), 1);
                        }
                        state.store(hotkey::OFF, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Windows' notification sound, so results are heard while the game has the screen.
pub fn chime(ok: bool) {
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(if ok { MB_ICONASTERISK } else { MB_ICONHAND }) };
}

// ---------------------------------------------------------------- launching

pub fn launch(exe: &Path, args: &[&str], admin: bool, hidden: bool) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    if admin {
        let r = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                wide("runas").as_ptr(),
                wide(exe).as_ptr(),
                wide(args.join(" ")).as_ptr(),
                wide(dir).as_ptr(),
                SW_SHOWNORMAL,
            )
        } as isize;
        return if r > 32 { Ok(()) } else if r == 5 { Err(String::new()) } else { Err(format!("Couldn't start {}", name(exe))) };
    }
    let mut c = Command::new(exe);
    c.args(args).current_dir(dir);
    if hidden {
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.spawn().map(|_| ()).map_err(|e| format!("Couldn't start {}: {e}", name(exe)))
}

fn name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn shell_open(target: &str) {
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), wide(target).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

pub fn open_folder(p: &Path) {
    let _ = Command::new("explorer.exe").arg(p).spawn();
}

/// Moves a file to the Recycle Bin.
pub fn recycle(p: &Path) -> Result<(), String> {
    let mut from = wide(p);
    from.push(0); // double-null terminated
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE as _;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as _;
    let r = unsafe { SHFileOperationW(&mut op) };
    if r == 0 { Ok(()) } else { Err(format!("Couldn't delete {}", name(p))) }
}

// ---------------------------------------------------------------- registry / versions

pub fn steam_path() -> Option<PathBuf> {
    unsafe {
        let mut buf = [0u16; 520];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Valve\\Steam").as_ptr(),
            wide("SteamPath").as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        );
        if r != 0 {
            return None;
        }
        Some(PathBuf::from(from_wide(&buf).replace('/', "\\")))
    }
}

/// "1.2.3" style version from the file's version resource (trailing ".0" dropped).
pub fn file_version(p: &Path) -> Option<String> {
    unsafe {
        let w = wide(p);
        let mut dummy = 0u32;
        let len = GetFileVersionInfoSizeW(w.as_ptr(), &mut dummy);
        if len == 0 {
            return None;
        }
        let mut data = vec![0u8; len as usize];
        if GetFileVersionInfoW(w.as_ptr(), 0, len, data.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut n = 0u32;
        if VerQueryValueW(data.as_ptr() as *const c_void, wide("\\").as_ptr(), &mut ptr, &mut n) == 0 || ptr.is_null() {
            return None;
        }
        let fi = &*(ptr as *const VS_FIXEDFILEINFO);
        let v = [fi.dwFileVersionMS >> 16, fi.dwFileVersionMS & 0xffff, fi.dwFileVersionLS >> 16, fi.dwFileVersionLS & 0xffff];
        let s = if v[3] == 0 { format!("{}.{}.{}", v[0], v[1], v[2]) } else { format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]) };
        Some(s)
    }
}

// ---------------------------------------------------------------- file dialogs (call off the UI thread)

/// filter: [("Goal lists", "*.json")]
pub fn open_files(owner: isize, title: &str, filter: &[(&str, &str)], multi: bool) -> Vec<PathBuf> {
    let mut buf = vec![0u16; 32 * 1024];
    let filt: Vec<u16> = filter.iter().flat_map(|(a, b)| wide(a).into_iter().chain(wide(b))).chain(Some(0)).collect();
    let t = wide(title);
    let mut o: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    o.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    o.hwndOwner = owner as HWND;
    o.lpstrFilter = filt.as_ptr();
    o.lpstrFile = buf.as_mut_ptr();
    o.nMaxFile = buf.len() as u32;
    o.lpstrTitle = t.as_ptr();
    o.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | if multi { OFN_ALLOWMULTISELECT } else { 0 };
    if unsafe { GetOpenFileNameW(&mut o) } == 0 {
        return vec![];
    }
    let parts: Vec<String> = buf.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect();
    match parts.len() {
        0 => vec![],
        1 => vec![PathBuf::from(&parts[0])],
        _ => parts[1..].iter().map(|f| Path::new(&parts[0]).join(f)).collect(),
    }
}

pub fn save_file(owner: isize, title: &str, filter: &[(&str, &str)], default_name: &str, ext: &str) -> Option<PathBuf> {
    let mut buf = vec![0u16; 1024];
    for (i, c) in default_name.encode_utf16().enumerate().take(1000) {
        buf[i] = c;
    }
    let filt: Vec<u16> = filter.iter().flat_map(|(a, b)| wide(a).into_iter().chain(wide(b))).chain(Some(0)).collect();
    let t = wide(title);
    let e = wide(ext);
    let mut o: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    o.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    o.hwndOwner = owner as HWND;
    o.lpstrFilter = filt.as_ptr();
    o.lpstrFile = buf.as_mut_ptr();
    o.nMaxFile = buf.len() as u32;
    o.lpstrTitle = t.as_ptr();
    o.lpstrDefExt = e.as_ptr();
    o.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR;
    if unsafe { GetSaveFileNameW(&mut o) } == 0 {
        return None;
    }
    Some(PathBuf::from(from_wide(&buf)))
}
