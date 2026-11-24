//! Starting classes for the current seed, from the Randomizer's spoiler log
//! (Bingo Randomizer\Cache\Spoilers\Current_Seed.log). After the classes the log lists what the
//! seed swapped (weapons everywhere, then each shop's spells); that part is kept apart so the
//! page can hide it.

use std::path::PathBuf;

use crate::core;

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub name: String,
    /// "Level estimate" the Randomizer prints when the class can't wield it yet.
    pub level: Option<u32>,
    /// Ammo count, e.g. arrows.
    pub count: Option<u32>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Class {
    pub name: String,
    pub items: Vec<Item>,
    /// (stat, value) in the game's order.
    pub stats: Vec<(String, u32)>,
}

/// One list of swaps, e.g. "Weapons" or a merchant's spells: (vanilla item, what replaces it).
#[derive(Clone, PartialEq, Debug)]
pub struct Swaps {
    pub group: String,
    pub pairs: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Spoiler {
    pub seed: String,
    pub classes: Vec<Class>,
    pub swaps: Vec<Swaps>,
}

pub fn log_path() -> PathBuf {
    core::paths().rando_dir.join("Cache").join("Spoilers").join("Current_Seed.log")
}

fn parse_item(s: &str) -> Item {
    let mut name = s.trim().to_string();
    let mut grab = |open: char, close: char| -> Option<u32> {
        if name.ends_with(close) {
            if let Some(i) = name.rfind(open) {
                let v = name[i + 1..name.len() - 1].trim().parse().ok();
                if v.is_some() {
                    name = name[..i].trim().to_string();
                }
                return v;
            }
        }
        None
    };
    let count = grab('[', ']');
    let level = grab('(', ')');
    // Classes always start at +0. The Randomizer's names can carry a stray "+25" (its list of preset
    // NPC weapons shares IDs with infused weapons and overwrites their names), so drop any level.
    if let Some((base, lv)) = name.rsplit_once(" +") {
        if !lv.is_empty() && lv.chars().all(|c| c.is_ascii_digit()) {
            name = base.trim_end().to_string();
        }
    }
    Item { name, level, count }
}

pub fn parse(text: &str) -> Option<Spoiler> {
    let seed = text.lines().find_map(|l| l.trim().strip_prefix("Seed:")).map(|s| s.trim().to_string())?;
    let mut classes: Vec<Class> = Vec::new();
    let mut swaps: Vec<Swaps> = Vec::new();
    let mut in_stats = false;
    let mut in_swaps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // ">> Item Replacements ..." / "<> Shop Replacements ...": the classes are done.
        if t.starts_with(">>") || t.starts_with("<>") {
            in_swaps = true;
            continue;
        }
        if in_swaps {
            if let Some(g) = t.strip_prefix("## ").or_else(|| t.strip_prefix("~* ")) {
                let g = g.trim().trim_end_matches('.').trim();
                let g = if g == "Guaranteed Weapons" { "Weapons" } else { g };
                if !swaps.last().is_some_and(|s| s.group == g) {
                    swaps.push(Swaps { group: g.to_string(), pairs: vec![] });
                }
            } else if let Some((a, b)) = [" --> ", " -> ", " : "].iter().find_map(|sep| t.split_once(sep)) {
                let (a, b) = (a.trim(), b.trim().trim_end_matches(',').trim());
                if a != b {
                    if swaps.is_empty() {
                        swaps.push(Swaps { group: "Other".into(), pairs: vec![] });
                    }
                    swaps.last_mut().unwrap().pairs.push((a.to_string(), b.to_string()));
                }
            }
            continue;
        }
        if let Some(name) = t.strip_prefix("## ") {
            classes.push(Class { name: name.trim().to_string(), items: vec![], stats: vec![] });
            in_stats = false;
        } else if t.starts_with("__") {
            in_stats = true;
        } else if let Some(c) = classes.last_mut() {
            if in_stats {
                if let Some((k, v)) = t.split_once(':') {
                    if let Ok(v) = v.trim().parse() {
                        c.stats.push((k.trim().to_string(), v));
                    }
                }
            } else {
                // Items are comma separated, but some names have a comma ("Surge, O Flame!").
                for part in t.split(", ").filter(|s| !s.trim().is_empty()) {
                    let joined = c.items.last().map(|prev| parse_item(&format!("{}, {part}", prev.name)));
                    match joined {
                        Some(j) if crate::items::info(&j.name).is_some() => *c.items.last_mut().unwrap() = j,
                        _ => c.items.push(parse_item(part)),
                    }
                }
            }
        }
    }
    // A merchant can appear twice (Sellen does); show each once, and drop empty headings.
    let mut merged: Vec<Swaps> = Vec::new();
    for s in swaps {
        match merged.iter_mut().find(|m| m.group == s.group) {
            Some(m) => m.pairs.extend(s.pairs.into_iter().filter(|p| !m.pairs.contains(p)).collect::<Vec<_>>()),
            None => merged.push(s),
        }
    }
    merged.retain(|s| !s.pairs.is_empty());
    (!classes.is_empty()).then_some(Spoiler { seed, classes, swaps: merged })
}

pub fn load() -> Option<Spoiler> {
    parse(&std::fs::read_to_string(log_path()).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spoiler_log() {
        let s = parse("Starting Class Randomization\nSeed: gold gold gold\nLevel estimate ...\n\n## Vagabond\nTooth Whip(14), Battle Axe\n\n__ Starting Stats\nVigor: 7\nMind: 11\n\n## Prisoner\nMisbegotten Shortbow(1), Shattershard Arrow (Fletched)[21], Glintstone Arc\n\n__ Starting Stats\nVigor: 9\n").unwrap();
        assert_eq!(s.seed, "gold gold gold");
        assert_eq!(s.classes.len(), 2);
        assert_eq!(s.classes[0].items[0], Item { name: "Tooth Whip".into(), level: Some(14), count: None });
        assert_eq!(s.classes[0].stats, vec![("Vigor".into(), 7), ("Mind".into(), 11)]);
        assert_eq!(s.classes[1].items[1], Item { name: "Shattershard Arrow (Fletched)".into(), level: None, count: Some(21) });
        assert_eq!(parse_item("Quality Longsword +25").name, "Quality Longsword");
        assert_eq!(parse_item("Dueling Shield +0").name, "Dueling Shield");
        assert!(s.swaps.is_empty());
    }

    #[test]
    fn keeps_swaps_out_of_the_classes() {
        let s = parse("Seed: duelist

## Wretch
Club

__ Starting Stats
Vigor: 14

>> Item Replacements - all instances
## Guaranteed Weapons
Great Mace : Rotten Battle Hammer,
Great Stars : Great Stars,

<> Shop Replacements - Random

~* Shop Magic Replacement.

~* Sorceress Sellen
Glintstone Pebble --> Night Shard

~* Gowry
Pest Threads --> Black Flame

~* Sorceress Sellen
Carian Slicer --> Magic Glintblade
Glintstone Pebble --> Night Shard

~* Dragon Communion
Dragonfire -> Rotten Breath
").unwrap();
        assert_eq!(s.classes.len(), 1);
        assert_eq!(s.classes[0].stats, vec![("Vigor".into(), 14)]);
        let groups: Vec<_> = s.swaps.iter().map(|g| (g.group.as_str(), g.pairs.len())).collect();
        assert_eq!(groups, vec![("Weapons", 1), ("Sorceress Sellen", 2), ("Gowry", 1), ("Dragon Communion", 1)]);
        assert_eq!(s.swaps[0].pairs[0], ("Great Mace".into(), "Rotten Battle Hammer".into()));
    }

    #[test]
    fn reads_the_current_log() {
        // The test exe isn't in the install, so look there directly.
        let log = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("Bingo Randomizer").join("Cache").join("Spoilers").join("Current_Seed.log");
        let Ok(text) = std::fs::read_to_string(log) else { return };
        let s = parse(&text).unwrap();
        assert_eq!(s.classes.len(), 10, "{:?}", s.classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        for c in &s.classes {
            for it in &c.items {
                assert!(crate::items::info(&it.name).is_some(), "{} not in items.tsv", it.name);
            }
        }
    }
}
