//! The ship the player flies, from the newest journal Loadout, re-read only when the newest journal
//! file changes.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use corai_engine::{interchange, journal};

/// What the window shows about the current ship.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentShip {
    /// Model, e.g. "Krait Mk II".
    pub model: String,
    pub name: String,
    pub ident: String,
    pub link: Option<String>,
}

/// The ship described by one Loadout event.
pub fn describe(loadout: &corai_journal::Loadout) -> CurrentShip {
    let model = journal::ship_id_from_fd(&loadout.ship)
        .and_then(|id| corai_data::DATA.ships.get(id))
        .map_or_else(|| loadout.ship.clone(), |s| s.properties.name.clone());
    let name = (!loadout.ship_name.is_empty()).then_some(loadout.ship_name.as_str());
    let link = interchange::parse_build(&loadout.line, None)
        .ok()
        .map(|i| interchange::coriolis_url(&i.ship, name));
    CurrentShip {
        model,
        name: loadout.ship_name.clone(),
        ident: loadout.ship_ident.clone(),
        link,
    }
}

/// Watches the journal folder; `poll` answers the ship again only after the newest file changed.
#[derive(Debug, Default)]
pub struct Watcher {
    seen: Option<(PathBuf, SystemTime, u64)>,
}

impl Watcher {
    /// `Some(ship)` when the newest journal file changed since the last poll (the ship may still be
    /// `None` when no Loadout was written yet); `None` when nothing changed.
    pub fn poll(&mut self, dir: &Path) -> Option<Option<CurrentShip>> {
        let newest = corai_journal::journal_files(dir).ok()?.into_iter().next()?;
        let meta = std::fs::metadata(&newest).ok()?;
        let stamp = (newest, meta.modified().ok()?, meta.len());
        if self.seen.as_ref() == Some(&stamp) {
            return None;
        }
        self.seen = Some(stamp);
        Some(
            corai_journal::latest_loadout(dir)
                .ok()
                .flatten()
                .map(|l| describe(&l)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn game_loadout(case: usize) -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/corai-engine/tests/fixtures/interchange.json"
        );
        let fx: Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
        let mut l = fx["cases"][case]["exports"]["loadout"].clone();
        l["ShipID"] = 7.into();
        l["ShipIdent"] = "KD-07".into();
        l.to_string()
    }

    #[test]
    fn the_newest_loadout_names_the_ship_and_links_it() {
        let dir = std::env::temp_dir().join(format!("corai-window-ship-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("Journal.2026-01-01T100000.01.log");
        let mut w = Watcher::default();
        assert_eq!(w.poll(&dir), None, "no journal file yet");

        std::fs::write(&file, "{\"event\":\"Fileheader\"}\n").expect("write");
        assert_eq!(w.poll(&dir), Some(None), "a journal without a Loadout");
        assert_eq!(w.poll(&dir), None, "unchanged file: not read again");

        std::fs::write(
            &file,
            format!("{{\"event\":\"Fileheader\"}}\n{}\n", game_loadout(0)),
        )
        .expect("write");
        let ship = w.poll(&dir).flatten().expect("a ship");
        assert_eq!(ship.model, "Adder");
        assert_eq!(
            (ship.name.as_str(), ship.ident.as_str()),
            ("Oracle", "KD-07")
        );
        assert!(
            ship.link
                .is_some_and(|l| l.starts_with("https://coriolis.io/outfit/adder?code="))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
