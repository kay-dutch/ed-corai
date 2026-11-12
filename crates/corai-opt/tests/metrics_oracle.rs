//! Differential test of every metric against the Node implementation (`npm run oracle` writes
//! `tests/fixtures/metrics.json`): stock and random builds, three metric contexts.

use std::collections::BTreeMap;

use corai_engine::{Ship, codec};
use corai_opt::metrics::{ContextOptions, DamageProfile, Metric, MetricContext};
use serde_json::Value;

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/metrics.json");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&text).expect("fixture parses")
}

/// Rebuilds a context from the fixture's plain-data description (`makeContext` options).
fn context(spec: &Value) -> MetricContext {
    let f = |k: &str| spec.get(k).and_then(Value::as_f64);
    let damage = spec.get("damage").map(|d| DamageProfile {
        kinetic: d["kinetic"].as_f64().unwrap_or(0.0),
        thermal: d["thermal"].as_f64().unwrap_or(0.0),
        explosive: d["explosive"].as_f64().unwrap_or(0.0),
        absolute: d["absolute"].as_f64().unwrap_or(0.0),
    });
    let opponent = spec
        .get("opponent")
        .and_then(Value::as_str)
        .map(|id| Ship::stock(id).expect("opponent ship"));
    MetricContext::from_options(ContextOptions {
        sys: f("sys"),
        wep: f("wep"),
        recovery_sys: f("recoverySys"),
        damage_preset: spec
            .get("damagePreset")
            .and_then(Value::as_str)
            .map(str::to_string),
        damage,
        opponent,
        opponent_sys: f("opponentSys"),
        range: f("range"),
        incoming_dps: f("incomingDps"),
        regen_under_fire: f("regenUnderFire"),
        reserve_fuel_in_jump_mass: spec.get("reserveFuelInJumpMass").and_then(Value::as_bool),
        vent_hit_rate: f("ventHitRate"),
        thermal_vent_factor: f("thermalVentFactor"),
        corrosive_hull_multiplier: f("corrosiveHullMultiplier"),
        corrosive_piercing_bonus: f("corrosivePiercingBonus"),
    })
    .expect("context options are valid")
}

fn close(want: f64, got: f64) -> bool {
    (want - got).abs() <= 1e-9 * want.abs().max(1.0)
}

#[test]
fn every_metric_matches_node_in_every_context() {
    let fx = fixture();
    let contexts: BTreeMap<String, MetricContext> = fx["contexts"]
        .as_object()
        .expect("contexts")
        .iter()
        .map(|(k, v)| (k.clone(), context(v)))
        .collect();
    let names: Vec<&str> = fx["metrics"]
        .as_array()
        .expect("metric names")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let metrics: Vec<(&str, Option<Metric>)> = names.iter().map(|n| (*n, n.parse().ok())).collect();
    let missing: Vec<&str> = metrics
        .iter()
        .filter(|(_, m)| m.is_none())
        .map(|(n, _)| *n)
        .collect();

    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let cases = fx["cases"].as_array().expect("cases");
    for (i, case) in cases.iter().enumerate() {
        let ship_id = case["ship"].as_str().expect("ship");
        let ship =
            codec::decode(ship_id, case["code"].as_str().expect("code")).expect("code decodes");
        for (ctx_name, ctx) in &contexts {
            let want = &case["values"][ctx_name];
            for (name, metric) in &metrics {
                let Some(metric) = metric else { continue };
                let got = metric.compute(&ship, ctx);
                let ok = match want[*name].as_f64() {
                    Some(w) => close(w, got),
                    None => !got.is_finite(),
                };
                if !ok {
                    failures
                        .entry(format!("{name} [{ctx_name}]"))
                        .or_default()
                        .push(format!(
                            "case {i} ({ship_id}): want {} got {got}",
                            want[*name]
                        ));
                }
            }
        }
    }
    let report: Vec<String> = failures
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}: {} mismatches, e.g. {}",
                v.len(),
                v.iter().take(2).cloned().collect::<Vec<_>>().join(" | ")
            )
        })
        .collect();
    assert!(
        missing.is_empty() && failures.is_empty(),
        "metrics not implemented: {missing:?}\n{}",
        report.join("\n")
    );
}
