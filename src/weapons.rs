//! Weapon and spell numbers for the Classes page: skill, attack rating at a class's starting stats
//! (at +0 and upgraded), status buildup, scaling and requirements. Built from the game files by tools/item-icons
//! (assets/weapons.tsv). Classes always start with plain +0 weapons and their own skills, so the
//! vanilla numbers are the ones in the seed.
//!
//! Attack rating follows the game: each damage type is base × (1 + Σ scaling × curve(stat)), or
//! base × 0.6 when a stat that type scales with is under its requirement.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Strength, Dexterity, Intelligence, Faith, Arcane.
pub const ATTRS: [&str; 5] = ["Str", "Dex", "Int", "Fai", "Arc"];
/// Physical, Magic, Fire, Lightning, Holy.
pub const DAMAGE: [&str; 5] = ["Physical", "Magic", "Fire", "Lightning", "Holy"];
/// Poison, Scarlet Rot, Bleed, Frost, Sleep, Madness, Death Blight.
pub const STATUS: [&str; 7] = ["Poison", "Scarlet Rot", "Bleed", "Frost", "Sleep", "Madness", "Death Blight"];

pub struct Weapon {
    pub skill: &'static str,
    /// As the game shows it: "20", or "10/15" for a stance's light/heavy follow-ups.
    pub skill_fp: &'static str,
    /// "Slash / Pierce"
    pub damage_type: &'static str,
    base: [f64; 5],
    /// Scaling per attribute, ×100 (Uchigatana's Dex is 55).
    scaling: [f64; 5],
    pub req: [u32; 5],
    /// CalcCorrectGraph per damage type, then for poison, bleed, sleep, madness.
    graph: [u32; 9],
    element: u32,
    status: [f64; 7],
    paired: bool,
    pub sorcery: bool,
    pub incantation: bool,
    /// Highest upgrade: 25 (smithing), 10 (somber) or 0.
    pub max_level: u32,
    /// ReinforceParamWeapon type: rows `reinforce + level` scale the raw values.
    reinforce: u32,
    raw_base: [f64; 5],
    raw_scaling: [f64; 5],
    /// SpEffects that carry status buildup; the upgrade level offsets them.
    behavior: [u32; 3],
}

pub struct Spell {
    pub fp: u32,
    pub slots: u32,
    /// Intelligence, Faith, Arcane.
    pub req: [u32; 3],
}

struct Data {
    weapons: HashMap<&'static str, Weapon>,
    graphs: HashMap<u32, [f64; 15]>,
    /// Per damage type and attribute: None = no scaling, Some(None) = the weapon's own scaling,
    /// Some(Some(x)) = an override rate.
    elements: HashMap<u32, [Option<Option<f64>>; 25]>,
    spells: HashMap<&'static str, Spell>,
    /// (type, level) -> attack rates ×5, scaling rates ×5, SpEffect offsets ×3.
    reinforce: HashMap<(u32, u32), [f64; 13]>,
    /// SpEffect -> status buildup ×7.
    effects: HashMap<u32, [f64; 7]>,
}

static TABLE: &str = include_str!("../assets/weapons.tsv");

fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| {
        let mut d = Data { weapons: HashMap::new(), graphs: HashMap::new(), elements: HashMap::new(), spells: HashMap::new(), reinforce: HashMap::new(), effects: HashMap::new() };
        for line in TABLE.lines() {
            let f: Vec<&'static str> = line.split('\t').collect();
            let num = |i: usize| f.get(i).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
            let arr = |from: usize| -> [f64; 5] { std::array::from_fn(|k| num(from + k)) };
            match f[0] {
                "W" if f.len() >= 55 => {
                    let w = Weapon {
                        skill: f[2],
                        skill_fp: f[3],
                        damage_type: f[4],
                        base: arr(5),
                        scaling: arr(10),
                        req: std::array::from_fn(|k| num(15 + k) as u32),
                        graph: std::array::from_fn(|k| num(20 + k) as u32),
                        element: num(29) as u32,
                        status: std::array::from_fn(|k| num(30 + k)),
                        paired: f[37] == "1",
                        sorcery: f[38] == "1",
                        incantation: f[39] == "1",
                        reinforce: num(40) as u32,
                        max_level: num(41) as u32,
                        raw_base: arr(42),
                        raw_scaling: arr(47),
                        behavior: std::array::from_fn(|k| num(52 + k) as u32),
                    };
                    d.weapons.entry(f[1]).or_insert(w);
                }
                "G" if f.len() >= 17 => {
                    d.graphs.insert(num(1) as u32, std::array::from_fn(|k| num(2 + k)));
                }
                "E" if f.len() >= 27 => {
                    let e = std::array::from_fn(|k| match num(2 + k) {
                        v if v == -2.0 => None,
                        v if v == -1.0 => Some(None),
                        v => Some(Some(v)),
                    });
                    d.elements.insert(num(1) as u32, e);
                }
                "R" if f.len() >= 16 => {
                    d.reinforce.insert((num(1) as u32, num(2) as u32), std::array::from_fn(|k| num(3 + k)));
                }
                "X" if f.len() >= 9 => {
                    d.effects.insert(num(1) as u32, std::array::from_fn(|k| num(2 + k)));
                }
                "S" if f.len() >= 7 => {
                    let s = Spell { fp: num(2) as u32, slots: num(3) as u32, req: [num(4) as u32, num(5) as u32, num(6) as u32] };
                    d.spells.entry(f[1]).or_insert(s);
                }
                _ => {}
            }
        }
        d
    })
}

/// The spoiler writes some weapons with a " +0" suffix.
pub fn weapon(name: &str) -> Option<&'static Weapon> {
    let w = &data().weapons;
    w.get(name).or_else(|| w.get(name.trim_end_matches(" +0")))
}

/// The game version the Classes data was built from (the "V" line of weapons.tsv).
pub fn built_for() -> Option<&'static str> {
    TABLE.lines().find_map(|l| l.strip_prefix("V	")).map(str::trim)
}

pub fn spell(name: &str) -> Option<&'static Spell> {
    data().spells.get(name)
}

/// The game's stat curve: how much of a weapon's scaling a stat value unlocks (0.0 to ~1.0).
fn curve(id: u32, stat: u32) -> f64 {
    let Some(g) = data().graphs.get(&id).or_else(|| data().graphs.get(&0)) else { return 0.0 };
    let stage = |i: usize| (g[i * 3], g[i * 3 + 1] / 100.0, g[i * 3 + 2]);
    let v = stat as f64;
    let mut i = 1;
    while i < 4 && v > stage(i).0 {
        i += 1;
    }
    let ((lo, lo_grow, adj), (hi, hi_grow, _)) = (stage(i - 1), stage(i));
    let mut r = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    if adj > 0.0 {
        r = r.powf(adj);
    } else if adj < 0.0 {
        r = 1.0 - (1.0 - r).powf(-adj);
    }
    lo_grow + (hi_grow - lo_grow) * r
}

pub struct Attack {
    /// What the game's status screen shows: the sum of the damage types, each rounded down.
    pub total: u32,
    pub damage: [u32; 5],
    pub status: [u32; 7],
    /// Shown two-handed: one hand doesn't meet the Strength requirement but two do.
    pub two_handed: bool,
    /// A damage type is weakened because a stat it scales with is too low.
    pub weakened: bool,
    /// Sorcery or incantation scaling, for catalysts.
    pub spell_scaling: Option<u32>,
}

/// Attack at +0 with these stats (Str, Dex, Int, Fai, Arc). `two_hand_only` is for bows.
pub fn attack(w: &Weapon, stats: [u32; 5], two_hand_only: bool) -> Attack {
    attack_at(w, stats, two_hand_only, 0)
}

/// Base attack, scaling and status buildup at an upgrade level (the +0 values when it's unknown).
fn upgraded(w: &Weapon, level: u32) -> ([f64; 5], [f64; 5], [f64; 7]) {
    let lv = level.min(w.max_level);
    let Some(r) = data().reinforce.get(&(w.reinforce, lv)).filter(|_| lv > 0) else { return (w.base, w.scaling, w.status) };
    let mut status = [0.0; 7];
    for (k, &b) in w.behavior.iter().enumerate() {
        if b == 0 {
            continue;
        }
        if let Some(e) = data().effects.get(&(b + r[10 + k] as u32)) {
            for s in 0..7 {
                status[s] += e[s];
            }
        }
    }
    (std::array::from_fn(|t| w.raw_base[t] * r[t]), std::array::from_fn(|a| w.raw_scaling[a] * r[5 + a]), status)
}

/// Attack at an upgrade level (capped at the weapon's highest).
pub fn attack_at(w: &Weapon, stats: [u32; 5], two_hand_only: bool, level: u32) -> Attack {
    let (base, scaling, status) = upgraded(w, level);
    let two = two_hand_only || (!w.paired && stats[0] < w.req[0] && stats[0] * 3 / 2 >= w.req[0]);
    let mut eff = stats;
    if two && !w.paired {
        eff[0] = stats[0] * 3 / 2;
    }
    let short: [bool; 5] = std::array::from_fn(|a| eff[a] < w.req[a]);
    let el = data().elements.get(&w.element);
    let mut out = Attack { total: 0, damage: [0; 5], status: [0; 7], two_handed: two, weakened: false, spell_scaling: None };

    let catalyst = w.sorcery || w.incantation;
    for t in 0..5 {
        if base[t] <= 0.0 && !catalyst {
            continue;
        }
        let scales = |a: usize| el.and_then(|e| e[t * 5 + a]);
        let mut mult = 1.0;
        if (0..5).any(|a| short[a] && scales(a).is_some()) {
            mult = 0.6;
            if base[t] > 0.0 {
                out.weakened = true;
            }
        } else {
            for a in 0..5 {
                let Some(over) = scales(a) else { continue };
                if scaling[a] <= 0.0 {
                    continue;
                }
                let s = over.unwrap_or(scaling[a]) / 100.0;
                mult += curve(w.graph[t], eff[a]) * s;
            }
        }
        out.damage[t] = (base[t] * mult) as u32;
        if (w.sorcery && t == 1) || (w.incantation && t == 4 && out.spell_scaling.is_none()) {
            out.spell_scaling = Some((100.0 * mult) as u32);
        }
    }
    out.total = out.damage.iter().sum();

    // Poison, bleed, sleep and madness scale with Arcane when the weapon has Arcane scaling.
    for (s, graph) in [(0, 5), (2, 6), (4, 7), (5, 8)] {
        if status[s] <= 0.0 {
            continue;
        }
        let mult = if stats[4] < w.req[4] {
            0.6
        } else if scaling[4] > 0.0 {
            1.0 + curve(w.graph[graph], stats[4]) * scaling[4] / 100.0
        } else {
            1.0
        };
        out.status[s] = (status[s] * mult) as u32;
    }
    for s in [1, 3, 6] {
        out.status[s] = status[s] as u32;
    }
    out
}

/// The scaling letter the game shows for a value ×100.
pub fn letter(v: f64) -> Option<&'static str> {
    Some(match v {
        v if v >= 175.0 => "S",
        v if v >= 140.0 => "A",
        v if v >= 90.0 => "B",
        v if v >= 60.0 => "C",
        v if v >= 25.0 => "D",
        v if v > 0.0 => "E",
        _ => return None,
    })
}

impl Weapon {
    /// Scaling letters, e.g. [("Str", "D"), ("Dex", "C")].
    pub fn letters(&self) -> Vec<(&'static str, &'static str)> {
        (0..5).filter_map(|a| letter(self.scaling[a]).map(|l| (ATTRS[a], l))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ar(name: &str, stats: [u32; 5]) -> Attack {
        attack(weapon(name).unwrap_or_else(|| panic!("{name} missing")), stats, false)
    }

    #[test]
    fn reads_skills_and_spells() {
        let m = weapon("Moonveil").unwrap();
        assert_eq!((m.skill, m.skill_fp), ("Transient Moonlight", "15/20"));
        assert_eq!(weapon("Uchigatana").unwrap().letters(), vec![("Str", "D"), ("Dex", "D")]);
        assert_eq!(weapon("Coded Sword").unwrap().letters(), vec![("Fai", "B")]);
        let pebble = spell("Glintstone Pebble").unwrap();
        assert_eq!((pebble.fp, pebble.slots, pebble.req[0]), (7, 1, 10));
        assert!(weapon("Glintstone Staff").unwrap().sorcery && weapon("Finger Seal").unwrap().incantation);
        assert!(built_for().is_some_and(|v| v.split('.').count() >= 3), "weapons.tsv starts with the game version");
    }

    #[test]
    fn under_requirement_is_weakened() {
        // Bloodhound's Fang needs 18 Str; at 12 it only works two-handed (18).
        let a = ar("Bloodhound's Fang", [12, 17, 10, 10, 10]);
        assert!(a.two_handed && !a.weakened);
        let a = ar("Bloodhound's Fang", [10, 17, 10, 10, 10]);
        assert!(a.weakened && a.damage[0] == (141.0 * 0.6) as u32);
    }

    #[test]
    fn known_attack_ratings() {
        // Worked by hand: 115 + 115 × (0.36 × curve(12) + 0.55 × curve(15)) = 115 + 18.
        let a = ar("Uchigatana", [12, 15, 10, 10, 10]);
        assert_eq!((a.total, a.status[2], a.two_handed, a.weakened), (133, 45, false, false));
        // Rivers of Blood's bleed scales with Arcane.
        let low = ar("Rivers of Blood", [12, 18, 10, 10, 20]).status[2];
        let high = ar("Rivers of Blood", [12, 18, 10, 10, 40]).status[2];
        assert!(low >= 50 && high > low);
        // A catalyst reports its spell scaling.
        assert!(ar("Glintstone Staff", [6, 10, 16, 10, 10]).spell_scaling.unwrap() > 100);
    }

    #[test]
    fn upgrades() {
        let st = [12, 15, 10, 10, 10];
        let u = weapon("Uchigatana").unwrap();
        assert_eq!(u.max_level, 25);
        assert_eq!(attack_at(u, st, false, 0).total, 133);
        // +25 Uchigatana: 115 × 2.45 = 281 physical before scaling.
        let (b, _, _) = upgraded(u, 25);
        assert_eq!(b[0] as u32, 281);
        let at = |lv| attack_at(u, st, false, lv).total;
        assert!(at(6) > at(0) && at(12) > at(6) && at(25) > at(12));
        // Somber weapons stop at +10; bleed grows with upgrades.
        let r = weapon("Rivers of Blood").unwrap();
        assert_eq!(r.max_level, 10);
        let rs = [12, 18, 10, 10, 20];
        assert_eq!(attack_at(r, rs, false, 25).total, attack_at(r, rs, false, 10).total);
        assert!(attack_at(r, rs, false, 10).status[2] > attack_at(r, rs, false, 0).status[2]);
        // No upgrades at all.
        let m = weapon("Meteorite Staff").unwrap();
        assert_eq!(m.max_level, 0);
        let ms = [6, 10, 18, 10, 10];
        assert_eq!(attack_at(m, ms, false, 10).spell_scaling, attack(m, ms, false).spell_scaling);
        // Catalysts: spell scaling grows.
        let g = weapon("Glintstone Staff").unwrap();
        assert!(attack_at(g, [6, 10, 16, 10, 10], false, 12).spell_scaling > attack(g, [6, 10, 16, 10, 10], false).spell_scaling);
    }
}

/// Prints this seed's class weapons at their stats: `cargo test seed_attack -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn seed_attack() {
    let log = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../Bingo Randomizer/Cache/Spoilers/Current_Seed.log")).unwrap();
    let sp = crate::classes::parse(&log).unwrap();
    for c in &sp.classes {
        let g = |k: &str| c.stats.iter().find(|(n, _)| n == k).map_or(0, |(_, v)| *v);
        let st = [g("Strength"), g("Dexterity"), g("Intelligence"), g("Faith"), g("Arcane")];
        for it in &c.items {
            if let Some(w) = weapon(&it.name) {
                let a = attack(w, st, false);
                println!("{:<11} {:<30} AR {:>4} 2H {} weak {} spell {:?} status {:?} | {} {} | {:?}", c.name, it.name, a.total, a.two_handed, a.weakened, a.spell_scaling, a.status, w.skill, w.skill_fp, w.letters());
            } else if let Some(s) = spell(&it.name) {
                println!("{:<11} {:<30} FP {} req {:?}", c.name, it.name, s.fp, s.req);
            }
        }
    }
}
