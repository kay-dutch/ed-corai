#![allow(clippy::print_stdout, reason = "a benchmark reports to stdout")]
//! Evaluation throughput (run with --release):
//! - engineer: place a module, apply a blueprint, recompute the ship, read defence metrics
//! - reuse: swap in an already-engineered module (its stat cache warm), recompute, read metrics
use std::hint::black_box;
use std::time::Instant;

use corai_engine::{Module, Ship, calc};

fn main() {
    let n: usize = std::env::var("N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);
    let mut ship = Ship::stock("fer_de_lance").expect("ship");
    let slot = ship
        .internal
        .iter()
        .position(|s| s.max_class >= 5)
        .expect("class 5 slot");
    let defs: Vec<_> = corai_data::DATA.modules.internal["sg"]
        .iter()
        .filter(|m| m.class <= 5)
        .collect();

    let t = Instant::now();
    for i in 0..n {
        Ship::place(&mut ship.internal[slot], Some(defs[i % defs.len()]));
        if let Some(m) = ship.internal[slot].module.as_mut() {
            let _ = m.apply_blueprint("ShieldGenerator_Reinforced", 5, 1.0);
        }
        ship.recalc();
        black_box(calc::shield_metrics(&ship, 2.0));
        black_box(calc::armour_metrics(&ship));
    }
    report("engineer", t, n);

    let engineered: Vec<Module> = defs
        .iter()
        .map(|d| {
            let mut m = Module::new(d);
            let _ = m.apply_blueprint("ShieldGenerator_Reinforced", 5, 1.0);
            black_box(m.r());
            m
        })
        .collect();
    let t = Instant::now();
    for i in 0..n {
        ship.internal[slot].module = Some(engineered[i % engineered.len()].clone());
        ship.recalc();
        black_box(calc::shield_metrics(&ship, 2.0));
        black_box(calc::armour_metrics(&ship));
    }
    report("reuse", t, n);
}

fn report(name: &str, t: Instant, n: usize) {
    println!(
        "{name:>9}: {:.2} µs/eval",
        t.elapsed().as_secs_f64() / n as f64 * 1e6
    );
}
