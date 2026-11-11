//! Differential test against the upstream Coriolis engine.
//!
//! `tests/fixtures/oracle.json` holds random engineered builds evaluated by the original
//! JavaScript engine (`npm run oracle`). Every build is rebuilt here and every figure compared.

use std::collections::BTreeMap;

use corai_engine::ship::find_module;
use corai_engine::{STANDARD_GROUPS, Ship, SlotKind, calc, codec};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    /// Pre-v3 ('A') codes for ships whose slot layout changed, and what Coriolis decodes from them.
    #[serde(default)]
    legacy: Vec<Legacy>,
}

#[derive(Deserialize)]
struct Legacy {
    ship: String,
    code: String,
    decoded: Value,
}

#[derive(Deserialize)]
struct Case {
    ship: String,
    slots: IndexMap<String, Option<SlotSpec>>,
    expect: Value,
    /// Coriolis' share code for the build, and the stats Coriolis rebuilds from it.
    code: String,
    decoded: Value,
}

#[derive(Deserialize)]
struct SlotSpec {
    group: String,
    id: String,
    eng: Option<Eng>,
}

#[derive(Deserialize)]
struct Eng {
    blueprint: String,
    grade: u8,
    special: Option<String>,
}

fn apply_eng(m: &mut corai_engine::Module, eng: &Eng) {
    if let Some(pe) = &m.def.pre_engineered {
        // Mirrors the oracle: pre-engineered modules only take an experimental.
        if let Some(s) = &eng.special
            && pe.can_apply_experimental
            && m.blueprint().is_some()
        {
            m.set_special(Some(s)).expect("oracle special is valid");
        }
        return;
    }
    m.apply_blueprint(&eng.blueprint, eng.grade, 1.0)
        .expect("oracle blueprint is valid");
    if let Some(s) = &eng.special {
        m.set_special(Some(s)).expect("oracle special is valid");
    }
}

fn rebuild(case: &Case) -> Ship {
    let mut ship = Ship::empty(&case.ship).expect("ship exists");
    for (key, spec) in &case.slots {
        let (kind, n) = key.split_once(':').unwrap_or((key.as_str(), ""));
        if kind == "bulkheads" {
            let spec = spec.as_ref().expect("bulkheads always fitted");
            ship.use_bulkhead(spec.id.parse().expect("bulkhead index"))
                .expect("bulkhead exists");
            if let Some(eng) = &spec.eng {
                apply_eng(&mut ship.bulkhead, eng);
            }
            continue;
        }
        let slot = match kind {
            "standard" => {
                &mut ship.standard[STANDARD_GROUPS
                    .iter()
                    .position(|g| *g == n)
                    .expect("std group")]
            }
            "internal" => &mut ship.internal[n.parse::<usize>().expect("index") - 1],
            "hardpoint" | "utility" => {
                let want = if kind == "hardpoint" {
                    SlotKind::Hardpoint
                } else {
                    SlotKind::Utility
                };
                let nth = n.parse::<usize>().expect("index") - 1;
                ship.hardpoints
                    .iter_mut()
                    .filter(|s| s.kind == want)
                    .nth(nth)
                    .expect("slot exists")
            }
            other => panic!("unknown slot kind {other}"),
        };
        let def = spec.as_ref().map(|s| {
            find_module(&s.group, &s.id).unwrap_or_else(|| panic!("module {}/{}", s.group, s.id))
        });
        Ship::place(slot, def);
        if let (Some(m), Some(eng)) = (
            slot.module.as_mut(),
            spec.as_ref().and_then(|s| s.eng.as_ref()),
        ) {
            apply_eng(m, eng);
        }
    }
    ship.recalc();
    ship
}

/// Per-module figures keyed like the oracle's slot keys (`standard:pp`, `hardpoint:2`, ...).
fn module_records(ship: &Ship) -> Value {
    let num = |x: Option<f64>| match x {
        Some(v) if v.is_finite() => Value::from(v),
        _ => Value::Null,
    };
    let rec = |m: &corai_engine::Module| {
        serde_json::json!({
            "grp": m.grp(), "mass": num(m.get("mass")), "power": num(m.get("power")),
            "damage": num(m.get("damage")), "rof": num(Some(m.rof())), "dps": num(Some(m.dps())),
            "sdps": num(Some(m.sdps())), "eps": num(Some(m.eps())), "hps": num(Some(m.hps())),
        })
    };
    let mut out = serde_json::Map::new();
    for (i, slot) in ship.standard.iter().enumerate() {
        if let Some(m) = &slot.module {
            out.insert(format!("standard:{}", STANDARD_GROUPS[i]), rec(m));
        }
    }
    out.insert("bulkheads".to_string(), rec(&ship.bulkhead));
    let (mut hp, mut ut) = (0, 0);
    for slot in &ship.hardpoints {
        let key = if slot.kind == SlotKind::Hardpoint {
            hp += 1;
            format!("hardpoint:{hp}")
        } else {
            ut += 1;
            format!("utility:{ut}")
        };
        if let Some(m) = &slot.module {
            out.insert(key, rec(m));
        }
    }
    for (i, slot) in ship.internal.iter().enumerate() {
        if let Some(m) = &slot.module {
            out.insert(format!("internal:{}", i + 1), rec(m));
        }
    }
    Value::Object(out)
}

/// Groups `.modules.<slot>.<attr>` mismatches by module group, so the report names the module type.
fn field_key(field: &str, expect: &Value) -> String {
    if let Some(rest) = field.strip_prefix(".modules.")
        && let Some((slot, attr)) = rest.rsplit_once('.')
    {
        let grp = expect["modules"][slot]["grp"].as_str().unwrap_or("?");
        return format!(".modules[{grp}].{attr}");
    }
    field.to_string()
}

fn actual(ship: &Ship, opponent: &Ship) -> Value {
    let s = &ship.stats;
    let (unladen, full_tank, laden) = ship.coriolis_jump_stats();
    let fin = |x: f64| {
        if x.is_finite() {
            Value::from(x)
        } else {
            Value::Null
        }
    };
    let sh = calc::shield_metrics(ship, 2.0);
    let ar = calc::armour_metrics(ship);
    let vs = calc::sustained_dps(ship, opponent, 2.0, 1000.0);
    let typed = |m: calc::ShieldMults| serde_json::json!({ "generator": m.generator, "boosters": m.boosters, "total": m.total });
    serde_json::json!({
        "dryMass": s.dry_mass, "unladenMass": s.unladen_mass, "ladenMass": s.laden_mass,
        "fuelCapacity": s.fuel_capacity, "cargoCapacity": s.cargo_capacity, "passengerCapacity": s.passenger_capacity,
        "powerAvailable": s.power_available, "powerRetracted": s.power_retracted, "powerDeployed": s.power_deployed,
        "topSpeed": s.top_speed, "topBoost": s.top_boost, "topPitch": s.top_pitch, "topRoll": s.top_roll, "topYaw": s.top_yaw,
        "shield": s.shield, "shieldCells": s.shield_cells, "armour": s.armour,
        "totalDps": s.total_dps, "totalSDps": s.total_sdps, "totalEps": s.total_eps, "totalHps": s.total_hps,
        "totalCost": s.total_cost,
        "unladenRange": unladen, "fullTankRange": full_tank, "ladenRange": laden,
        "canBoost": ship.can_boost(s.cargo_capacity, s.fuel_capacity),
        "shieldMetrics": sh.map(|m| serde_json::json!({
            "generator": m.generator, "boosters": m.boosters, "addition": m.addition, "summary": m.summary,
            "recover": fin(m.recover), "recharge": fin(m.recharge),
            "absolute": typed(m.absolute), "explosive": typed(m.explosive), "kinetic": typed(m.kinetic), "thermal": typed(m.thermal),
        })),
        "armourMetrics": {
            "bulkheads": ar.bulkheads, "reinforcement": ar.reinforcement, "total": ar.total,
            "moduleArmour": ar.module_armour, "moduleProtection": ar.module_protection,
            "explosive": ar.explosive.total, "kinetic": ar.kinetic.total, "thermal": ar.thermal.total, "caustic": ar.caustic.total,
        },
        "vsAnaconda": { "shields": vs.shields.total(), "armour": vs.armour.total(), "eps": vs.eps },
        "timeToDrainWep": fin(calc::time_to_drain_wep(ship, 2.0)),
        "modules": module_records(ship),
    })
}

/// Collects `path: expected vs actual` for every leaf that differs beyond a relative 1e-9.
fn diff(path: &str, want: &Value, got: &Value, out: &mut Vec<(String, String)>) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            for (k, wv) in w {
                diff(
                    &format!("{path}.{k}"),
                    wv,
                    g.get(k).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Number(w), Value::Number(g)) => {
            let (w, g) = (
                w.as_f64().unwrap_or(f64::NAN),
                g.as_f64().unwrap_or(f64::NAN),
            );
            if (w - g).abs() > 1e-9 * w.abs().max(1.0) {
                out.push((path.to_string(), format!("want {w} got {g}")));
            }
        }
        (w, g) if w == g => {}
        (w, g) => out.push((path.to_string(), format!("want {w} got {g}"))),
    }
}

fn load_fixture() -> Fixture {
    // ORACLE_FIXTURE points at a larger, freshly generated fixture for stress runs.
    let text = match std::env::var("ORACLE_FIXTURE") {
        Ok(path) => {
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
        }
        Err(_) => include_str!("fixtures/oracle.json").to_string(),
    };
    serde_json::from_str(&text).expect("fixture parses")
}

fn report(failures: &BTreeMap<String, Vec<String>>) -> String {
    failures
        .iter()
        .map(|(f, v)| {
            format!(
                "{f}: {} mismatches, e.g. {}",
                v.len(),
                v.iter().take(3).cloned().collect::<Vec<_>>().join(" | ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn gunzip_b64(part: &str) -> Vec<u8> {
    use base64::Engine;
    let gz = base64::engine::general_purpose::STANDARD
        .decode(part)
        .expect("base64");
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz.as_slice())
        .read_to_end(&mut out)
        .expect("gzip");
    out
}

#[test]
fn coriolis_codes_round_trip() {
    let fixture = load_fixture();
    let opponent = Ship::stock("anaconda").expect("anaconda");
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut fail = |kind: &str, i: usize, case: &Case, msg: String| {
        failures
            .entry(kind.to_string())
            .or_default()
            .push(format!("case {i} ({}): {msg}", case.ship));
    };
    for (i, case) in fixture.cases.iter().enumerate() {
        // Decoding Coriolis' own code reproduces what Coriolis rebuilds from it.
        let decoded = codec::decode(&case.ship, &case.code).expect("decodes");
        let mut d = Vec::new();
        diff("", &case.decoded, &actual(&decoded, &opponent), &mut d);
        for (field, msg) in d {
            fail(&format!("decode{field}"), i, case, msg);
        }

        // Our encoding: identical module ids, power flags and (uncompressed) engineering record.
        let ship = rebuild(case);
        let ours = codec::encode(&ship);
        let theirs: Vec<&str> = case.code.split('.').collect();
        let mine: Vec<&str> = ours.split('.').collect();
        for (k, name) in ["ids", "enabled", "priorities"].iter().enumerate() {
            if theirs[k] != mine[k] {
                fail(name, i, case, format!("want {} got {}", theirs[k], mine[k]));
            }
        }
        let want = theirs
            .get(3)
            .filter(|p| !p.is_empty())
            .map(|p| gunzip_b64(p));
        let got = codec::encode_modifications(&ship);
        if want != got {
            let (w, g) = (want.clone().unwrap_or_default(), got.unwrap_or_default());
            let at = w
                .iter()
                .zip(&g)
                .position(|(a, b)| a != b)
                .unwrap_or(w.len().min(g.len()));
            let lo = at.saturating_sub(6);
            fail(
                "modifications",
                i,
                case,
                format!(
                    "len {} vs {}, first diff at {at}: want {:?} got {:?}",
                    w.len(),
                    g.len(),
                    &w[lo..(at + 10).min(w.len())],
                    &g[lo..(at + 10).min(g.len())]
                ),
            );
        }

        // And our code decodes back to the same build.
        let again = codec::decode(&case.ship, &ours).expect("own code decodes");
        let mut d = Vec::new();
        diff("", &case.decoded, &actual(&again, &opponent), &mut d);
        for (field, msg) in d {
            fail(&format!("roundtrip{field}"), i, case, msg);
        }
    }
    assert!(failures.is_empty(), "{}", report(&failures));
}

#[test]
fn matches_upstream_engine_on_random_builds() {
    let fixture = load_fixture();
    let opponent = Ship::stock("anaconda").expect("anaconda");
    let mut by_field: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut failed_cases = 0;
    for (i, case) in fixture.cases.iter().enumerate() {
        let ship = rebuild(case);
        let mut d = Vec::new();
        diff("", &case.expect, &actual(&ship, &opponent), &mut d);
        if !d.is_empty() {
            failed_cases += 1;
        }
        for (field, msg) in d {
            by_field
                .entry(field_key(&field, &case.expect))
                .or_default()
                .push(format!("case {i} ({}): {msg}", case.ship));
        }
    }
    assert!(
        by_field.is_empty(),
        "{failed_cases}/{} builds differ\n{}",
        fixture.cases.len(),
        report(&by_field)
    );
}

#[test]
fn old_links_migrate_to_current_slot_layout() {
    let fixture = load_fixture();
    assert!(!fixture.legacy.is_empty(), "fixture has legacy codes");
    let opponent = Ship::stock("anaconda").expect("anaconda");
    let mut by_field: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (i, l) in fixture.legacy.iter().enumerate() {
        let ship = codec::decode(&l.ship, &l.code).expect("legacy code decodes");
        let mut d = Vec::new();
        diff("", &l.decoded, &actual(&ship, &opponent), &mut d);
        for (field, msg) in d {
            by_field
                .entry(field)
                .or_default()
                .push(format!("legacy {i} ({}): {msg}", l.ship));
        }
    }
    assert!(by_field.is_empty(), "{}", report(&by_field));
}
