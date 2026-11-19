//! Patch-day check: does the Randomizer know every weapon in the game?
//!
//! S2 Style names each starting class's weapons from a table built into its code
//! (`Project.Settings.Equipment.EquipmentNameList`) rather than the game's text. When a game update
//! adds weapons, any seed that gives one to a class crashes ("This seed doesn't work"). This compares
//! that table with the weapon names in the game text the Randomizer itself reads (its `Cache\msg`
//! copy), so a gap shows up in Setup before race night instead of as crashing seeds.
//!
//! Everything here only reads files. The fix is done once by the owner with
//! `tools/randomizer-patch` and reaches everyone as a tool update, so all hubs run the same build.

use std::collections::{BTreeMap, HashSet};
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Marks the bundle header pointer in a .NET single-file host.
const BUNDLE_SIG: [u8; 32] = [
    0x8b, 0x12, 0x02, 0xb9, 0x6a, 0x61, 0x20, 0x38, 0x72, 0x7b, 0x93, 0x02, 0x14, 0xd7, 0xa0, 0x32, 0x13, 0xf5, 0xb9, 0xe6, 0xef, 0xae, 0x33, 0x18,
    0xee, 0x3b, 0x2d, 0xce, 0x24, 0xb3, 0x6a, 0xae,
];

fn bytes<const N: usize>(b: &[u8], o: usize) -> Option<[u8; N]> {
    b.get(o..o.checked_add(N)?)?.try_into().ok()
}
fn u32le(b: &[u8], o: usize) -> Option<u32> {
    bytes(b, o).map(u32::from_le_bytes)
}
fn i32le(b: &[u8], o: usize) -> Option<i32> {
    bytes(b, o).map(i32::from_le_bytes)
}
fn i64le(b: &[u8], o: usize) -> Option<i64> {
    bytes(b, o).map(i64::from_le_bytes)
}
fn u32be(b: &[u8], o: usize) -> Option<u32> {
    bytes(b, o).map(u32::from_be_bytes)
}

fn read_at(f: &mut File, off: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut b = vec![0; len];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// A .NET bundle string: 7-bit encoded length, then UTF-8.
fn bundle_str(b: &[u8], p: &mut usize) -> Option<String> {
    let (mut n, mut shift) = (0usize, 0);
    loop {
        let x = *b.get(*p)?;
        *p += 1;
        n |= ((x & 0x7f) as usize) << shift;
        shift += 7;
        if x < 0x80 {
            break;
        }
    }
    let s = String::from_utf8(b.get(*p..*p + n)?.to_vec()).ok()?;
    *p += n;
    Some(s)
}

/// One file from a .NET single-file bundle (uncompressed entries only).
fn bundle_file(exe: &Path, name: &str) -> Option<Vec<u8>> {
    let mut f = File::open(exe).ok()?;
    let len = f.metadata().ok()?.len();
    // The pointer to the manifest sits right before the signature, in the host at the start.
    let head = read_at(&mut f, 0, len.min(8 << 20) as usize)?;
    let at = head.windows(32).position(|w| w == BUNDLE_SIG)?;
    let hdr = u64::try_from(i64le(&head, at.checked_sub(8)?)?).ok()?;
    let m = read_at(&mut f, hdr, len.checked_sub(hdr)?.min(16 << 20) as usize)?;
    let major = u32le(&m, 0)?;
    let count = i32le(&m, 8)?;
    let mut p = 12;
    bundle_str(&m, &mut p)?; // bundle id
    if major >= 2 {
        p += 40; // deps.json and runtimeconfig.json locations, flags
    }
    for _ in 0..count {
        let off = u64::try_from(i64le(&m, p)?).ok()?;
        let size = usize::try_from(i64le(&m, p + 8)?).ok()?;
        p += 16;
        let compressed = if major >= 6 {
            p += 8;
            i64le(&m, p - 8)? != 0
        } else {
            false
        };
        p += 1; // type
        if bundle_str(&m, &mut p)? == name {
            return if compressed { None } else { read_at(&mut f, off, size) };
        }
    }
    None
}

/// Weapon IDs in the Randomizer's built-in name table. The compiler fills it with one long run of
/// `dup; ldc.i4 <id>; ldstr <name>; callvirt Add` (16 bytes each), so the longest such run is it.
fn table_ids(dll: &[u8]) -> Option<HashSet<u32>> {
    let at = |i: usize| dll.get(i..i + 16).is_some_and(|w| w[0] == 0x25 && w[1] == 0x20 && w[6] == 0x72 && w[10] == 0x70 && w[11] == 0x6f);
    let mut best: (usize, usize) = (0, 0); // (start, entries)
    let mut i = 0;
    while i + 16 <= dll.len() {
        if at(i) {
            let call = &dll[i + 12..i + 16];
            let mut n = 1;
            while at(i + n * 16) && &dll[i + n * 16 + 12..i + n * 16 + 16] == call {
                n += 1;
            }
            if n > best.1 {
                best = (i, n);
            }
            i += n * 16;
        } else {
            i += 1;
        }
    }
    (best.1 >= 100).then(|| (0..best.1).filter_map(|k| u32le(dll, best.0 + k * 16 + 2)).collect())
}

type OodleDecompress =
    unsafe extern "system" fn(*const u8, isize, *mut u8, isize, i32, i32, i32, *mut c_void, isize, *mut c_void, *mut c_void, *mut c_void, isize, i32) -> isize;

/// The game's Oodle decompressor, loaded once from the game folder.
fn oodle(dll: &Path) -> Option<OodleDecompress> {
    static F: OnceLock<Option<usize>> = OnceLock::new();
    let f = F.get_or_init(|| {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let h = unsafe { LoadLibraryW(crate::sys::wide(dll).as_ptr()) };
        if h.is_null() {
            return None;
        }
        unsafe { GetProcAddress(h, c"OodleLZ_Decompress".as_ptr().cast()) }.map(|p| p as usize)
    });
    f.map(|p| unsafe { std::mem::transmute::<usize, OodleDecompress>(p) })
}

/// A DCX file compressed with Oodle Kraken, as the game ships its text.
fn dcx(data: &[u8], oodle_dll: &Path) -> Option<Vec<u8>> {
    if data.get(0..4)? != b"DCX\0" || data.get(0x28..0x2C)? != b"KRAK" {
        return None;
    }
    let raw_len = u32be(data, 0x1C)? as usize;
    let comp_len = u32be(data, 0x20)? as usize;
    let comp = data.get(0x4C..0x4C + comp_len)?;
    let mut out = vec![0u8; raw_len];
    let n = unsafe {
        oodle(oodle_dll)?(
            comp.as_ptr(),
            comp.len() as isize,
            out.as_mut_ptr(),
            raw_len as isize,
            1,
            0,
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            3,
        )
    };
    (n == raw_len as isize).then_some(out)
}

fn utf16z(b: &[u8], mut p: usize) -> Option<String> {
    let mut u = Vec::new();
    loop {
        let c = u16::from_le_bytes(bytes(b, p)?);
        if c == 0 {
            break;
        }
        u.push(c);
        p += 2;
    }
    Some(String::from_utf16_lossy(&u))
}

/// Files in a BND4 archive as (name, bytes), for the layout the game's .msgbnd files use.
fn bnd4(b: &[u8]) -> Option<Vec<(String, &[u8])>> {
    if b.get(0..4)? != b"BND4" || i64le(b, 0x20)? != 0x24 || *b.get(0x30)? != 1 {
        return None; // not the 36-byte, UTF-16-named entry layout
    }
    let mut v = Vec::new();
    for k in 0..usize::try_from(i32le(b, 0x0C)?).ok()? {
        let e = 0x40 + k * 0x24;
        let size = usize::try_from(i64le(b, e + 8)?).ok()?;
        let off = u32le(b, e + 0x18)? as usize;
        let name = utf16z(b, u32le(b, e + 0x20)? as usize)?;
        v.push((name, b.get(off..off.checked_add(size)?)?));
    }
    Some(v)
}

/// Entries of a version-2 (64-bit) FMG text file.
fn fmg(b: &[u8]) -> Option<Vec<(u32, String)>> {
    if b.get(0..4)? != [0, 0, 2, 0] {
        return None;
    }
    let groups = usize::try_from(i32le(b, 0x0C)?).ok()?;
    let offsets = usize::try_from(i64le(b, 0x18)?).ok()?;
    let mut v = Vec::new();
    for g in 0..groups {
        let e = 0x28 + g * 16;
        let (index, first, last) = (i32le(b, e)?, i32le(b, e + 4)?, i32le(b, e + 8)?);
        for id in first..=last {
            let slot = usize::try_from(index + (id - first)).ok()?;
            let at = usize::try_from(i64le(b, offsets + slot * 8)?).ok()?;
            if at != 0 {
                v.push((id as u32, utf16z(b, at)?));
            }
        }
    }
    Some(v)
}

/// Named weapons (base IDs only) in the game text the Randomizer uses.
fn game_weapons(msgbnd: &Path, oodle_dll: &Path) -> Option<BTreeMap<u32, String>> {
    let raw = dcx(&std::fs::read(msgbnd).ok()?, oodle_dll)?;
    let mut names = BTreeMap::new();
    let files = bnd4(&raw)?;
    // Base game, then the DLC files, the same order the game layers them.
    let mut weapon_files: Vec<_> = files.iter().filter(|(n, _)| n.rsplit('\\').next().is_some_and(|f| f.starts_with("WeaponName") && f.ends_with(".fmg"))).collect();
    weapon_files.sort_by_key(|(n, _)| n.len());
    if weapon_files.is_empty() {
        return None;
    }
    for (_, data) in weapon_files {
        for (id, text) in fmg(data)? {
            if id % 10000 == 0 && !text.trim().is_empty() && !text.to_lowercase().contains("[error]") {
                names.insert(id, text);
            }
        }
    }
    Some(names)
}

/// Weapons the game has that the Randomizer can't name, as (id, name).
pub fn missing(rando_exe: &Path, msgbnd: &Path, oodle_dll: &Path) -> Option<Vec<(u32, String)>> {
    let known = table_ids(&bundle_file(rando_exe, "Randomizer.dll")?)?;
    let game = game_weapons(msgbnd, oodle_dll)?;
    Some(game.into_iter().filter(|(id, _)| !known.contains(id)).collect())
}

/// `missing`, remembered until the Randomizer or its text cache changes. `None` = couldn't check
/// (not randomized yet, or a Randomizer this check doesn't understand).
pub fn missing_cached(rando_exe: &Path, game_dir: &Path) -> Option<Vec<(u32, String)>> {
    type Key = (PathBuf, Option<SystemTime>, Option<SystemTime>);
    static LAST: Mutex<Option<(Key, Option<Vec<(u32, String)>>)>> = Mutex::new(None);
    let msgbnd = msgbnd_path();
    let key: Key = (rando_exe.to_path_buf(), crate::core::mtime(rando_exe), crate::core::mtime(&msgbnd));
    let mut last = LAST.lock().unwrap();
    if let Some((_, v)) = last.as_ref().filter(|(k, _)| *k == key) {
        return v.clone();
    }
    let v = missing(rando_exe, &msgbnd, &game_dir.join("oo2core_6_win64.dll"));
    *last = Some((key, v.clone()));
    v
}

/// The Randomizer's copy of the game's item text (made on its first run).
pub fn msgbnd_path() -> PathBuf {
    crate::core::paths().rando_dir.join("Cache").join("msg").join("engus").join("item_dlc02.msgbnd.dcx")
}

/// Writes the missing names as `id<TAB>name` lines, the input of tools/randomizer-patch.
pub fn write_list(list: &[(u32, String)]) {
    let text: String = list.iter().map(|(id, n)| format!("{id}\t{n}\n")).collect();
    let _ = crate::core::write_atomic(&crate::core::paths().data_dir.join("randomizer-missing-names.tsv"), text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the real install: `cargo test -- --ignored randocheck --nocapture`.
    #[test]
    #[ignore]
    fn randocheck_real_install() {
        let game = PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\ELDEN RING\Game");
        let msg = PathBuf::from(r"C:\Games\Elden Ring Bingo\Bingo Randomizer\Cache\msg\engus\item_dlc02.msgbnd.dcx");
        for exe in [r"C:\Games\Elden Ring Bingo\Data\Randomizer backup\S2_Style_1.0.exe", r"C:\Games\Elden Ring Bingo\Bingo Randomizer\S2_Style_1.0.exe"] {
            let ids = table_ids(&bundle_file(Path::new(exe), "Randomizer.dll").unwrap()).unwrap();
            let m = missing(Path::new(exe), &msg, &game.join("oo2core_6_win64.dll")).unwrap();
            println!("{exe}: table {} names, {} missing", ids.len(), m.len());
            for (id, n) in &m {
                println!("TSV	{id}	{n}");
            }
        }
    }
}
