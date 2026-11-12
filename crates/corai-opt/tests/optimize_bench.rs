//! Optimiser benchmark against the Node version (`tests/fixtures/optimize.json`): for every
//! scenario the Rust search must find a legal build at least as good on the objective, and feasible
//! whenever Node's was. Timings are printed for comparison (`--nocapture`).

// The comparison table is this test's report.
#![allow(clippy::print_stdout)]

use std::time::{Duration, Instant};

use corai_engine::{Ship, codec};
use corai_opt::catalog::SlotRef;
use corai_opt::metrics::{ContextOptions, DamageProfile, Metric, MetricContext};
use corai_opt::search::{Constraint, Direction, Effort, Goal, PowerCheck, Request, optimize};
use corai_opt::validate::validate;
use serde_json::Value;

fn context(spec: &Value) -> MetricContext {
    let f = |k: &str| spec.get(k).and_then(Value::as_f64);
    MetricContext::from_options(ContextOptions {
        sys: f("sys"),
        wep: f("wep"),
        recovery_sys: f("recoverySys"),
        damage_preset: spec
            .get("damagePreset")
            .and_then(Value::as_str)
            .map(str::to_string),
        damage: spec.get("damage").map(|d| DamageProfile {
            kinetic: d["kinetic"].as_f64().unwrap_or(0.0),
            thermal: d["thermal"].as_f64().unwrap_or(0.0),
            explosive: d["explosive"].as_f64().unwrap_or(0.0),
            absolute: d["absolute"].as_f64().unwrap_or(0.0),
        }),
        incoming_dps: f("incomingDps"),
        range: f("rangeMeters"),
        ..ContextOptions::default()
    })
    .expect("valid context")
}

fn metric(name: &str) -> Metric {
    name.parse().unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn rust_matches_or_beats_node_on_every_scenario() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/optimize.json");
    let fx: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses");
    let mut failures = Vec::new();
    for s in fx["scenarios"].as_array().expect("scenarios") {
        let name = s["name"].as_str().expect("name");
        let ship_id = s["ship"].as_str().expect("ship");
        let base = match s.get("code").and_then(Value::as_str) {
            Some(code) => codec::decode(ship_id, code).expect("base decodes"),
            None => Ship::stock(ship_id).expect("ship"),
        };
        let ctx = context(s.get("context").unwrap_or(&Value::Null));
        let objective_metric = metric(s["objective"]["metric"].as_str().expect("objective"));
        let mut req = Request::new(base.clone(), Goal::natural(objective_metric));
        req.constraints = s["constraints"]
            .as_array()
            .map(|cs| {
                cs.iter()
                    .map(|c| Constraint {
                        metric: metric(c["metric"].as_str().expect("metric")),
                        min: c["min"].as_f64(),
                        max: c["max"].as_f64(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rules: Vec<String> = s["lock"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        req.lock = SlotRef::locked_by(&base, &rules);
        req.keep_base = s["keepBase"].as_bool().unwrap_or(false);
        req.power = match s["power"].as_str() {
            Some("deployed") => PowerCheck::Deployed,
            Some("retracted") => PowerCheck::Retracted,
            Some("ignore") => PowerCheck::Ignore,
            _ => PowerCheck::Priority,
        };
        req.effort = Effort::Normal;
        req.time_limit = Duration::from_secs(300);

        let t0 = Instant::now();
        let out = optimize(&req, &ctx);
        let ms = t0.elapsed().as_millis();
        let ours = objective_metric.compute(&out.ship, &ctx);
        let node = &s["node"];
        let theirs = node["objective"].as_f64().expect("node objective");
        println!(
            "{name:40} rust {ours:>12.3} ({ms:>5} ms)   node {theirs:>12.3} ({:>5} ms)",
            node["ms"]
        );
        let tol = 1e-6 * theirs.abs().max(1.0);
        let worse = match Goal::natural(objective_metric).direction {
            Direction::Max => ours < theirs - tol,
            Direction::Min => ours > theirs + tol,
        };
        if worse {
            failures.push(format!(
                "{name}: objective {ours} worse than Node's {theirs}"
            ));
        }
        if node["feasible"].as_bool() == Some(true) && !out.feasible {
            failures.push(format!(
                "{name}: infeasible where Node was feasible: {:?}",
                out.violations
            ));
        }
        let illegal = validate(&out.ship);
        if !illegal.is_empty() {
            failures.push(format!("{name}: illegal build {illegal:?}"));
        }
        for r in &req.lock {
            let before = r
                .slot(&base)
                .and_then(|s| s.module.as_ref())
                .map(|m| m.def.id.clone());
            let after = r
                .slot(&out.ship)
                .and_then(|s| s.module.as_ref())
                .map(|m| m.def.id.clone());
            if before != after {
                failures.push(format!("{name}: locked slot {} changed", r.key(&base)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
