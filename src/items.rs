//! What the Classes page knows about an item: its type, how it upgrades and its icon.
//! Built from the game files by tools/item-icons (assets/items.tsv, assets/icons.bin).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Upgrade {
    None,
    Smithing,
    Somber,
}

#[derive(Clone, Copy, Debug)]
pub struct Info {
    /// "Katana", "Sorcery", "Arrow"...
    pub kind: &'static str,
    pub upgrade: Upgrade,
    pub icon: u32,
}

static TABLE: &str = include_str!("../assets/items.tsv");
/// Empty when the build had no assets/icons.bin (it's game art, so it isn't in git).
static ICONS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icons.bin"));

fn table() -> &'static HashMap<&'static str, Info> {
    static T: OnceLock<HashMap<&'static str, Info>> = OnceLock::new();
    T.get_or_init(|| {
        let mut m = HashMap::new();
        for line in TABLE.lines() {
            let mut p = line.split('\t');
            let (Some(name), Some(kind), Some(up), Some(icon)) = (p.next(), p.next(), p.next(), p.next()) else { continue };
            let upgrade = match up {
                "smithing" => Upgrade::Smithing,
                "somber" => Upgrade::Somber,
                _ => Upgrade::None,
            };
            m.entry(name).or_insert(Info { kind, upgrade, icon: icon.trim().parse().unwrap_or(0) });
        }
        m
    })
}

/// Ash of War affinities, as the game puts them in front of a weapon's name.
pub const AFFINITIES: [&str; 12] = ["Heavy", "Keen", "Quality", "Fire", "Flame Art", "Lightning", "Sacred", "Magic", "Cold", "Poison", "Blood", "Occult"];

/// "Quality Longsword" -> ("Quality", "Longsword"), if the rest is a known weapon.
pub fn split_affinity(name: &str) -> Option<(&'static str, &str)> {
    AFFINITIES.iter().find_map(|a| name.strip_prefix(a).and_then(|r| r.strip_prefix(' ')).filter(|r| table().contains_key(r)).map(|r| (*a, r)))
}

/// The spoiler names items as the game does; ammo and a few weapons carry a " +0" suffix, and an
/// infused weapon ("Heavy Longsword") looks and upgrades like its plain one.
pub fn info(name: &str) -> Option<Info> {
    let t = table();
    let name = name.trim_end_matches(" +0");
    t.get(name).or_else(|| split_affinity(name).and_then(|(_, base)| t.get(base))).copied()
}

static PICKS: &str = include_str!("../assets/bingo-weapons.txt");

fn picks() -> impl Iterator<Item = &'static str> {
    PICKS.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'))
}

/// Weapons the group rates for races (assets/bingo-weapons.txt).
pub fn bingo_pick(name: &str) -> bool {
    let name = name.trim_end_matches(" +0");
    let name = split_affinity(name).map_or(name, |(_, base)| base);
    picks().any(|p| p == name)
}

fn icon_png(id: u32) -> Option<&'static [u8]> {
    static INDEX: OnceLock<HashMap<u32, (usize, usize)>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let mut m = HashMap::new();
        let rd = |o: usize| ICONS.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        if ICONS.get(..4) != Some(b"BHIC") {
            return m;
        }
        let n = rd(4).unwrap_or(0);
        let mut off = 8 + 8 * n;
        for i in 0..n {
            let (Some(id), Some(len)) = (rd(8 + 8 * i), rd(12 + 8 * i)) else { break };
            m.insert(id as u32, (off, len));
            off += len;
        }
        m
    });
    let &(o, l) = index.get(&id)?;
    ICONS.get(o..o + l)
}

fn decode(png_bytes: &[u8]) -> Option<ColorImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let f = r.next_frame(&mut buf).ok()?;
    let px = &buf[..f.buffer_size()];
    let rgba: Vec<u8> = match f.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &rgba))
}

/// The item's icon as a texture, decoded the first time it's asked for.
pub fn texture(ctx: &egui::Context, icon: u32) -> Option<TextureHandle> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Option<TextureHandle>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache
        .entry(icon)
        .or_insert_with(|| decode(icon_png(icon)?).map(|img| ctx.load_texture(format!("item-{icon}"), img, TextureOptions::LINEAR)))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_weapons_and_spells() {
        let moonveil = info("Moonveil").unwrap();
        assert_eq!((moonveil.kind, moonveil.upgrade), ("Katana", Upgrade::Somber));
        assert_eq!(info("Uchigatana").unwrap().upgrade, Upgrade::Smithing);
        assert_eq!(info("Glintstone Arc").unwrap().kind, "Sorcery");
        assert_eq!(info("Dueling Shield +0").unwrap().kind, "Thrusting Shield");
        let q = info("Quality Longsword").unwrap();
        assert_eq!((q.kind, q.upgrade, q.icon), ("Straight Sword", Upgrade::Smithing, info("Longsword").unwrap().icon));
        assert!(info("Quality Nonsense").is_none() && bingo_pick("Keen Great Katana"));
    }

    #[test]
    fn bingo_picks_are_real_weapons() {
        for p in picks() {
            let i = info(p).unwrap_or_else(|| panic!("{p} isn't in items.tsv"));
            // Classes never roll spells, catalysts, greatbows or Remembrance shop weapons.
            assert!(!matches!(i.kind, "Sorcery" | "Incantation" | "Glintstone Staff" | "Sacred Seal" | "Greatbow"), "{p} can't be a class weapon");
            const REMEMBRANCE: &[&str] = &[
                "Sword Lance", "Reverse-Bladed Sword", "Axe of Godrick", "Grafted Dragon", "Starscourge Greatsword", "Lion Greatbow",
                "Morgott's Cursed Sword", "Blasphemous Blade", "Hand of Malenia", "Mohgwyn's Sacred Spear", "Maliketh's Black Blade",
                "Axe of Godfrey", "Dragon King's Cragblade", "Carian Regal Scepter", "Giant's Red Braid", "Winged Greathorn",
                "Marika's Hammer", "Sacred Relic Sword", "Bastard's Stars", "Rellana's Twin Blades", "Putrescence Cleaver",
                "Poleblade of the Bud", "Greatsword of Damnation", "Shadow Sunflower Blossom", "Spear of the Impaler",
                "Staff of the Great Beyond", "Gazing Finger", "Greatsword of Radahn (Lord)", "Greatsword of Radahn (Light)",
            ];
            assert!(!REMEMBRANCE.contains(&p), "{p} is a Remembrance weapon; classes never get it");
        }
        assert!(bingo_pick("Wing of Astel") && !bingo_pick("Club"));
    }

    #[test]
    fn icons_decode_when_bundled() {
        if ICONS.is_empty() {
            return;
        }
        let img = decode(icon_png(info("Moonveil").unwrap().icon).unwrap()).unwrap();
        assert_eq!(img.size, [96, 96]);
    }
}
