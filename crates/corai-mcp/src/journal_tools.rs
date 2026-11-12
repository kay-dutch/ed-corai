//! `current_ship` and `recent_ships`: the player's own ships from the game journal, evaluated by
//! the service like any other build.

use std::path::Path;

use corai_journal::Loadout;
use corai_service::dispatch::call;
use serde_json::{Map, Value, json};

/// Figures listed per ship in `recent_ships`.
const KEY_METRICS: &[&str] = &[
    "unladenRange",
    "ladenRange",
    "cargoCapacity",
    "topSpeed",
    "shieldEHP",
    "hullEHP",
    "sustainedDps",
];

fn no_journal() -> String {
    format!(
        "No Elite Dangerous journal folder found. ED Corai reads Saved Games\\Frontier Developments\\Elite Dangerous \
         in the Windows user folder; set {} to use another folder.",
        corai_journal::DIR_ENV
    )
}

fn read_error(dir: &Path, e: &std::io::Error) -> String {
    format!("Cannot read the journal folder {}: {e}", dir.display())
}

fn journal_json(l: &Loadout) -> Value {
    json!({
        "shipId": l.ship_id,
        "shipName": l.ship_name,
        "shipIdent": l.ship_ident,
        "lastSeen": l.timestamp,
    })
}

fn evaluate(l: &Loadout, context: Option<&Value>) -> Result<Value, String> {
    let mut args = json!({ "build": l.line });
    if let Some(c) = context {
        args["context"] = c.clone();
    }
    call("evaluate_build", &args).map_err(|e| e.to_string())
}

/// `current_ship`: the ship flown now (or the one with `shipId`), fully evaluated.
///
/// # Errors
/// No journal folder, no Loadout event, or the build cannot be read.
pub fn current_ship(dir: Option<&Path>, args: &Value) -> Result<Value, String> {
    let dir = dir.ok_or_else(no_journal)?;
    let found = match args["shipId"].as_u64() {
        Some(id) => corai_journal::loadout_of(dir, id),
        None => corai_journal::latest_loadout(dir),
    }
    .map_err(|e| read_error(dir, &e))?;
    let l = found.ok_or_else(|| match args["shipId"].as_u64() {
        Some(id) => format!("No ship with shipId {id} in the journal. Use recent_ships."),
        None => "The journal has no Loadout yet: the game writes one when you log in, switch ship or outfit.".into(),
    })?;
    Ok(json!({ "journal": journal_json(&l), "build": evaluate(&l, args.get("context"))? }))
}

/// `recent_ships`: the ships flown recently, newest first, with links and key figures.
///
/// # Errors
/// No journal folder or it cannot be read.
pub fn recent_ships(dir: Option<&Path>, args: &Value) -> Result<Value, String> {
    let dir = dir.ok_or_else(no_journal)?;
    let limit = args["limit"].as_u64().map_or(10, |n| n.clamp(1, 50)) as usize;
    let ships = corai_journal::recent_loadouts(dir, limit).map_err(|e| read_error(dir, &e))?;
    let rows: Vec<Value> = ships
        .iter()
        .map(|l| {
            let mut row = match journal_json(l) {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            match evaluate(l, None) {
                Ok(s) => {
                    row.insert("ship".into(), s["ship"].clone());
                    let metrics: Map<String, Value> = KEY_METRICS
                        .iter()
                        .filter_map(|k| s["metrics"].get(*k).map(|v| ((*k).to_string(), v.clone())))
                        .collect();
                    row.insert("key".into(), Value::Object(metrics));
                    row.insert("coriolisUrl".into(), s["coriolisUrl"].clone());
                    row.insert("edsyUrl".into(), s["edsyUrl"].clone());
                }
                Err(e) => {
                    row.insert("ship".into(), l.ship.clone().into());
                    row.insert("error".into(), e.into());
                }
            }
            Value::Object(row)
        })
        .collect();
    Ok(Value::Array(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real build as the game writes it: the exported Loadout of the parity fixtures, with the
    /// fields only the game adds.
    fn game_loadout(case: usize, ship_id: u64, ts: &str) -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../corai-engine/tests/fixtures/interchange.json"
        );
        let fx: Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
        let mut l = fx["cases"][case]["exports"]["loadout"].clone();
        l["timestamp"] = ts.into();
        l["ShipID"] = ship_id.into();
        l["ShipIdent"] = format!("ID-{ship_id}").into();
        l.to_string()
    }

    fn journal(lines: &[String]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("corai-mcp-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("Journal.2026-01-01T100000.01.log"),
            lines.join("\n"),
        )
        .expect("write");
        dir
    }

    #[test]
    fn the_flown_ship_is_evaluated_and_listed() {
        let dir = journal(&[
            game_loadout(0, 11, "2026-01-01T10:00:00Z"),
            game_loadout(2, 12, "2026-01-01T11:00:00Z"),
        ]);
        let now = current_ship(Some(&dir), &json!({})).expect("current ship");
        assert_eq!(now["journal"]["shipId"], 12);
        assert_eq!(now["journal"]["lastSeen"], "2026-01-01T11:00:00Z");
        assert!(
            now["build"]["metrics"]["unladenRange"]
                .as_f64()
                .is_some_and(|r| r > 0.0)
        );
        assert!(
            now["build"]["coriolisUrl"]
                .as_str()
                .is_some_and(|u| u.contains("coriolis.io"))
        );

        let other = current_ship(Some(&dir), &json!({ "shipId": 11 })).expect("by id");
        assert_ne!(other["build"]["ship"], now["build"]["ship"]);
        assert!(
            current_ship(Some(&dir), &json!({ "shipId": 99 }))
                .is_err_and(|e| e.contains("recent_ships"))
        );

        let list = recent_ships(Some(&dir), &json!({})).expect("recent");
        let ids: Vec<_> = list
            .as_array()
            .expect("list")
            .iter()
            .map(|r| r["shipId"].clone())
            .collect();
        assert_eq!(ids, vec![json!(12), json!(11)]);
        assert!(list[0]["key"]["unladenRange"].is_number());
        assert_eq!(list[0]["ship"], now["build"]["ship"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_journal_the_answer_says_where_to_look() {
        assert!(current_ship(None, &json!({})).is_err_and(|e| e.contains(corai_journal::DIR_ENV)));
        assert!(recent_ships(None, &json!({})).is_err());
    }
}
