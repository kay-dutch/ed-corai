//! Coriolis' heat model (4.1.18, the heat rows of coriolis.io's ship summary), ported from
//! `Ship.js`/`Module.js` with JavaScript's numeric semantics (`Math.max`/`Math.min` pass NaN on,
//! `||` treats 0 and NaN alike), so the figures equal what coriolis.io shows.
//!
//! Heat level H follows dH/dt = load - dissipation * H^2 up to H = 1 (equilibrium sqrt(load /
//! dissipation)), then climbs linearly at load - dissipation; coriolis.io displays H / 1.5, so 100 %
//! is H = 1.5, where the ship takes damage.

use corai_engine::jsmath::{num, or, truthy};
use corai_engine::{Module, Ship};

/// `Math.max` / `Math.min`: NaN in, NaN out.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// One heat row of coriolis.io's summary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeatRow {
    /// The hull has no heat data ('-').
    Unavailable,
    /// The base load alone exceeds the hull's dissipation ('∞').
    BaseOverload,
    /// No extra load ('N/A').
    NoLoad,
    /// Displayed heat level reached (0..1, 1 = 100 %).
    Level(f64),
    /// Seconds until the ship overheats.
    Overheats(f64),
}

/// A row with the base load it was computed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatEntry {
    pub row: HeatRow,
    pub base: f64,
}

/// The six heat rows coriolis.io shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatRows {
    pub idle: HeatEntry,
    pub thrust: HeatEntry,
    pub fsd_charge: HeatEntry,
    pub firing_full: HeatEntry,
    pub firing: HeatEntry,
    pub shield_cells: HeatEntry,
}

fn equilibrium(dissipation: f64, load: f64) -> f64 {
    (load / dissipation).sqrt()
}

fn time_until(capacity: f64, dissipation: f64, load: f64, level0: f64, level: f64) -> f64 {
    let d = dissipation / capacity;
    if !truthy(Some(load)) {
        let c = -1.0 / (d * level0);
        return 1.0 / (d * level) + c;
    }
    let l = load / capacity;
    let a_div_b = (d / l).sqrt();
    let a_mul_b = (d * l).sqrt();
    let c = -(level0 * a_div_b).atanh() / a_mul_b;
    (level * a_div_b).atanh() / a_mul_b + c
}

fn level_at(capacity: f64, dissipation: f64, load: f64, level0: f64, seconds: f64) -> f64 {
    let d = dissipation / capacity;
    if !truthy(Some(load)) {
        let c = -1.0 / (d * level0);
        return (1.0 / (seconds - c)) / d;
    }
    let l = load / capacity;
    let a_div_b = (d / l).sqrt();
    let a_mul_b = (d * l).sqrt();
    let c = -(level0 * a_div_b).atanh() / a_mul_b;
    ((seconds - c) * a_mul_b).tanh() / a_div_b
}

/// `Ship.getHeatLevel`.
fn heat_level(ship: &Ship, load: f64, base: f64, duration: Option<f64>) -> HeatRow {
    let props = &ship.def.properties;
    let Some(diss) = props.heat_dissipation.filter(|d| truthy(Some(*d))) else {
        return HeatRow::Unavailable;
    };
    let capacity = props.heat_capacity;
    if base > diss {
        return HeatRow::BaseOverload;
    }
    if load == 0.0 {
        return HeatRow::NoLoad;
    }
    let load = load + base;
    if load <= diss {
        return HeatRow::Level(equilibrium(diss, load) / 1.5);
    }
    let level_base = equilibrium(diss, base);
    let time10 = time_until(capacity, diss, load, level_base, 1.0);
    // Displayed 66 % -> 100 % is actual heat 1.0 -> 1.5, climbing linearly.
    let time15 = (capacity / 2.0) / (load - diss);
    let Some(duration) = duration.filter(|d| truthy(Some(*d))) else {
        return HeatRow::Overheats(time10 + time15);
    };
    if time10 > duration {
        return HeatRow::Level(level_at(capacity, diss, load, level_base, duration) / 1.5);
    }
    if time10 + time15 > duration {
        return HeatRow::Level((2.0 + (duration - time10) / time15) / 3.0);
    }
    HeatRow::Overheats(time10 + time15)
}

/// `Module.getSustainedFiringsPerCycle`.
fn firings_per_cycle(m: &Module) -> f64 {
    let clip = m.get("clip");
    if truthy(clip) {
        let clip = num(clip);
        return if auto_loader(m) {
            clip + clip - 1.0
        } else {
            clip
        };
    }
    or(m.get("burst"), 1.0)
}

fn auto_loader(m: &Module) -> bool {
    m.special().is_some_and(|s| s.def.name == "Auto loader")
}

/// `Module.getSustainedSecondsPerCycle`.
fn seconds_per_cycle(m: &Module) -> f64 {
    let burst_interval = or(m.get("fireint"), 0.0);
    let reload = or(m.get("reload"), 0.0);
    let burst = or(m.get("burst"), 1.0);
    let burst_rof = or(m.get("burstrof"), 1.0);
    let mut charge = or(m.get("chargetime"), 0.0);
    if truthy(m.get("damagemultiplier")) {
        charge *= 0.0;
    }
    if m.def.name.as_deref() == Some("Imperial Hammer") {
        charge = 0.0;
    }
    let mut spc = charge + (burst - 1.0) / burst_rof + burst_interval;
    let clip = m.get("clip");
    if truthy(clip) {
        let mut clip = num(clip);
        if auto_loader(m) {
            clip += clip - 1.0;
        }
        spc *= (clip / burst).ceil();
    }
    spc + js_max(0.0, reload - charge - burst_interval)
}

/// `Ship.getSustainedHardpointEnergyDrawPerSecond`: weapons and utilities, reloads included.
fn sustained_energy_draw(ship: &Ship) -> f64 {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
        .map(|m| {
            let eps = m.eps();
            match m.clip().filter(|c| truthy(Some(*c))) {
                Some(clip) => {
                    let rof = m.rof();
                    (clip * eps / rof) / (clip / rof + num(m.reload()))
                }
                None => eps,
            }
        })
        .sum()
}

/// `Ship.getEffectiveWeaponThermalLoad`: weapons heat up to 5x as the capacitor runs dry.
fn effective_weapon_load(load: f64, distdraw: f64, capacitor: f64, wep_level: f64) -> f64 {
    load * (1.0
        + 4.0
            * js_min(
                js_max(1.0 - (capacitor * wep_level - distdraw) / capacitor, 0.0),
                1.0,
            ))
}

/// `Ship.getWeaponsThermalLoad`: (capacitor full, capacitor near empty) at `wep` pips.
fn weapons_load(ship: &Ship, wep: f64) -> (f64, f64) {
    let pd = ship.power_distributor();
    let recharge = pd.map_or(f64::NAN, |m| num(m.get("weprate")));
    let capacitor = pd.map_or(f64::NAN, |m| num(m.get("wepcap")));
    let draw = sustained_energy_draw(ship);
    // Weapons that draw no capacitor energy (mines, missiles) fire at full rate: Coriolis' formula
    // gets that at any WEP pips but 0, where it divides 0 by 0.
    let fire_rate = if draw == 0.0 {
        1.0
    } else {
        js_min(js_max(recharge * (wep / 4.0).powf(1.1) / draw, 0.0), 1.0)
    };
    let (mut full, mut empty) = (0.0, 0.0);
    for slot in ship
        .hardpoints
        .iter()
        .filter(|s| s.enabled && s.max_class > 0)
    {
        let Some(m) = slot.module.as_ref() else {
            continue;
        };
        let spc = or(Some(seconds_per_cycle(m)), 1.0);
        let load = num(m.get("thermload")) * (firings_per_cycle(m) / spc);
        let distdraw = num(m.get("distdraw"));
        full += effective_weapon_load(load, distdraw, capacitor, 1.0);
        empty += effective_weapon_load(load, distdraw, capacitor, 0.0);
    }
    (full * fire_rate, empty * fire_rate)
}

/// `Ship.getShieldBankCellStats`: (heat per second while spinning up, longest spin-up).
fn shield_cells(ship: &Ship) -> (f64, f64) {
    let (mut load, mut spinup) = (0.0, 0.0);
    for m in ship
        .internal
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.def.grp == "scb")
    {
        let s = num(m.get("spinup"));
        load += num(m.get("thermload")) / s;
        spinup = js_max(spinup, s);
    }
    (load, spinup)
}

/// `Ship.getPowerPlantThermalEfficiency`: Armoured never makes the plant hotter (game bug kept).
fn power_plant_efficiency(ship: &Ship) -> f64 {
    let Some(pp) = ship.standard[0].module.as_ref() else {
        return f64::NAN;
    };
    let modified = num(pp.get("eff"));
    if pp
        .blueprint()
        .is_some_and(|b| b.def.fdname == "PowerPlant_Armoured")
    {
        return js_max(modified, num(pp.base("eff")));
    }
    modified
}

/// The heat rows as coriolis.io adds the loads: modules through the power plant's efficiency,
/// thrusters at full thrust, the FSD charging, weapons at `wep` pips, shield cells spinning up.
pub fn heat_rows(ship: &Ship, wep: f64) -> HeatRows {
    let pp = power_plant_efficiency(ship);
    let [draw_retracted, draw_deployed] = ship.stats.power_draw;
    let retracted = draw_retracted * pp;
    let deployed = draw_deployed * pp;
    let thrusters = or(ship.thrusters().and_then(|m| m.get("thermload")), 0.0);
    let fsd = or(ship.fsd().and_then(|m| m.get("thermload")), 0.0);
    // coriolis.io hides the weapon rows on an unarmed ship (where its formula divides 0 by 0 at
    // 0 WEP pips); unarmed means no weapon heat.
    let armed = ship
        .hardpoints
        .iter()
        .any(|s| s.max_class > 0 && s.module.is_some());
    let (full, empty) = if armed {
        weapons_load(ship, wep)
    } else {
        (0.0, 0.0)
    };
    let (cells, spinup) = shield_cells(ship);
    let entry = |load: f64, base: f64, duration: Option<f64>| HeatEntry {
        row: heat_level(ship, load, base, duration),
        base,
    };
    HeatRows {
        idle: entry(retracted, 0.0, None),
        thrust: entry(thrusters, retracted, None),
        fsd_charge: entry(fsd, retracted + thrusters, None),
        firing_full: entry(full, deployed + thrusters, None),
        firing: entry(empty, deployed + thrusters, None),
        shield_cells: entry(cells, deployed + thrusters, Some(spinup)),
    }
}

/// A row as a displayed percentage: the level reached (100 = overheats); without an extra load,
/// the level of the base load alone; NaN without heat data.
pub fn percent(ship: &Ship, e: HeatEntry) -> f64 {
    let Some(diss) = ship
        .def
        .properties
        .heat_dissipation
        .filter(|d| truthy(Some(*d)))
    else {
        return f64::NAN;
    };
    match e.row {
        HeatRow::Unavailable => f64::NAN,
        HeatRow::Level(x) => x * 100.0,
        HeatRow::NoLoad => equilibrium(diss, e.base) / 1.5 * 100.0,
        HeatRow::BaseOverload | HeatRow::Overheats(_) => 100.0,
    }
}

/// Seconds until a row overheats; `never` when it does not; NaN without heat data.
pub fn time_to_overheat(e: HeatEntry, never: f64) -> f64 {
    match e.row {
        HeatRow::Unavailable => f64::NAN,
        HeatRow::Overheats(t) => t,
        HeatRow::BaseOverload => 0.0,
        HeatRow::Level(_) | HeatRow::NoLoad => never,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_min_and_max_pass_nan_on() {
        assert!(js_max(f64::NAN, 0.0).is_nan());
        assert!(js_min(1.0, f64::NAN).is_nan());
        assert_eq!(js_max(-1.0, 0.0), 0.0);
    }

    #[test]
    fn heat_climbs_to_equilibrium_and_the_closed_forms_agree() {
        // Below the hull's maximum dissipation the level settles at sqrt(load / dissipation), and
        // the time to a level and the level at that time invert each other.
        let (cap, diss, load) = (300.0, 52.05, 30.0);
        let eq = equilibrium(diss, load);
        let t = time_until(cap, diss, load, 0.2, 0.5);
        assert!((level_at(cap, diss, load, 0.2, t) - 0.5).abs() < 1e-9);
        assert!(level_at(cap, diss, load, 0.2, 1e4) <= eq + 1e-9);
    }
}
