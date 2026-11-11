//! Port of Coriolis' `Calculations.js`: jump range, mass curves, shields, armour, sustained DPS.

use crate::jsmath::{num, truthy};
use crate::module::{Curve, Module};
use crate::ship::{Ship, SlotKind};

/// Single jump range (Ly) for a ship of `mass` burning `fuel` (default: the FSD's max per jump).
/// Adds the ship's reserve fuel and any powered Guardian FSD booster, like Coriolis.
pub fn jump_range(ship: &Ship, mass: f64, fsd: &Module, fuel: Option<f64>) -> f64 {
    let max_fuel = num(fsd.r().maxfuel);
    let opt_mass = num(fsd.r().optmass);
    let mass = mass + ship.def.properties.reserve_fuel_capacity;
    let boost: f64 = ship
        .internal
        .iter()
        .filter(|s| {
            s.module.as_ref().is_some_and(|m| m.grp() == "gfsb") && ship.slot_status(s, false) == 3
        })
        .filter_map(|s| s.module.as_ref().map(|m| num(m.r().jumpboost)))
        .sum();
    let fuelmul = num(fsd.r().fuelmul);
    let fuelpower = num(fsd.r().fuelpower);
    (fuel.unwrap_or(max_fuel).min(max_fuel) / fuelmul).powf(1.0 / fuelpower) * opt_mass / mass
        + boost
}

/// Total distance covered by consecutive jumps until `fuel` is used up.
pub fn total_jump_range(ship: &Ship, mut mass: f64, fsd: &Module, fuel: f64) -> f64 {
    let max_fuel = num(fsd.r().maxfuel);
    let mut remaining = fuel;
    let mut total = 0.0;
    while remaining > 0.0 {
        let burn = remaining.min(max_fuel);
        if burn <= 0.0 {
            break;
        }
        total += jump_range(ship, mass, fsd, Some(burn));
        mass -= burn;
        remaining -= burn;
    }
    total
}

/// Frontier's mass-curve multiplier (thrusters, shields).
pub fn mass_curve(
    min_mass: f64,
    opt_mass: f64,
    max_mass: f64,
    min_mul: f64,
    opt_mul: f64,
    max_mul: f64,
    mass: f64,
) -> f64 {
    let xnorm = ((max_mass - mass) / (max_mass - min_mass)).min(1.0);
    let exponent = ((opt_mul - min_mul) / (max_mul - min_mul)).ln()
        / ((max_mass - opt_mass) / (max_mass - min_mass))
            .min(1.0)
            .ln();
    let ynorm = xnorm.powf(exponent);
    min_mul + ynorm * (max_mul - min_mul)
}

fn curve(m: &Module, kind: Curve, mass: f64) -> f64 {
    let r = m.r();
    let [min_mul, opt_mul, max_mul] = r.muls[kind as usize];
    mass_curve(
        num(r.min_mass),
        num(r.optmass),
        num(r.max_mass),
        min_mul,
        opt_mul,
        max_mul,
        mass,
    )
}

/// Shield generator strength (MJ) for a hull mass.
pub fn shield_strength(hull_mass: f64, base_shield: f64, sg: &Module, multiplier: f64) -> f64 {
    base_shield * curve(sg, Curve::Generic, hull_mass) * multiplier
}

/// Speeds for 0..=4 ENG pips.
pub fn speed(mass: f64, base_speed: f64, thrusters: Option<&Module>, minthrust: f64) -> [f64; 5] {
    let Some(t) = thrusters else { return [0.0; 5] };
    let mul = curve(t, Curve::Speed, mass);
    let min_pct = minthrust / 100.0;
    std::array::from_fn(|eng| {
        let eng_mul = eng as f64 / 4.0;
        mul * base_speed * (eng_mul + min_pct * (1.0 - eng_mul))
    })
}

/// Pitch / yaw / roll rates for 0..=4 ENG pips.
pub fn rotation(mass: f64, base: f64, thrusters: Option<&Module>, engpip: f64) -> [f64; 5] {
    let Some(t) = thrusters else { return [0.0; 5] };
    let res = base * curve(t, Curve::Rotation, mass);
    [
        res * (1.0 - engpip * 4.0),
        res * (1.0 - engpip * 3.0),
        res * (1.0 - engpip * 2.0),
        res * (1.0 - engpip),
        res,
    ]
}

pub fn sys_resistance(sys: f64) -> f64 {
    sys.powf(0.85) * 0.6 / 4f64.powf(0.85)
}

pub fn sys_recharge_rate(pd: Option<&Module>, sys: f64) -> f64 {
    pd.map_or(0.0, |pd| {
        num(pd.r().sysrate) * sys.powf(1.1) / 4f64.powf(1.1)
    })
}

pub fn wep_recharge_rate(pd: Option<&Module>, wep: f64) -> f64 {
    pd.map_or(0.0, |pd| num(pd.r().weprate) * (wep / 4.0).powf(1.1))
}

pub fn map_into_diminishing_range(min: f64, max: f64, now: f64) -> f64 {
    min + (max - min) * (now / max)
}

pub fn diminishing_returns_shields(shield_mult: f64, combined_mult: f64) -> f64 {
    let max = shield_mult * 0.7;
    if combined_mult < max {
        map_into_diminishing_range(max / 2.0, max, combined_mult)
    } else {
        combined_mult
    }
}

/// `stackDamageResistance`: one module's resistance folded onto a running armour resistance.
/// Past 30 % the stack is mapped into [max(30 %, base), 65 %] (half credit).
pub fn stack_damage_resistance(base: f64, module: f64) -> f64 {
    const MIN_LOWER_BOUND: f64 = 0.30;
    const UPPER_BOUND: f64 = 0.65;
    let lower = MIN_LOWER_BOUND.max(base);
    let stacked = 1.0 - (1.0 - base) * (1.0 - module);
    let capped = lower + (stacked - lower) / (1.0 - lower) * (UPPER_BOUND - lower);
    if capped >= MIN_LOWER_BOUND {
        capped
    } else {
        stacked
    }
}

/// `stackArmourResistances`: bulkhead resistance plus hull reinforcement resistances, folded in
/// ascending order; a module above 30 % counts its excess twice; the result caps at 75 %.
pub fn stack_armour_resistances(base: f64, modules: &[f64]) -> f64 {
    let mut sorted = modules.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut res = base;
    for m in sorted {
        let adapted = if m > 0.3 { m * 2.0 - 0.3 } else { m };
        res = stack_damage_resistance(res, adapted);
    }
    res.min(0.75)
}

pub fn diminishing_returns_armour(mults: &[f64]) -> f64 {
    let max = mults.iter().copied().fold(0.7, f64::min);
    let combined: f64 = mults.iter().product();
    let diminished = map_into_diminishing_range(0.35, max, combined);
    if diminished < 0.7 {
        diminished
    } else {
        combined
    }
}

/// Damage multipliers (1 = full damage) a defence applies to one damage type.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShieldMults {
    pub generator: f64,
    pub boosters: f64,
    pub sys: f64,
    pub base: f64,
    pub total: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShieldMetrics {
    pub generator: f64,
    pub boosters: f64,
    pub addition: f64,
    pub cells: f64,
    /// generator + boosters + guardian shield reinforcement.
    pub summary: f64,
    pub total: f64,
    /// Seconds from broken to 50% (NaN where Coriolis yields `undefined`).
    pub recover: f64,
    /// Seconds from 50% to 100%.
    pub recharge: f64,
    pub absolute: ShieldMults,
    pub explosive: ShieldMults,
    pub kinetic: ShieldMults,
    pub thermal: ShieldMults,
}

/// `shieldMetrics(ship, sys)`; `None` when no enabled, working shield generator is fitted.
pub fn shield_metrics(ship: &Ship, sys: f64) -> Option<ShieldMetrics> {
    let sys_res = sys_resistance(sys);
    let max_sys_res = sys_resistance(4.0);
    let slot = ship.find_internal_by_group("sg").filter(|s| s.enabled)?;
    let sg = slot.module.as_ref()?;

    let mut boost = 1.0;
    let (mut b_expl, mut b_kin, mut b_therm) = (1.0, 1.0, 1.0);
    for m in ship
        .hardpoints
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.grp() == "sb")
    {
        boost += num(m.r().shieldboost);
        b_expl *= 1.0 - num(m.r().explres);
        b_kin *= 1.0 - num(m.r().kinres);
        b_therm *= 1.0 - num(m.r().thermres);
    }
    boost -= 1.0;

    let addition: f64 = ship
        .internal
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.grp() == "gsrp")
        .map(|m| num(m.r().shieldaddition))
        .sum();
    let p = &ship.def.properties;
    let generator = shield_strength(p.hull_mass, p.base_shield_strength, sg, 1.0);
    // A generator rated below the hull mass yields NaN; Coriolis then treats the ship as unshielded.
    if !truthy(Some(generator)) {
        return None;
    }
    let boosters = generator * boost;
    let summary = generator + boosters + addition;

    let mut out = ShieldMetrics {
        generator,
        boosters,
        addition,
        cells: ship.stats.shield_cells,
        summary,
        total: summary + ship.stats.shield_cells,
        recover: f64::INFINITY,
        recharge: f64::INFINITY,
        ..ShieldMetrics::default()
    };

    if let Some(pd) = ship.standard[4].module.as_ref() {
        let sys_rate = sys_recharge_rate(Some(pd), sys);
        let distdraw = num(sg.r().distdraw);
        let broken = num(sg.r().brokenregen);
        let regen = num(sg.r().regen);
        let syscap = num(pd.r().syscap);
        let half = summary / 2.0;

        let drain = broken * distdraw - sys_rate;
        let lifetime = syscap / drain;
        let mut recover = 16.0;
        if drain <= 0.0 || half < lifetime * broken {
            recover += half / broken;
        } else {
            recover += lifetime;
            let rest = half - lifetime * broken;
            // Coriolis assigns `Math.Infinity` (undefined) here.
            recover = if sys == 0.0 {
                f64::NAN
            } else {
                recover + rest / (sys_rate / distdraw)
            };
        }

        let drain = regen * distdraw - sys_rate;
        let lifetime = syscap / drain;
        let mut recharge = 0.0;
        if drain <= 0.0 || half < lifetime * regen {
            recharge += half / regen;
        } else {
            recharge += lifetime;
            let rest = half - lifetime * regen;
            recharge = if sys == 0.0 {
                f64::NAN
            } else {
                recharge + rest / (sys_rate / distdraw)
            };
        }
        out.recover = recover;
        out.recharge = recharge;
    }

    out.absolute = ShieldMults {
        generator: 1.0,
        boosters: 1.0,
        sys: 1.0 - sys_res,
        base: 1.0,
        total: 1.0 - sys_res,
        max: 1.0 - max_sys_res,
    };
    let typed = |res: f64, booster: f64| {
        let g = 1.0 - res;
        let gb = diminishing_returns_shields(g, g * booster);
        ShieldMults {
            generator: g,
            boosters: gb / g,
            sys: 1.0 - sys_res,
            base: gb,
            total: gb * (1.0 - sys_res),
            max: gb * (1.0 - max_sys_res),
        }
    };
    out.explosive = typed(num(sg.r().explres), b_expl);
    out.kinetic = typed(num(sg.r().kinres), b_kin);
    out.thermal = typed(num(sg.r().thermres), b_therm);
    Some(out)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArmourMults {
    pub bulkheads: f64,
    pub reinforcement: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArmourMetrics {
    pub bulkheads: f64,
    pub reinforcement: f64,
    pub module_armour: f64,
    pub module_protection: f64,
    pub total: f64,
    pub absolute: ArmourMults,
    pub explosive: ArmourMults,
    pub kinetic: ArmourMults,
    pub thermal: ArmourMults,
    pub caustic: ArmourMults,
}

pub fn armour_metrics(ship: &Ship) -> ArmourMetrics {
    let base = ship.def.properties.base_armour;
    let bh = &ship.bulkhead;
    let bulkheads = base + base * num(bh.r().hullboost);
    let mut reinforcement = 0.0;
    let mut module_armour = 0.0;
    let mut protection = 1.0;
    let (mut expl, mut kin, mut therm, mut caus) = (vec![], vec![], vec![], vec![]);
    for m in ship
        .internal
        .iter()
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
    {
        match m.grp() {
            "hr" | "ghrp" | "mahr" => {
                reinforcement += num(m.r().hullreinforcement);
                reinforcement += base * num(m.r().hullboost_mod) / 10000.0;
                expl.push(num(m.r().explres));
                kin.push(num(m.r().kinres));
                therm.push(num(m.r().thermres));
                caus.push(num(m.r().causres));
            }
            "mrp" | "gmrp" => {
                module_armour += num(m.r().integrity);
                protection *= 1.0 - num(m.r().protection);
            }
            _ => {}
        }
    }
    // Bulkhead and reinforcement resistances combine with the game's sequential stacking and are
    // reported as damage multipliers (1 - resistance).
    let typed = |res: f64, others: &[f64]| {
        let b = 1.0 - res;
        let total = 1.0 - stack_armour_resistances(res, others);
        ArmourMults {
            bulkheads: b,
            reinforcement: if b == 0.0 { 1.0 } else { total / b },
            total,
        }
    };
    ArmourMetrics {
        bulkheads,
        reinforcement,
        module_armour,
        module_protection: 1.0 - protection,
        total: bulkheads + reinforcement,
        absolute: ArmourMults {
            bulkheads: 1.0,
            reinforcement: 1.0,
            total: 1.0,
        },
        explosive: typed(num(bh.r().explres), &expl),
        kinetic: typed(num(bh.r().kinres), &kin),
        thermal: typed(num(bh.r().thermres), &therm),
        caustic: typed(num(bh.r().causres), &caus),
    }
}

/// Damage per second split by type.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TypedDps {
    pub absolute: f64,
    pub explosive: f64,
    pub kinetic: f64,
    pub thermal: f64,
}

impl TypedDps {
    pub fn total(&self) -> f64 {
        self.absolute + self.explosive + self.kinetic + self.thermal
    }

    fn add(&mut self, o: &TypedDps) {
        self.absolute += o.absolute;
        self.explosive += o.explosive;
        self.kinetic += o.kinetic;
        self.thermal += o.thermal;
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeaponDamage {
    pub eps: f64,
    pub shields: TypedDps,
    pub armour: TypedDps,
}

/// Sustained damage of one weapon against an opponent at a range (falloff, resistances, hardness).
pub fn weapon_sustained_dps(
    m: &Module,
    opponent: &Ship,
    shields: Option<&ShieldMetrics>,
    armour: &ArmourMetrics,
    range: f64,
) -> WeaponDamage {
    let clip = m.r().clip.filter(|c| *c != 0.0);
    let eps = match clip {
        Some(c) => (c * m.r().eps / m.r().rof) / (c / m.r().rof + num(m.r().reload)),
        None => m.r().eps,
    };
    let mut sdps = m.r().sdps;
    if let Some(falloff) = m.r().falloff.filter(|f| *f != 0.0)
        && range > falloff
    {
        let dropoff = 1.0 - ((range - falloff) / (num(m.r().range) - falloff)).min(1.0);
        sdps *= dropoff;
    }
    let dist = m.r().damage_dist;
    let piercing = num(m.r().piercing);
    let hardness = opponent.def.properties.hardness;
    let armour_multiple = if piercing >= hardness {
        1.0
    } else {
        piercing / hardness
    };
    let shield_mult = |f: fn(&ShieldMetrics) -> ShieldMults| shields.map_or(1.0, |s| f(s).total);
    let mut out = WeaponDamage {
        eps,
        ..WeaponDamage::default()
    };
    if truthy(Some(dist.absolute)) {
        out.shields.absolute += sdps * dist.absolute * shield_mult(|s| s.absolute);
        out.armour.absolute += sdps * dist.absolute * armour_multiple * armour.absolute.total;
    }
    if truthy(Some(dist.explosive)) {
        out.shields.explosive += sdps * dist.explosive * shield_mult(|s| s.explosive);
        out.armour.explosive += sdps * dist.explosive * armour_multiple * armour.explosive.total;
    }
    if truthy(Some(dist.kinetic)) {
        out.shields.kinetic += sdps * dist.kinetic * shield_mult(|s| s.kinetic);
        out.armour.kinetic += sdps * dist.kinetic * armour_multiple * armour.kinetic.total;
    }
    if truthy(Some(dist.thermal)) {
        out.shields.thermal += sdps * dist.thermal * shield_mult(|s| s.thermal);
        out.armour.thermal += sdps * dist.thermal * armour_multiple * armour.thermal.total;
    }
    out
}

/// `sustainedDps(ship, opponent, sys, range)`: total sustained damage against the opponent's
/// shields (at `sys` pips) and hull.
pub fn sustained_dps(ship: &Ship, opponent: &Ship, sys: f64, range: f64) -> WeaponDamage {
    let shields = shield_metrics(opponent, sys);
    let armour = armour_metrics(opponent);
    let mut total = WeaponDamage::default();
    for slot in ship
        .hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
    {
        if let Some(m) = &slot.module {
            let w = weapon_sustained_dps(m, opponent, shields.as_ref(), &armour, range);
            total.shields.add(&w.shields);
            total.armour.add(&w.armour);
            total.eps += w.eps;
        }
    }
    total
}

/// Seconds of continuous fire before the WEP capacitor is empty (`INFINITY` if sustainable).
pub fn time_to_drain_wep(ship: &Ship, wep: f64) -> f64 {
    let mut seps = 0.0;
    for slot in ship
        .hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
    {
        let Some(m) = &slot.module else { continue };
        if !truthy(Some(m.r().dps)) {
            continue;
        }
        seps += match m.r().clip.filter(|c| *c != 0.0) {
            Some(c) => (c * m.r().eps / m.r().rof) / (c / m.r().rof + num(m.r().reload)),
            None => m.r().eps,
        };
    }
    let Some(pd) = ship.standard[4].module.as_ref() else {
        return f64::INFINITY;
    };
    let drain = seps - num(pd.r().weprate) * (wep / 4.0).powf(1.1);
    if drain <= 0.0 {
        f64::INFINITY
    } else {
        num(pd.r().wepcap) / drain
    }
}
