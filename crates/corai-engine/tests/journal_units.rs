//! Loadouts written by the game itself, as opposed to EDSY's SLEF or Coriolis' exports: the game
//! writes resistances, shield and hull boosts and thruster/shield performance as percentages and
//! sensor range in metres; it names stock fitted modules `…_free`; it rounds engineered cargo racks
//! to whole tonnes. Built from the parity fixtures (Coriolis units) by converting them the way the
//! game writes them.

use corai_engine::{codec, journal};
use serde_json::{Value, json};

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/interchange.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

const GAME_SCALE: &[(&str, f64)] = &[
    ("KineticResistance", 100.0),
    ("ThermicResistance", 100.0),
    ("ExplosiveResistance", 100.0),
    ("DefenceModifierHealthMultiplier", 100.0),
    ("DefenceModifierShieldMultiplier", 100.0),
    ("EngineOptPerformance", 100.0),
    ("ShieldGenStrength", 100.0),
    ("Range", 1000.0),
];

/// The Loadout as the game would write it, and how many modifiers changed units.
fn as_game_writes_it(loadout: &Value) -> (Value, usize) {
    let mut out = loadout.clone();
    out["timestamp"] = "2026-11-12T10:00:00Z".into();
    let mut scaled = 0;
    for m in out["Modules"].as_array_mut().into_iter().flatten() {
        for mf in m["Engineering"]["Modifiers"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let label = mf["Label"].as_str().unwrap_or_default().to_string();
            if let Some((_, s)) = GAME_SCALE.iter().find(|(l, _)| *l == label) {
                for k in ["Value", "OriginalValue"] {
                    if let Some(v) = mf[k].as_f64() {
                        mf[k] = json!(v * s);
                    }
                }
                scaled += 1;
            }
        }
    }
    (out, scaled)
}

fn figures(ship: &corai_engine::Ship) -> [f64; 6] {
    let s = &ship.stats;
    [
        s.top_speed,
        s.top_boost,
        s.shield,
        s.armour,
        s.dry_mass,
        s.cargo_capacity,
    ]
}

#[test]
fn game_and_edsy_units_give_the_same_ship() {
    let mut checked = 0;
    for c in fx_cases() {
        let loadout = &c["exports"]["loadout"];
        let (game, scaled) = as_game_writes_it(loadout);
        if scaled == 0 {
            continue;
        }
        let (from_edsy, edsy_issues) = journal::ship_from_loadout(loadout).expect("imports");
        let (from_game, game_issues) = journal::ship_from_loadout(&game).expect("imports");
        assert_eq!(edsy_issues, Vec::<String>::new(), "{}", c["ship"]);
        assert_eq!(game_issues, Vec::<String>::new(), "{}", c["ship"]);
        for (a, b) in figures(&from_edsy).iter().zip(figures(&from_game)) {
            let same = (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-6 * a.abs().max(1.0);
            assert!(same, "{}: {a} vs {b}", c["ship"]);
        }
        // Resistances and boosts too: the stored engineering is the same.
        assert_eq!(
            codec::encode(&from_game),
            codec::encode(&from_edsy),
            "{}",
            c["ship"]
        );
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} engineered builds");
}

fn fx_cases() -> Vec<Value> {
    fixture()["cases"].as_array().cloned().unwrap_or_default()
}

#[test]
fn stock_fitted_modules_are_the_modules_they_name() {
    let c = &fx_cases()[1];
    let mut free = c["exports"]["loadout"].clone();
    for m in free["Modules"].as_array_mut().expect("modules") {
        if m["Slot"].as_str().is_some_and(|s| s.starts_with("Slot")) {
            m["Item"] = format!("{}_free", m["Item"].as_str().expect("item")).into();
        }
    }
    let (paid, _) = journal::ship_from_loadout(&c["exports"]["loadout"]).expect("imports");
    let (stock, _) = journal::ship_from_loadout(&free).expect("imports");
    assert_eq!(codec::encode(&stock), codec::encode(&paid));
}

#[test]
fn an_engineered_cargo_rack_holds_whole_tonnes() {
    // Case 1 (Chieftain) carries a 5E rack (32 t); the game's Increased Capacity makes it 43 t.
    let c = &fx_cases()[1];
    let mut loadout = c["exports"]["loadout"].clone();
    let rack = loadout["Modules"]
        .as_array_mut()
        .expect("modules")
        .iter_mut()
        .find(|m| m["Item"] == "int_cargorack_size5_class1")
        .expect("a 5E cargo rack");
    rack["Engineering"] = json!({ "BlueprintName": "CargoRack_IncreasedCapacity", "Level": 1, "Quality": 1,
        "Modifiers": [{ "Label": "CargoCapacity", "Value": 43.0, "OriginalValue": 32.0, "LessIsGood": 0 }] });
    let (before, _) = journal::ship_from_loadout(&c["exports"]["loadout"]).expect("imports");
    let (after, issues) = journal::ship_from_loadout(&loadout).expect("imports");
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(
        after.stats.cargo_capacity - before.stats.cargo_capacity,
        11.0
    );
}
