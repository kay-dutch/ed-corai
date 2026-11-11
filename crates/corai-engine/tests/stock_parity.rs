//! Stock-build values captured from the upstream Coriolis engine (see git history of the TS oracle).

use corai_engine::Ship;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn stock_python_matches_coriolis() {
    let ship = Ship::stock("python").expect("python exists");
    let s = &ship.stats;
    let (unladen, full_tank, laden) = ship.coriolis_jump_stats();
    let checks = [
        ("unladenMass", s.unladen_mass, 696.0),
        ("ladenMass", s.laden_mass, 778.0),
        ("cargoCapacity", s.cargo_capacity, 82.0),
        ("fuelCapacity", s.fuel_capacity, 32.0),
        ("unladenRange", unladen, 8.205012361270894),
        ("fullTankRange", full_tank, 7.881913895553958),
        ("ladenRange", laden, 7.375903989954921),
        ("topSpeed", s.top_speed, 234.22008165685364),
        ("topBoost", s.top_boost, 305.5044543350265),
        ("shield", s.shield, 293.9938844735135),
        ("armour", s.armour, 468.0),
        ("powerAvailable", s.power_available, 20.0),
        ("powerRetracted", s.power_retracted, 9.31),
        ("powerDeployed", s.power_deployed, 10.09),
        ("totalDps", s.total_dps, 15.769230769230766),
        ("totalCost", s.total_cost, 56978179.0),
    ];
    let failures: Vec<String> = checks
        .iter()
        .filter(|(_, got, want)| !close(*got, *want))
        .map(|(name, got, want)| format!("{name}: got {got}, want {want}"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_ship_has_a_buildable_stock_fit() {
    let data = &*corai_data::DATA;
    for id in data.ships.keys() {
        let ship = Ship::stock(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(ship.stats.top_speed > 0.0, "{id} cannot move");
        let (unladen, _, _) = ship.coriolis_jump_stats();
        assert!(unladen > 0.0, "{id} cannot jump");
    }
}
