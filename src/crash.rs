//! Explains crashes of the Randomizer and the game, using the reason Windows logs for them.

use std::os::windows::process::CommandExt;
use std::process::Command;

use crate::core;

#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    pub title: String,
    pub body: String,
    /// Event time, used to avoid reporting the same crash twice.
    pub when: String,
}

struct Event {
    source: String,
    date: String,
    app: String,
    exception: String,
    text: String,
}

/// Crash events from the Application log in the last `secs` seconds, newest first. Read as XML:
/// the plain-text form's labels and messages are translated on non-English Windows, the XML
/// field names aren't.
fn recent_events(secs: u64) -> Vec<Event> {
    let q = format!(
        "*[System[Provider[@Name='.NET Runtime' or @Name='Application Error'] and TimeCreated[timediff(@SystemTime) <= {}]]]",
        secs * 1000
    );
    let out = Command::new("wevtutil")
        .args(["qe", "Application", &format!("/q:{q}"), "/f:xml", "/c:10", "/rd:true"])
        .creation_flags(crate::sys::CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return vec![] };
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Text between `start` and the next `end` after it.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    Some(&s[a..a + s[a..].find(end)?])
}

fn parse(xml: &str) -> Vec<Event> {
    xml.split("<Event ")
        .skip(1)
        .map(|ev| {
            let source = between(ev, "<Provider Name='", "'").unwrap_or_default().to_string();
            let date = between(ev, "SystemTime='", "'").unwrap_or_default().to_string();
            // Application Error has named fields; .NET Runtime has one block of English text.
            let text = unescape(between(ev, "<Data>", "</Data>").or_else(|| between(ev, "<EventData>", "</EventData>")).unwrap_or_default());
            let named = between(ev, "<Data Name='AppName'>", "</Data>").map(unescape);
            let line = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)).map(|v| v.trim().to_string()).unwrap_or_default();
            let app = named.unwrap_or_else(|| line("Application:")).to_lowercase();
            Event { source, date, app, exception: line("Exception Info:"), text }
        })
        .collect()
}

fn first_sentence(s: &str) -> String {
    let s = s.split(" at ").next().unwrap_or(s).trim();
    let s: String = s.chars().take(220).collect();
    s
}

/// Why the Randomizer (exe file name, any case) just crashed, if Windows logged it.
pub fn randomizer(exe: &str, within_secs: u64) -> Option<Crash> {
    let exe = exe.to_lowercase();
    let ev = recent_events(within_secs).into_iter().find(|e| e.source == ".NET Runtime" && e.app == exe)?;
    let x = &ev.exception;
    let (title, body) = if x.contains("KeyNotFoundException") && ev.text.contains("addDescriptionString") {
        (
            "This seed doesn't work",
            "This seed gave a starting class a weapon the Randomizer has no name for, usually one added by a game update (Setup lists them). Change a letter of the seed and send the new one to everyone in the race.".to_string(),
        )
    } else if x.contains("IOException") && x.contains("Access to the path") {
        core::unlock_randomizer_output();
        (
            "The Randomizer couldn't write its files",
            "Its output folder was locked. The hub has unlocked it, so press Randomize again. If it happens again, close any File Explorer window open on the Bingo Randomizer folder.".to_string(),
        )
    } else if x.contains("FileNotFoundException") || x.contains("DirectoryNotFoundException") {
        ("The Randomizer couldn't find a file", "Check the Elden Ring path at the top of the Randomizer, then try again.".to_string())
    } else {
        ("The Randomizer crashed", format!("Windows reported: {}", first_sentence(x)))
    };
    Some(Crash { title: title.into(), body, when: ev.date })
}

/// Why Elden Ring just crashed, if Windows logged it. `game_dir` is the install the hub uses.
pub fn game(within_secs: u64, game_dir: Option<&std::path::Path>) -> Option<Crash> {
    let ev = recent_events(within_secs).into_iter().find(|e| e.source == "Application Error" && e.app == "eldenring.exe")?;
    let p = core::paths();
    let mut hints = Vec::new();
    if !p.regulation.is_file() {
        hints.push("There was no randomized file to load. Randomize, then launch again.".to_string());
    }
    if let Some(dir) = game_dir {
        let mods: Vec<String> = ["dinput8.dll", "winhttp.dll", "dxgi.dll", "d3d12.dll", "version.dll"]
            .iter()
            .filter(|n| dir.join(n).is_file())
            .map(|n| n.to_string())
            .collect();
        if !mods.is_empty() {
            hints.push(format!("Other mods in the game folder ({}) load in bingo runs too and are a common cause. Try moving them out of the game folder.", mods.join(", ")));
        }
    }
    if hints.is_empty() {
        hints.push("If it keeps happening, verify the game files in Steam (right-click Elden Ring, Properties, Installed Files).".into());
    }
    Some(Crash { title: "Elden Ring crashed".into(), body: hints.join("\n\n"), when: ev.date })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='.NET Runtime'/><EventID Qualifiers='0'>1026</EventID><TimeCreated SystemTime='2026-11-14T15:18:54.8270000Z'/></System><EventData><Data>Application: BingoRandomizer0.91.exe
CoreCLR Version: 8.0.28
Exception Info: System.Collections.Generic.KeyNotFoundException: The given key &apos;66530000&apos; was not present in the dictionary.
   at Project.Tasks.Randomizer.addDescriptionString(CharaInitParam chr, Int32 id)</Data></EventData></Event><Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Application Error' Guid='{a0e9b465}'/><EventID>1000</EventID><TimeCreated SystemTime='2026-11-14T02:04:21.0000000Z'/></System><EventData><Data Name='AppName'>eldenring.exe</Data><Data Name='AppVersion'>2.7.2.0</Data></EventData></Event>";

    #[test]
    fn parses_both_kinds() {
        let ev = parse(SAMPLE);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].app, "bingorandomizer0.91.exe");
        assert!(ev[0].exception.starts_with("System.Collections.Generic.KeyNotFoundException: The given key '66530000'"));
        assert!(ev[0].text.contains("addDescriptionString"));
        assert_eq!(ev[1].source, "Application Error");
        assert_eq!(ev[1].app, "eldenring.exe");
        assert_eq!(ev[1].date, "2026-11-14T02:04:21.0000000Z");
    }
}

#[cfg(test)]
mod live {
    /// Reads the real event log: `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn explains_real_crashes() {
        println!("{:#?}", super::randomizer("S2_Style_1.0.exe", 3 * 86_400));
        println!("{:#?}", super::game(3 * 86_400, None));
    }
}
