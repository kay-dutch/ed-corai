//! The player's ships from the Elite Dangerous journal.
//!
//! The game writes a `Loadout` event (the full outfitting, engineering included) when the player
//! logs in, switches ship and after outfitting. The newest one is the ship flown now; the newest
//! one per `ShipID` lists the ships flown recently. Files are read newest first, by modification
//! time, since the file name format changed in 2021 and names no longer sort by date.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// Journal files read at most per request: enough for months of play, and still quick.
pub const MAX_FILES: usize = 400;

/// Environment variable that points at another journal folder.
pub const DIR_ENV: &str = "CORAI_JOURNAL_DIR";

/// The journal folder: `CORAI_JOURNAL_DIR`, else `Saved Games\Frontier Developments\Elite
/// Dangerous` in the user profile (Windows; elsewhere only when it exists there).
pub fn journal_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let dir = PathBuf::from(home)
        .join("Saved Games")
        .join("Frontier Developments")
        .join("Elite Dangerous");
    dir.is_dir().then_some(dir)
}

/// `Journal.*.log` files in `dir`, newest first.
pub fn journal_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("Journal.") && name.ends_with(".log")
        })
        .map(|e| {
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, e.path())
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    Ok(files.into_iter().map(|(_, p)| p).collect())
}

/// One `Loadout` event.
#[derive(Debug, Clone, PartialEq)]
pub struct Loadout {
    /// The event line as written, for the build importer.
    pub line: String,
    pub ship_id: Option<u64>,
    /// Journal ship type, e.g. `krait_mkii`.
    pub ship: String,
    pub ship_name: String,
    pub ship_ident: String,
    pub timestamp: String,
    pub file: PathBuf,
}

/// Loadout events of one file, newest first.
fn loadouts_in(file: &Path) -> Vec<Loadout> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .filter(|l| l.contains("\"Loadout\""))
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line).ok()?;
            (v["event"] == "Loadout").then(|| {
                let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
                Loadout {
                    line: line.to_string(),
                    ship_id: v["ShipID"].as_u64(),
                    ship: s("Ship"),
                    ship_name: s("ShipName"),
                    ship_ident: s("ShipIdent"),
                    timestamp: s("timestamp"),
                    file: file.to_path_buf(),
                }
            })
        })
        .collect()
}

/// The newest loadout per ship, newest first, at most `limit` ships.
///
/// # Errors
/// The folder cannot be read.
pub fn recent_loadouts(dir: &Path, limit: usize) -> std::io::Result<Vec<Loadout>> {
    let mut out: Vec<Loadout> = Vec::new();
    for file in journal_files(dir)?.into_iter().take(MAX_FILES) {
        for l in loadouts_in(&file) {
            let seen = out
                .iter()
                .any(|o| o.ship_id == l.ship_id && (l.ship_id.is_some() || o.ship == l.ship));
            if !seen {
                out.push(l);
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

/// The newest loadout: the ship flown now (or last).
///
/// # Errors
/// The folder cannot be read.
pub fn latest_loadout(dir: &Path) -> std::io::Result<Option<Loadout>> {
    Ok(recent_loadouts(dir, 1)?.into_iter().next())
}

/// The newest loadout of the ship with this `ShipID`.
///
/// # Errors
/// The folder cannot be read.
pub fn loadout_of(dir: &Path, ship_id: u64) -> std::io::Result<Option<Loadout>> {
    for file in journal_files(dir)?.into_iter().take(MAX_FILES) {
        if let Some(l) = loadouts_in(&file)
            .into_iter()
            .find(|l| l.ship_id == Some(ship_id))
        {
            return Ok(Some(l));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn loadout(id: u64, ship: &str, name: &str, ts: &str) -> String {
        format!(
            r#"{{ "timestamp":"{ts}", "event":"Loadout", "Ship":"{ship}", "ShipID":{id}, "ShipName":"{name}", "ShipIdent":"ID-{id}", "Modules":[] }}"#
        )
    }

    /// Writes journal files oldest first, each one second newer than the last.
    fn journal(name: &str, files: &[(&str, Vec<String>)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corai-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let start = SystemTime::now() - Duration::from_secs(3600);
        for (i, (file, lines)) in files.iter().enumerate() {
            let path = dir.join(file);
            std::fs::write(&path, lines.join("\r\n")).expect("write");
            let f = std::fs::File::options()
                .write(true)
                .open(&path)
                .expect("open");
            f.set_modified(start + Duration::from_secs(i as u64))
                .expect("mtime");
        }
        std::fs::write(dir.join("Status.json"), "{}").expect("write");
        dir
    }

    #[test]
    fn the_newest_loadout_is_the_current_ship() {
        let other = r#"{ "timestamp":"x", "event":"FSDJump", "StarSystem":"Sol" }"#.to_string();
        // An old-format name that sorts after the new format: order comes from the file times.
        let dir = journal(
            "current",
            &[
                (
                    "Journal.2025-01-01T100000.01.log",
                    vec![loadout(1, "python", "Hauler", "a"), other.clone()],
                ),
                (
                    "Journal.230101100000.01.log",
                    vec![
                        loadout(2, "krait_mkii", "Raider", "b"),
                        loadout(3, "anaconda", "Big", "c"),
                        other,
                    ],
                ),
                ("Journal.2025-02-01T100000.01.log", vec!["not json".into()]),
            ],
        );
        let now = latest_loadout(&dir).expect("reads").expect("found");
        assert_eq!(
            (now.ship_id, now.ship.as_str(), now.ship_name.as_str()),
            (Some(3), "anaconda", "Big")
        );
        let ids: Vec<_> = recent_loadouts(&dir, 10)
            .expect("reads")
            .iter()
            .map(|l| l.ship_id)
            .collect();
        assert_eq!(ids, vec![Some(3), Some(2), Some(1)]);
        assert_eq!(recent_loadouts(&dir, 2).expect("reads").len(), 2);
        assert_eq!(
            loadout_of(&dir, 1).expect("reads").map(|l| l.ship_name),
            Some("Hauler".into())
        );
        assert_eq!(loadout_of(&dir, 9).expect("reads"), None);
    }

    #[test]
    fn a_ship_counts_once_with_its_newest_outfitting() {
        let dir = journal(
            "dedup",
            &[(
                "Journal.2025-03-01T100000.01.log",
                vec![
                    loadout(5, "python", "Before", "a"),
                    loadout(5, "python", "After", "b"),
                ],
            )],
        );
        let ships = recent_loadouts(&dir, 10).expect("reads");
        assert_eq!(ships.len(), 1);
        assert_eq!(ships[0].ship_name, "After");
    }

    #[test]
    fn an_empty_journal_has_no_ship() {
        let dir = journal("empty", &[]);
        assert_eq!(latest_loadout(&dir).expect("reads"), None);
        assert!(latest_loadout(&dir.join("missing")).is_err());
    }
}
