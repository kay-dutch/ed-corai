//! Named numeric metrics over a ship: the vocabulary for objectives and constraints.
//!
//! Engine figures come from `corai-engine` (Coriolis parity). Where Coriolis' own derived
//! numbers are physically wrong, the metric is computed correctly here instead; notably jump
//! ranges, because Coriolis' `unladenMass` already contains a full tank and its range code
//! adds the fuel a second time.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use corai_data::DATA;
use corai_engine::calc::{self, ArmourMetrics, ShieldMetrics};
use corai_engine::jsmath::num;
use corai_engine::module::Resolved;
use corai_engine::{Module, Ship, SlotKind};

/// Stand-in for "never" / "infinite" so that comparisons stay numeric.
pub const UNBOUNDED: f64 = 1e6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Better {
    Higher,
    Lower,
}

/// Incoming damage mix used by effective-HP metrics (relative weights).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageProfile {
    pub kinetic: f64,
    pub thermal: f64,
    pub explosive: f64,
    pub absolute: f64,
}

impl Default for DamageProfile {
    fn default() -> Self {
        Self {
            kinetic: 1.0,
            thermal: 1.0,
            explosive: 1.0,
            absolute: 0.0,
        }
    }
}

impl DamageProfile {
    /// Weights scaled to sum to 1 (an all-zero profile falls back to the default).
    pub fn normalised(self) -> Self {
        let sum = self.kinetic + self.thermal + self.explosive + self.absolute;
        if sum <= 0.0 || !sum.is_finite() {
            return Self::default().normalised();
        }
        Self {
            kinetic: self.kinetic / sum,
            thermal: self.thermal / sum,
            explosive: self.explosive / sum,
            absolute: self.absolute / sum,
        }
    }
}

/// Assumptions metrics are evaluated under.
#[derive(Debug, Clone)]
pub struct MetricContext {
    /// SYS pips while fighting, 0..=4 (shield resistance, regen under fire).
    pub sys: f64,
    /// WEP pips while fighting, 0..=4 (weapon capacitor recharge).
    pub wep: f64,
    /// SYS pips while recovering, 0..=4 (shield recharge and rebuild times).
    pub recovery_sys: f64,
    /// Normalised incoming damage mix.
    pub damage: DamageProfile,
    /// Engagement range in metres for damage falloff.
    pub range: f64,
    opponent: Ship,
    opponent_sys: f64,
    opponent_shields: Option<ShieldMetrics>,
    opponent_armour: ArmourMetrics,
}

impl MetricContext {
    pub fn new(
        sys: f64,
        wep: f64,
        damage: DamageProfile,
        opponent: Ship,
        opponent_sys: f64,
        range: f64,
    ) -> Self {
        let opponent_shields = calc::shield_metrics(&opponent, opponent_sys);
        let opponent_armour = calc::armour_metrics(&opponent);
        Self {
            sys,
            wep,
            recovery_sys: 4.0,
            damage: damage.normalised(),
            range,
            opponent,
            opponent_sys,
            opponent_shields,
            opponent_armour,
        }
    }

    /// Reference target for the DPS-against-target metrics.
    pub fn opponent(&self) -> &Ship {
        &self.opponent
    }

    pub fn opponent_sys(&self) -> f64 {
        self.opponent_sys
    }
}

impl Default for MetricContext {
    /// Fight on 2 SYS / 4 WEP, recover on 4 SYS; equal kinetic/thermal/explosive damage; stock
    /// Anaconda at 1 km.
    fn default() -> Self {
        let opponent = Ship::stock("anaconda").expect("the Anaconda is in the embedded data set");
        Self::new(2.0, 4.0, DamageProfile::default(), opponent, 2.0, 1000.0)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MetricError {
    #[error("unknown metric {0}")]
    Unknown(String),
    #[error("unknown module group {group} in metric {metric}")]
    UnknownGroup { metric: String, group: String },
}

/// Which module groups can move a metric in its good direction (beyond core modules).
#[derive(Debug, Clone, Copy)]
enum Groups {
    None,
    Fixed(&'static [&'static str]),
    Weapons,
    ExperimentalWeapons,
}

/// A built-in metric.
pub struct MetricDef {
    pub name: &'static str,
    pub unit: &'static str,
    pub better: Better,
    pub description: &'static str,
    groups: Groups,
    compute: fn(&Ship, &MetricContext) -> f64,
}

impl fmt::Debug for MetricDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricDef")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A metric to optimise or constrain.
#[derive(Debug, Clone, Copy)]
pub enum Metric {
    Named(&'static MetricDef),
    /// `count:<group>`: number of fitted modules of a group.
    Count(&'static str),
    /// `class:<group>`: sum of the classes of fitted modules of a group.
    ClassSum(&'static str),
}

impl Metric {
    pub fn name(&self) -> String {
        match self {
            Self::Named(d) => d.name.to_string(),
            Self::Count(g) => format!("count:{g}"),
            Self::ClassSum(g) => format!("class:{g}"),
        }
    }

    pub fn unit(&self) -> &'static str {
        match self {
            Self::Named(d) => d.unit,
            Self::Count(_) => "modules",
            Self::ClassSum(_) => "class",
        }
    }

    pub fn better(&self) -> Better {
        match self {
            Self::Named(d) => d.better,
            Self::Count(_) | Self::ClassSum(_) => Better::Higher,
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::Named(d) => d.description.to_string(),
            Self::Count(g) => format!("Number of fitted {g} modules."),
            Self::ClassSum(g) => format!("Sum of the classes of fitted {g} modules."),
        }
    }

    /// Module groups whose presence can improve this metric.
    pub fn groups(&self) -> Vec<&'static str> {
        match self {
            Self::Named(d) => match d.groups {
                Groups::None => Vec::new(),
                Groups::Fixed(g) => g.to_vec(),
                Groups::Weapons => WEAPON_GROUPS.clone(),
                Groups::ExperimentalWeapons => EXPERIMENTAL_WEAPON_GROUPS.clone(),
            },
            Self::Count(g) | Self::ClassSum(g) => vec![*g],
        }
    }

    pub fn compute(&self, ship: &Ship, ctx: &MetricContext) -> f64 {
        match self {
            Self::Named(d) => (d.compute)(ship, ctx),
            Self::Count(g) => modules(ship).filter(|m| m.grp() == *g).count() as f64,
            Self::ClassSum(g) => modules(ship)
                .filter(|m| m.grp() == *g)
                .map(|m| f64::from(m.def.class))
                .sum(),
        }
    }

    /// All built-in metrics.
    pub fn catalog() -> impl Iterator<Item = Metric> {
        METRICS.iter().map(Metric::Named)
    }
}

impl FromStr for Metric {
    type Err = MetricError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(def) = METRICS.iter().find(|d| d.name == s) {
            return Ok(Self::Named(def));
        }
        let (kind, group) = s
            .split_once(':')
            .ok_or_else(|| MetricError::Unknown(s.into()))?;
        let group = group_name(group).ok_or_else(|| MetricError::UnknownGroup {
            metric: s.into(),
            group: group.into(),
        })?;
        match kind {
            "count" => Ok(Self::Count(group)),
            "class" => Ok(Self::ClassSum(group)),
            _ => Err(MetricError::Unknown(s.into())),
        }
    }
}

/// The interned (data-owned) name of a module group.
pub fn group_name(group: &str) -> Option<&'static str> {
    let data = &*DATA;
    [
        &data.modules.standard,
        &data.modules.hardpoints,
        &data.modules.internal,
    ]
    .into_iter()
    .find_map(|groups| groups.get_key_value(group))
    .map(|(k, _)| k.as_str())
}

static WEAPON_GROUPS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let data = &*DATA;
    data.modules
        .hardpoints
        .iter()
        .filter(|(_, ms)| ms.iter().any(|m| m.class > 0 && m.stat("damage").is_some()))
        .map(|(g, _)| g.as_str())
        .collect()
});

static EXPERIMENTAL_WEAPON_GROUPS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let data = &*DATA;
    WEAPON_GROUPS
        .iter()
        .copied()
        .filter(|g| data.modules.hardpoints[*g].iter().any(|m| m.experimental))
        .collect()
});

// ---------------------------------------------------------------- helpers

fn modules(ship: &Ship) -> impl Iterator<Item = &Module> {
    ship.internal
        .iter()
        .chain(&ship.hardpoints)
        .filter_map(|s| s.module.as_ref())
}

fn sum_stat(ship: &Ship, grp: &str, stat: fn(&Resolved) -> Option<f64>) -> f64 {
    modules(ship)
        .filter(|m| m.grp() == grp)
        .map(|m| num(stat(m.r())))
        .sum()
}

fn bounded(x: f64) -> f64 {
    if x.is_finite() { x } else { UNBOUNDED }
}

/// One jump with `fuel` tonnes aboard and `cargo` tonnes of cargo, burning as much of the fuel
/// as the FSD allows. Mass is the dry mass plus what is actually aboard.
pub fn jump(ship: &Ship, cargo: f64, fuel: f64) -> f64 {
    let Some(fsd) = ship.fsd() else { return 0.0 };
    let burn = num(fsd.r().maxfuel).min(fuel);
    calc::jump_range(ship, ship.stats.dry_mass + cargo + fuel, fsd, Some(burn))
}

/// Total distance over consecutive jumps until the tank is empty.
pub fn total_range(ship: &Ship, cargo: f64) -> f64 {
    let Some(fsd) = ship.fsd() else { return 0.0 };
    let fuel = ship.stats.fuel_capacity;
    calc::total_jump_range(ship, ship.stats.dry_mass + cargo + fuel, fsd, fuel)
}

fn shield_mult(s: &ShieldMetrics, d: &DamageProfile) -> f64 {
    d.kinetic * s.kinetic.total
        + d.thermal * s.thermal.total
        + d.explosive * s.explosive.total
        + d.absolute * s.absolute.total
}

fn shield_ehp(ship: &Ship, ctx: &MetricContext, with_cells: bool) -> f64 {
    calc::shield_metrics(ship, ctx.sys).map_or(0.0, |s| {
        let raw = s.summary + if with_cells { s.cells } else { 0.0 };
        raw / shield_mult(&s, &ctx.damage)
    })
}

fn hull_ehp(ship: &Ship, ctx: &MetricContext) -> f64 {
    let a = calc::armour_metrics(ship);
    let d = &ctx.damage;
    a.total
        / (d.kinetic * a.kinetic.total
            + d.thermal * a.thermal.total
            + d.explosive * a.explosive.total
            + d.absolute * a.absolute.total)
}

fn vs_target(ship: &Ship, ctx: &MetricContext) -> (f64, f64) {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
        .filter_map(|s| s.module.as_ref())
        .map(|m| {
            calc::weapon_sustained_dps(
                m,
                &ctx.opponent,
                ctx.opponent_shields.as_ref(),
                &ctx.opponent_armour,
                ctx.range,
            )
        })
        .fold((0.0, 0.0), |(sh, ar), w| {
            (sh + w.shields.total(), ar + w.armour.total())
        })
}

fn experimental_sdps(ship: &Ship) -> f64 {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.def.experimental)
        .map(|m| {
            let r = m.r();
            match r.clip.filter(|c| *c != 0.0) {
                Some(c) => (c * r.dps / r.rof) / (c / r.rof + num(r.reload)),
                None => r.dps,
            }
        })
        .sum()
}

/// Fitted, enabled weapons (hardpoints that deal damage).
fn weapons(ship: &Ship) -> impl Iterator<Item = &Module> {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.r().dps != 0.0)
}

fn distinct<'a>(items: impl Iterator<Item = &'a str>) -> f64 {
    items.collect::<std::collections::HashSet<_>>().len() as f64
}

/// Seconds of continuous fire until the first ammunition-limited weapon has used its whole
/// magazine and reserve. Plasma Slug draws fuel instead of ammunition (Coriolis does not model
/// that), so it counts as unlimited.
fn ammo_endurance(ship: &Ship) -> f64 {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| {
            m.r().dps != 0.0
                && !m
                    .special()
                    .is_some_and(|sp| sp.def.edname.starts_with("special_plasma_slug"))
        })
        .filter_map(|m| {
            let r = m.r();
            let rounds = num(r.ammo) + num(r.clip);
            let shots_per_second = r.rof * (r.sdps / r.dps);
            (rounds > 0.0 && shots_per_second > 0.0).then(|| rounds / shots_per_second)
        })
        .fold(UNBOUNDED, f64::min)
}

const SHIELD: &[&str] = &["sg", "bsg", "psg", "sb", "gsrp"];
const SHIELD_CELLS: &[&str] = &["sg", "bsg", "psg", "sb", "gsrp", "scb"];
const HULL: &[&str] = &["hr", "ghrp", "mahr"];
const DEFENCE: &[&str] = &["sg", "bsg", "psg", "sb", "gsrp", "hr", "ghrp", "mahr"];
const DEFENCE_CELLS: &[&str] = &[
    "sg", "bsg", "psg", "sb", "gsrp", "hr", "ghrp", "mahr", "scb",
];

macro_rules! metric {
    ($name:literal, $unit:literal, $better:ident, $groups:expr, $desc:literal, $f:expr) => {
        MetricDef {
            name: $name,
            unit: $unit,
            better: Better::$better,
            description: $desc,
            groups: $groups,
            compute: $f,
        }
    };
}

static METRICS: &[MetricDef] = &[
    // --- travel
    metric!(
        "unladenRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Longest single jump: no cargo, carrying just the fuel one maximum jump burns.",
        |s, _| jump(
            s,
            0.0,
            s.stats
                .fuel_capacity
                .min(s.fsd().map_or(0.0, |f| num(f.r().maxfuel)))
        )
    ),
    metric!(
        "fullTankRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Single jump with a full fuel tank and no cargo.",
        |s, _| jump(s, 0.0, s.stats.fuel_capacity)
    ),
    metric!(
        "ladenRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Single jump with a full fuel tank and every cargo rack full.",
        |s, _| jump(s, s.stats.cargo_capacity, s.stats.fuel_capacity)
    ),
    metric!(
        "totalRangeUnladen",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb", "ft"]),
        "Distance covered by consecutive jumps on one tank, no cargo.",
        |s, _| total_range(s, 0.0)
    ),
    metric!(
        "totalRangeLaden",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb", "ft"]),
        "Distance covered by consecutive jumps on one tank with full cargo.",
        |s, _| total_range(s, s.stats.cargo_capacity)
    ),
    metric!(
        "topSpeed",
        "m/s",
        Higher,
        Groups::None,
        "Top speed (4 pips ENG, no cargo, full tank).",
        |s, _| s.stats.top_speed
    ),
    metric!(
        "topBoost",
        "m/s",
        Higher,
        Groups::None,
        "Boost speed (4 pips ENG); 0 if the ship cannot boost.",
        |s, _| s.stats.top_boost
    ),
    metric!(
        "pitch",
        "deg/s",
        Higher,
        Groups::None,
        "Pitch rate (4 pips ENG).",
        |s, _| s.stats.top_pitch
    ),
    metric!(
        "yaw",
        "deg/s",
        Higher,
        Groups::None,
        "Yaw rate (4 pips ENG).",
        |s, _| s.stats.top_yaw
    ),
    metric!(
        "roll",
        "deg/s",
        Higher,
        Groups::None,
        "Roll rate (4 pips ENG).",
        |s, _| s.stats.top_roll
    ),
    // --- capacity, mass, money
    metric!(
        "cargoCapacity",
        "t",
        Higher,
        Groups::Fixed(&["cr", "crl"]),
        "Total cargo rack capacity.",
        |s, _| s.stats.cargo_capacity
    ),
    metric!(
        "passengerCapacity",
        "seats",
        Higher,
        Groups::Fixed(&["pce", "pci", "pcm", "pcq"]),
        "Total passenger berths.",
        |s, _| s.stats.passenger_capacity
    ),
    metric!(
        "fuelCapacity",
        "t",
        Higher,
        Groups::Fixed(&["ft"]),
        "Main plus internal fuel tank capacity.",
        |s, _| s.stats.fuel_capacity
    ),
    metric!(
        "dryMass",
        "t",
        Lower,
        Groups::None,
        "Hull and modules, without fuel or cargo.",
        |s, _| s.stats.dry_mass
    ),
    metric!(
        "ladenMass",
        "t",
        Lower,
        Groups::None,
        "Hull, modules, full fuel and full cargo.",
        |s, _| s.stats.laden_mass
    ),
    metric!(
        "totalCost",
        "CR",
        Lower,
        Groups::None,
        "Purchase price of hull and modules (no discounts; engineering not priced).",
        |s, _| s.stats.total_cost
    ),
    metric!(
        "powerMargin",
        "MW",
        Higher,
        Groups::None,
        "Power plant output minus draw with hardpoints deployed.",
        |s, _| s.stats.power_available - s.stats.power_deployed
    ),
    metric!(
        "fuelScoopRate",
        "kg/s",
        Higher,
        Groups::Fixed(&["fs"]),
        "Fuel scoop rate at a main-sequence star.",
        |s, _| sum_stat(s, "fs", |r| r.rate)
    ),
    // --- mining
    metric!(
        "refineryBins",
        "bins",
        Higher,
        Groups::Fixed(&["rf"]),
        "Refinery bins.",
        |s, _| sum_stat(s, "rf", |r| r.bins)
    ),
    metric!(
        "collectorLimpets",
        "limpets",
        Higher,
        Groups::Fixed(&["cc", "mlc"]),
        "Simultaneously active collector limpets (collector and multi-limpet controllers).",
        |s, _| sum_stat(s, "cc", |r| r.maximum) + sum_stat(s, "mlc", |r| r.maximum)
    ),
    // --- defence
    metric!(
        "shieldRaw",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD),
        "Shield strength (generator, boosters, guardian reinforcement); no resistances.",
        |s, c| calc::shield_metrics(s, c.sys).map_or(0.0, |m| m.summary)
    ),
    metric!(
        "shieldEHP",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD),
        "Effective shield HP against the context damage mix at the context SYS pips.",
        |s, c| shield_ehp(s, c, false)
    ),
    metric!(
        "shieldEHPWithCells",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD_CELLS),
        "shieldEHP plus total shield cell bank capacity.",
        |s, c| shield_ehp(s, c, true)
    ),
    metric!(
        "shieldRecharge",
        "s",
        Lower,
        Groups::Fixed(SHIELD),
        "Seconds to recharge from 50% to 100% at the recovery SYS pips.",
        |s, c| calc::shield_metrics(s, c.recovery_sys).map_or(UNBOUNDED, |m| bounded(m.recharge))
    ),
    metric!(
        "shieldRecover",
        "s",
        Lower,
        Groups::Fixed(SHIELD),
        "Seconds from broken back to 50% at the recovery SYS pips (includes the 16 s delay).",
        |s, c| calc::shield_metrics(s, c.recovery_sys).map_or(UNBOUNDED, |m| bounded(m.recover))
    ),
    metric!(
        "hullRaw",
        "HP",
        Higher,
        Groups::Fixed(HULL),
        "Armour (bulkheads and hull reinforcements); no resistances.",
        |s, _| calc::armour_metrics(s).total
    ),
    metric!(
        "hullEHP",
        "HP",
        Higher,
        Groups::Fixed(HULL),
        "Effective armour against the context damage mix.",
        |s, c| hull_ehp(s, c)
    ),
    metric!(
        "totalEHP",
        "HP",
        Higher,
        Groups::Fixed(DEFENCE),
        "shieldEHP + hullEHP: damage absorbed before destruction.",
        |s, c| shield_ehp(s, c, false) + hull_ehp(s, c)
    ),
    metric!(
        "totalEHPWithCells",
        "HP",
        Higher,
        Groups::Fixed(DEFENCE_CELLS),
        "totalEHP plus shield cell bank capacity.",
        |s, c| shield_ehp(s, c, true) + hull_ehp(s, c)
    ),
    metric!(
        "moduleArmour",
        "HP",
        Higher,
        Groups::Fixed(&["mrp", "gmrp"]),
        "Total integrity of module reinforcement packages: the damage they absorb before they are used up.",
        |s, _| calc::armour_metrics(s).module_armour
    ),
    metric!(
        "moduleProtection",
        "fraction",
        Higher,
        Groups::Fixed(&["mrp", "gmrp"]),
        "Share of module damage absorbed by module reinforcements.",
        |s, _| calc::armour_metrics(s).module_protection
    ),
    // --- offence
    metric!(
        "sustainedDps",
        "dps",
        Higher,
        Groups::Weapons,
        "Raw sustained DPS of all weapons including reloads.",
        |s, _| s.stats.total_sdps
    ),
    metric!(
        "burstDps",
        "dps",
        Higher,
        Groups::Weapons,
        "Raw DPS of all weapons ignoring reloads.",
        |s, _| s.stats.total_dps
    ),
    metric!(
        "dpsVsShields",
        "dps",
        Higher,
        Groups::Weapons,
        "Sustained DPS against the reference opponent's shields at the context range.",
        |s, c| vs_target(s, c).0
    ),
    metric!(
        "dpsVsHull",
        "dps",
        Higher,
        Groups::Weapons,
        "Sustained DPS against the reference opponent's hull at the context range (hardness and falloff).",
        |s, c| vs_target(s, c).1
    ),
    metric!(
        "weaponSustain",
        "s",
        Higher,
        Groups::None,
        "Seconds of continuous fire before the WEP capacitor empties at the context WEP pips (1e6 = sustainable).",
        |s, c| bounded(calc::time_to_drain_wep(s, c.wep))
    ),
    metric!(
        "emptyHardpoints",
        "slots",
        Lower,
        Groups::None,
        "Weapon hardpoints left empty.",
        |s, _| s
            .hardpoints
            .iter()
            .filter(|h| h.kind == SlotKind::Hardpoint && h.module.is_none())
            .count() as f64
    ),
    metric!(
        "emptyUtilities",
        "slots",
        Lower,
        Groups::None,
        "Utility mounts left empty.",
        |s, _| s
            .hardpoints
            .iter()
            .filter(|h| h.kind == SlotKind::Utility && h.module.is_none())
            .count() as f64
    ),
    metric!(
        "weaponTypes",
        "types",
        Lower,
        Groups::None,
        "Number of distinct weapon types fitted; cap it for a matched, manageable loadout.",
        |s, _| distinct(weapons(s).map(Module::grp))
    ),
    metric!(
        "mountTypes",
        "types",
        Lower,
        Groups::None,
        "Number of distinct mount types among weapons (fixed, gimballed, turreted).",
        |s, _| distinct(weapons(s).map(|m| m.def.mount.as_deref().unwrap_or("-")))
    ),
    metric!(
        "weaponHeat",
        "heat/s",
        Lower,
        Groups::None,
        "Heat generated per second by all weapons firing.",
        |s, _| s.stats.total_hps
    ),
    metric!(
        "ammoEndurance",
        "s",
        Higher,
        Groups::None,
        "Seconds of continuous fire until the first ammunition-limited weapon runs dry (1e6 = none are ammo-limited).",
        |s, _| ammo_endurance(s)
    ),
    metric!(
        "axSustainedDps",
        "dps",
        Higher,
        Groups::ExperimentalWeapons,
        "Raw sustained DPS of AX/Guardian experimental weapons only.",
        |s, _| experimental_sdps(s)
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(name: &str) -> Metric {
        name.parse().unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn jump_ranges_count_fuel_once() {
        let ship = Ship::stock("python").expect("python");
        let ctx = MetricContext::default();
        let fsd = ship.fsd().expect("fsd");
        let fuel = ship.stats.fuel_capacity;
        // Full tank, no cargo: exactly dry mass + tank, burning one maximum jump.
        let expected = calc::jump_range(
            &ship,
            ship.stats.dry_mass + fuel,
            fsd,
            Some(num(fsd.r().maxfuel).min(fuel)),
        );
        assert_eq!(metric("fullTankRange").compute(&ship, &ctx), expected);
        // Coriolis' own figure counts the tank twice, so it is strictly shorter.
        let (_, coriolis_full_tank, _) = ship.coriolis_jump_stats();
        assert!(expected > coriolis_full_tank);
    }

    #[test]
    fn jump_ranges_are_ordered() {
        let ctx = MetricContext::default();
        for id in ["python", "anaconda", "type_9_heavy", "diamondback_explorer"] {
            let ship = Ship::stock(id).expect("ship");
            let [unladen, full, laden] = ["unladenRange", "fullTankRange", "ladenRange"]
                .map(|n| metric(n).compute(&ship, &ctx));
            assert!(
                unladen >= full && full >= laden && laden > 0.0,
                "{id}: {unladen} {full} {laden}"
            );
        }
    }

    #[test]
    fn parses_parametrised_metrics() {
        assert!(matches!(metric("count:fs"), Metric::Count("fs")));
        assert!(matches!(metric("class:cc"), Metric::ClassSum("cc")));
        assert!(matches!(
            "count:zz".parse::<Metric>(),
            Err(MetricError::UnknownGroup { .. })
        ));
        assert!(matches!(
            "nonsense".parse::<Metric>(),
            Err(MetricError::Unknown(_))
        ));
        assert!(matches!(
            "size:fs".parse::<Metric>(),
            Err(MetricError::Unknown(_))
        ));
    }

    #[test]
    fn every_catalog_metric_is_finite_on_every_stock_ship() {
        let ctx = MetricContext::default();
        for id in DATA.ships.keys() {
            let ship = Ship::stock(id).expect("ship");
            for m in Metric::catalog() {
                let v = m.compute(&ship, &ctx);
                assert!(v.is_finite(), "{id} {}: {v}", m.name());
            }
        }
    }

    #[test]
    fn stock_counts() {
        let ship = Ship::stock("python").expect("python");
        let ctx = MetricContext::default();
        assert!(metric("count:sg").compute(&ship, &ctx) == 1.0);
        assert!(metric("count:fs").compute(&ship, &ctx) == 0.0);
    }

    #[test]
    fn damage_profile_normalises() {
        let d = DamageProfile {
            kinetic: 2.0,
            thermal: 2.0,
            explosive: 0.0,
            absolute: 0.0,
        }
        .normalised();
        assert_eq!((d.kinetic, d.thermal), (0.5, 0.5));
        let zero = DamageProfile {
            kinetic: 0.0,
            thermal: 0.0,
            explosive: 0.0,
            absolute: 0.0,
        }
        .normalised();
        assert_eq!(zero, DamageProfile::default().normalised());
    }

    #[test]
    fn weapon_groups_come_from_data() {
        assert!(WEAPON_GROUPS.contains(&"mc") && WEAPON_GROUPS.contains(&"pa"));
        assert!(!WEAPON_GROUPS.contains(&"sb"));
        assert!(
            EXPERIMENTAL_WEAPON_GROUPS.contains(&"axmc")
                && !EXPERIMENTAL_WEAPON_GROUPS.contains(&"mc")
        );
    }
}
