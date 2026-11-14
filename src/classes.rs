//! Starting classes for the current seed, from the Randomizer's spoiler log
//! (Bingo Randomizer\Cache\Spoilers\Current_Seed.log).

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

#[derive(Clone, PartialEq, Debug)]
pub struct Spoiler {
    pub seed: String,
    pub classes: Vec<Class>,
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
    Item { name, level, count }
}

pub fn parse(text: &str) -> Option<Spoiler> {
    let seed = text.lines().find_map(|l| l.trim().strip_prefix("Seed:")).map(|s| s.trim().to_string())?;
    let mut classes: Vec<Class> = Vec::new();
    let mut in_stats = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("## ") {
            classes.push(Class { name: name.trim().to_string(), items: vec![], stats: vec![] });
            in_stats = false;
        } else if t.starts_with("__") {
            in_stats = true;
        } else if t.is_empty() {
            continue;
        } else if let Some(c) = classes.last_mut() {
            if in_stats {
                if let Some((k, v)) = t.split_once(':') {
                    if let Ok(v) = v.trim().parse() {
                        c.stats.push((k.trim().to_string(), v));
                    }
                }
            } else {
                c.items.extend(t.split(", ").filter(|s| !s.trim().is_empty()).map(parse_item));
            }
        }
    }
    (!classes.is_empty()).then_some(Spoiler { seed, classes })
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
    }
}
