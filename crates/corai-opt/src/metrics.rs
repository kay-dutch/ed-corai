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

use crate::arrange::combat_deployed_draw;

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
    /// Named mixes. `even` is the neutral default; `pve` leans thermal (NPC lasers and incendiary
    /// rounds) and is an assumption, not a measurement.
    pub fn preset(name: &str) -> Option<Self> {
        match name {
            "even" => Some(Self::default()),
            "pve" => Some(Self {
                kinetic: 0.30,
                thermal: 0.60,
                explosive: 0.05,
                absolute: 0.05,
            }),
            _ => None,
        }
    }

    /// Weights scaled to sum to 1 (an all-zero profile stays zero, as in the Node version).
    pub fn normalised(self) -> Self {
        let sum = self.kinetic + self.thermal + self.explosive + self.absolute;
        let sum = if sum == 0.0 || !sum.is_finite() {
            1.0
        } else {
            sum
        };
        Self {
            kinetic: self.kinetic / sum,
            thermal: self.thermal / sum,
            explosive: self.explosive / sum,
            absolute: self.absolute / sum,
        }
    }
}

/// Optional overrides for [`MetricContext`]; `None` takes the default (`makeContext` options).
#[derive(Debug, Clone, Default)]
pub struct ContextOptions {
    pub sys: Option<f64>,
    pub wep: Option<f64>,
    pub recovery_sys: Option<f64>,
    /// `even` (default) or `pve`; `damage` overrides it.
    pub damage_preset: Option<String>,
    pub damage: Option<DamageProfile>,
    pub opponent: Option<Ship>,
    pub opponent_sys: Option<f64>,
    pub range: Option<f64>,
    pub incoming_dps: Option<f64>,
    pub regen_under_fire: Option<f64>,
    pub reserve_fuel_in_jump_mass: Option<bool>,
    pub vent_hit_rate: Option<f64>,
    pub thermal_vent_factor: Option<f64>,
    pub corrosive_hull_multiplier: Option<f64>,
    pub corrosive_piercing_bonus: Option<f64>,
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
    /// Raw incoming DPS for the time-under-fire metrics.
    pub incoming_dps: f64,
    /// Share of time shields regenerate while under fire (0 = regen pauses on every hit).
    pub regen_under_fire: f64,
    /// Count the reserve tank in jump mass (Coriolis does; the journal's MaxJumpRange does not).
    pub reserve_fuel_in_jump_mass: bool,
    /// Share of time Thermal Vent beams are on target.
    pub vent_hit_rate: f64,
    /// Heat a hitting Thermal Vent beam removes, as a multiple of its own thermal load.
    pub thermal_vent_factor: f64,
    /// Hull damage multiplier while a Corrosive Shell weapon keeps hitting (does not stack).
    pub corrosive_hull_multiplier: f64,
    /// Armour piercing added to the Corrosive Shell weapon itself.
    pub corrosive_piercing_bonus: f64,
    opponent: Ship,
    opponent_sys: f64,
    opponent_shields: Option<ShieldMetrics>,
    opponent_armour: ArmourMetrics,
}

impl MetricContext {
    /// A context from options, with the Node version's defaults and clamps.
    ///
    /// # Errors
    /// Unknown damage preset.
    pub fn from_options(o: ContextOptions) -> Result<Self, MetricError> {
        let preset = o.damage_preset.as_deref().unwrap_or("even");
        let base = DamageProfile::preset(preset)
            .ok_or_else(|| MetricError::UnknownPreset(preset.into()))?;
        let opponent = match o.opponent {
            Some(s) => s,
            None => Ship::stock("anaconda").expect("the Anaconda is in the embedded data set"),
        };
        let opponent_sys = o.opponent_sys.unwrap_or(2.0);
        let opponent_shields = calc::shield_metrics(&opponent, opponent_sys);
        let opponent_armour = calc::armour_metrics(&opponent);
        Ok(Self {
            // Typical PvE pip use: fight on 2 SYS / 4 WEP, recover shields on 4 SYS.
            sys: o.sys.unwrap_or(2.0),
            wep: o.wep.unwrap_or(4.0),
            recovery_sys: o.recovery_sys.unwrap_or(4.0),
            damage: o.damage.unwrap_or(base).normalised(),
            range: o.range.unwrap_or(1000.0),
            incoming_dps: o.incoming_dps.unwrap_or(200.0),
            regen_under_fire: o.regen_under_fire.unwrap_or(0.0).clamp(0.0, 1.0),
            reserve_fuel_in_jump_mass: o.reserve_fuel_in_jump_mass.unwrap_or(false),
            vent_hit_rate: o.vent_hit_rate.unwrap_or(1.0).clamp(0.0, 1.0),
            thermal_vent_factor: o.thermal_vent_factor.unwrap_or(1.0).max(0.0),
            corrosive_hull_multiplier: o.corrosive_hull_multiplier.unwrap_or(1.25).max(1.0),
            corrosive_piercing_bonus: o.corrosive_piercing_bonus.unwrap_or(20.0).max(0.0),
            opponent,
            opponent_sys,
            opponent_shields,
            opponent_armour,
        })
    }

    pub fn new(
        sys: f64,
        wep: f64,
        damage: DamageProfile,
        opponent: Ship,
        opponent_sys: f64,
        range: f64,
    ) -> Self {
        Self::from_options(ContextOptions {
            sys: Some(sys),
            wep: Some(wep),
            damage: Some(damage),
            opponent: Some(opponent),
            opponent_sys: Some(opponent_sys),
            range: Some(range),
            ..ContextOptions::default()
        })
        .expect("no preset given")
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
    /// Anaconda at 1 km; 200 incoming DPS.
    fn default() -> Self {
        Self::from_options(ContextOptions::default()).expect("default preset exists")
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MetricError {
    #[error("unknown metric {0}")]
    Unknown(String),
    #[error("unknown module group {group} in metric {metric}")]
    UnknownGroup { metric: String, group: String },
    #[error("unknown damage preset {0} (even, pve)")]
    UnknownPreset(String),
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

/// JS truthiness of a number (`0` and `NaN` are false).
fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

/// Reserve tank mass to take back out of the jump mass (the engine's jump formula adds it).
fn reserve(ship: &Ship, ctx: &MetricContext) -> f64 {
    if ctx.reserve_fuel_in_jump_mass {
        0.0
    } else {
        ship.def.properties.reserve_fuel_capacity
    }
}

/// One jump with `fuel` tonnes aboard and `cargo` tonnes of cargo, burning as much of the fuel
/// as the FSD allows. Mass is the dry mass plus what is actually aboard.
pub fn jump(ship: &Ship, cargo: f64, fuel: f64, ctx: &MetricContext) -> f64 {
    let Some(fsd) = ship.fsd() else { return 0.0 };
    let burn = num(fsd.r().maxfuel).min(fuel);
    calc::jump_range(
        ship,
        ship.stats.dry_mass + cargo + fuel - reserve(ship, ctx),
        fsd,
        Some(burn),
    )
}

/// Total distance over consecutive jumps until the tank is empty.
pub fn total_range(ship: &Ship, cargo: f64, ctx: &MetricContext) -> f64 {
    let Some(fsd) = ship.fsd() else { return 0.0 };
    let fuel = ship.stats.fuel_capacity;
    calc::total_jump_range(
        ship,
        ship.stats.dry_mass + fuel + cargo - reserve(ship, ctx),
        fsd,
        fuel,
    )
}

/// Shield figures at `sys` pips (zero shield and unbounded times without a generator).
struct ShieldView {
    raw: f64,
    cells: f64,
    mult: f64,
    recharge: f64,
    recover: f64,
}

fn shield_view(ship: &Ship, ctx: &MetricContext, sys: f64) -> ShieldView {
    match calc::shield_metrics(ship, sys) {
        None => ShieldView {
            raw: 0.0,
            cells: 0.0,
            mult: 1.0,
            recharge: UNBOUNDED,
            recover: UNBOUNDED,
        },
        Some(s) => {
            let d = &ctx.damage;
            ShieldView {
                raw: s.summary,
                cells: s.cells,
                mult: d.kinetic * s.kinetic.total
                    + d.thermal * s.thermal.total
                    + d.explosive * s.explosive.total
                    + d.absolute * s.absolute.total,
                recharge: bounded(s.recharge),
                recover: bounded(s.recover),
            }
        }
    }
}

fn shield_ehp(ship: &Ship, ctx: &MetricContext, with_cells: bool) -> f64 {
    let s = shield_view(ship, ctx, ctx.sys);
    (s.raw + if with_cells { s.cells } else { 0.0 }) / s.mult
}

/// Regeneration the distributor can keep up indefinitely at the fighting SYS pips: each MJ
/// regenerated costs the generator's distributor draw from the SYS capacitor.
fn sustained_regen(ship: &Ship, ctx: &MetricContext) -> f64 {
    let Some(slot) = ship.find_internal_by_group("sg") else {
        return 0.0;
    };
    let (Some(sg), Some(pd)) = (slot.module.as_ref(), ship.power_distributor()) else {
        return 0.0;
    };
    if !slot.enabled || shield_view(ship, ctx, ctx.sys).raw == 0.0 {
        return 0.0;
    }
    let regen = num(sg.r().regen);
    let draw = num(sg.r().distdraw);
    let sys_rate = calc::sys_recharge_rate(Some(pd), ctx.sys);
    if draw > 0.0 {
        regen.min(sys_rate / draw)
    } else {
        regen
    }
}

/// Seconds until shields break under the context's incoming raw DPS; regeneration counts only for
/// the `regen_under_fire` share of the time.
fn shield_time_under_fire(ship: &Ship, ctx: &MetricContext, with_cells: bool) -> f64 {
    let s = shield_view(ship, ctx, ctx.sys);
    if s.raw == 0.0 {
        return 0.0;
    }
    let pool = s.raw + if with_cells { s.cells } else { 0.0 };
    let net = ctx.incoming_dps * s.mult - sustained_regen(ship, ctx) * ctx.regen_under_fire;
    if net > 0.0 {
        (pool / net).min(UNBOUNDED)
    } else {
        UNBOUNDED
    }
}

/// Effective shield HP against the single damage type the shield resists least.
fn shield_ehp_worst_type(ship: &Ship, ctx: &MetricContext) -> f64 {
    calc::shield_metrics(ship, ctx.sys).map_or(0.0, |s| {
        let worst = s.kinetic.total.max(s.thermal.total).max(s.explosive.total);
        if worst > 0.0 {
            s.summary / worst
        } else {
            UNBOUNDED
        }
    })
}

fn hull_mult(ship: &Ship, ctx: &MetricContext) -> f64 {
    let a = calc::armour_metrics(ship);
    let d = &ctx.damage;
    d.kinetic * a.kinetic.total
        + d.thermal * a.thermal.total
        + d.explosive * a.explosive.total
        + d.absolute * a.absolute.total
}

fn hull_ehp(ship: &Ship, ctx: &MetricContext) -> f64 {
    calc::armour_metrics(ship).total / hull_mult(ship, ctx)
}

fn special_is(m: &Module, edname: &str) -> bool {
    m.special().is_some_and(|s| s.def.edname == edname)
}

/// Piercing of a weapon including the Corrosive Shell bonus on the weapon that carries it.
fn piercing(m: &Module, ctx: &MetricContext) -> f64 {
    num(m.r().piercing)
        + if special_is(m, "special_corrosive_shell") {
            ctx.corrosive_piercing_bonus
        } else {
            0.0
        }
}

/// Damage falloff factor at `range` (1 inside falloff, 0 at maximum range).
fn falloff_factor(m: &Module, range: f64) -> f64 {
    let falloff = num(m.r().falloff);
    let max = num(m.r().range);
    if !truthy(falloff) || !truthy(max) || range <= falloff {
        return 1.0;
    }
    (1.0 - (range - falloff) / (max - falloff).max(1e-9)).max(0.0)
}

/// A powered Corrosive Shell weapon reaches the target: its 5 s debuff refreshes on every hit.
fn corrosive_active(ship: &Ship, ctx: &MetricContext) -> bool {
    ship.hardpoints.iter().any(|s| {
        s.enabled
            && s.module.as_ref().is_some_and(|m| {
                special_is(m, "special_corrosive_shell") && falloff_factor(m, ctx.range) > 0.0
            })
    })
}

/// Sustained damage against the reference opponent's shields and hull at the context range.
/// Coriolis models only Corrosive Shell's ammo penalty; its piercing bonus and hull debuff are
/// added here.
fn vs_target(ship: &Ship, ctx: &MetricContext) -> (f64, f64) {
    let shields = ctx.opponent_shields.as_ref();
    let fight = |m: &Module| {
        calc::weapon_sustained_dps(m, &ctx.opponent, shields, &ctx.opponent_armour, ctx.range)
    };
    let mounted = || {
        ship.hardpoints
            .iter()
            .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
            .filter_map(|s| s.module.as_ref())
    };
    let (sh, mut hull) = mounted().map(fight).fold((0.0, 0.0), |(sh, ar), w| {
        (sh + w.shields.total(), ar + w.armour.total())
    });
    if corrosive_active(ship, ctx) {
        let h = if truthy(ctx.opponent.def.properties.hardness) {
            ctx.opponent.def.properties.hardness
        } else {
            1.0
        };
        for m in ship
            .hardpoints
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.module.as_ref())
            .filter(|m| special_is(m, "special_corrosive_shell"))
        {
            let base = (num(m.r().piercing) / h).min(1.0);
            if base <= 0.0 {
                continue;
            }
            hull += fight(m).armour.total() * ((piercing(m, ctx) / h).min(1.0) / base - 1.0);
        }
        hull *= ctx.corrosive_hull_multiplier;
    }
    (sh, hull)
}

/// DPS-weighted share of hull damage that survives the opponent's hardness.
fn pierce_efficiency(ship: &Ship, ctx: &MetricContext) -> f64 {
    let total: f64 = weapons(ship).map(|m| m.r().sdps).sum();
    if !truthy(total) {
        return 0.0;
    }
    let hardness = ctx.opponent.def.properties.hardness;
    let h = if truthy(hardness) { hardness } else { 1.0 };
    weapons(ship)
        .map(|m| m.r().sdps * (piercing(m, ctx) / h).min(1.0))
        .sum::<f64>()
        / total
}

/// Seconds to strip the opponent's shields then hull at the context range (no opponent regen).
fn time_to_kill(ship: &Ship, ctx: &MetricContext) -> f64 {
    let (shields, hull) = vs_target(ship, ctx);
    let opp_shield = ctx.opponent_shields.as_ref().map_or(0.0, |s| s.summary);
    let opp_hull = ctx.opponent_armour.total;
    // NaN damage counts as none, as in the Node version.
    let positive = |x: f64| x > 0.0;
    if (opp_shield > 0.0 && !positive(shields)) || !positive(hull) {
        return UNBOUNDED;
    }
    let shield_time = if opp_shield > 0.0 {
        opp_shield / shields
    } else {
        0.0
    };
    (shield_time + opp_hull / hull).min(UNBOUNDED)
}

/// Seconds between boosts at 4 ENG pips (distributor refill or the hull's cooldown, if longer).
fn boost_interval(ship: &Ship) -> f64 {
    let Some(pd) = ship.power_distributor() else {
        return UNBOUNDED;
    };
    if !ship.can_boost(0.0, ship.stats.fuel_capacity) {
        return UNBOUNDED;
    }
    let rate = num(pd.r().engrate);
    let p = &ship.def.properties;
    if rate > 0.0 {
        p.boost_int.max(p.boost_energy / rate)
    } else {
        UNBOUNDED
    }
}

fn power_plant_heat(ship: &Ship) -> f64 {
    let eff = ship.standard[0]
        .module
        .as_ref()
        .map_or(0.0, |pp| num(pp.get("eff")));
    combat_deployed_draw(ship) * eff
}

/// Heat per second in a fight. Thermal Vent beams generate their heat only while missing; while on
/// target they remove `thermal_vent_factor` x their thermal load, reduced by falloff.
struct HeatBudget {
    vent_removal: f64,
    net: f64,
    break_even: f64,
}

fn heat_budget(ship: &Ship, ctx: &MetricContext) -> HeatBudget {
    let plant = power_plant_heat(ship);
    let (mut weapons_heat, mut vent_generation, mut vent_removal, mut unit_removal) =
        (0.0, 0.0, 0.0, 0.0);
    for m in weapons(ship) {
        let hps = if m.r().hps.is_nan() { 0.0 } else { m.r().hps };
        if special_is(m, "special_thermal_vent") {
            let reach = falloff_factor(m, ctx.range);
            vent_generation += (1.0 - ctx.vent_hit_rate) * hps;
            vent_removal += ctx.vent_hit_rate * ctx.thermal_vent_factor * hps * reach;
            unit_removal += ctx.vent_hit_rate * hps * reach;
        } else {
            weapons_heat += hps;
        }
    }
    let generated = plant + weapons_heat + vent_generation;
    HeatBudget {
        vent_removal,
        net: generated - vent_removal,
        break_even: if unit_removal > 0.0 {
            generated / unit_removal
        } else {
            UNBOUNDED
        },
    }
}

fn experimental_sdps(ship: &Ship) -> f64 {
    ship.hardpoints
        .iter()
        .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| m.def.experimental && truthy(m.r().dps))
        .map(|m| {
            let r = m.r();
            match r.clip.filter(|c| truthy(*c)) {
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
        .filter(|m| truthy(m.r().dps))
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
            truthy(m.r().dps)
                && !m
                    .special()
                    .is_some_and(|sp| sp.def.edname.starts_with("special_plasma_slug"))
        })
        .filter_map(|m| {
            let r = m.r();
            let rounds = num(r.ammo) + num(r.clip);
            let shots_per_second = r.rof * m.sustained_factor();
            (truthy(rounds) && shots_per_second > 0.0).then(|| rounds / shots_per_second)
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

/// In the Node version's order, so both list the same metrics the same way.
static METRICS: &[MetricDef] = &[
    // --- travel
    metric!(
        "unladenRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Max single jump with no cargo and just enough fuel for that jump (the headline Coriolis/EDSY jump range).",
        |s, c| jump(
            s,
            0.0,
            s.stats
                .fuel_capacity
                .min(s.fsd().map_or(0.0, |f| num(f.r().maxfuel))),
            c
        )
    ),
    metric!(
        "fullTankRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Single jump with a full fuel tank and no cargo.",
        |s, c| jump(s, 0.0, s.stats.fuel_capacity, c)
    ),
    metric!(
        "ladenRange",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb"]),
        "Single jump with full fuel tank and every cargo rack full.",
        |s, c| jump(s, s.stats.cargo_capacity, s.stats.fuel_capacity, c)
    ),
    metric!(
        "totalRangeUnladen",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb", "ft"]),
        "Total distance on one tank of fuel (consecutive jumps) with no cargo.",
        |s, c| total_range(s, 0.0, c)
    ),
    metric!(
        "totalRangeLaden",
        "Ly",
        Higher,
        Groups::Fixed(&["gfsb", "ft"]),
        "Total distance on one tank of fuel with full cargo.",
        |s, c| total_range(s, s.stats.cargo_capacity, c)
    ),
    metric!(
        "topSpeed",
        "m/s",
        Higher,
        Groups::None,
        "Top speed (4 pips ENG, unladen, full fuel).",
        |s, _| s.stats.top_speed
    ),
    metric!(
        "topBoost",
        "m/s",
        Higher,
        Groups::None,
        "Boost speed (4 pips ENG, unladen, full fuel). 0 when the distributor cannot boost.",
        |s, _| if s.can_boost(0.0, s.stats.fuel_capacity) {
            s.stats.top_boost
        } else {
            0.0
        }
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
        "Main + internal fuel tank capacity.",
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
        "Hull + modules + full fuel + full cargo.",
        |s, _| s.stats.laden_mass
    ),
    metric!(
        "totalCost",
        "CR",
        Lower,
        Groups::None,
        "Purchase price of hull and modules (no discounts, engineering not priced).",
        |s, _| s.stats.total_cost
    ),
    metric!(
        "powerMargin",
        "MW",
        Higher,
        Groups::None,
        "Power plant output minus power draw with hardpoints deployed.",
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
        "Simultaneously active collector limpets (collector + multi-limpet controllers).",
        |s, _| sum_stat(s, "cc", |r| r.maximum) + sum_stat(s, "mlc", |r| r.maximum)
    ),
    // --- defence
    metric!(
        "shieldRaw",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD),
        "Shield strength (generator + boosters + guardian SRPs), no resistances.",
        |s, c| shield_view(s, c, c.sys).raw
    ),
    metric!(
        "shieldEHP",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD),
        "Effective shield HP against the context damage mix at the context SYS pips (resistances incl. diminishing returns).",
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
        "Seconds to recharge shields from 50% to 100% at the recovery SYS pips (context.recoverySys).",
        |s, c| shield_view(s, c, c.recovery_sys).recharge
    ),
    metric!(
        "shieldRecover",
        "s",
        Lower,
        Groups::Fixed(SHIELD),
        "Seconds from shields broken back to 50% at the recovery SYS pips (includes 16 s delay).",
        |s, c| shield_view(s, c, c.recovery_sys).recover
    ),
    metric!(
        "shieldRegen",
        "MJ/s",
        Higher,
        Groups::Fixed(SHIELD),
        "Shield regeneration the distributor can sustain indefinitely at the fighting SYS pips.",
        |s, c| sustained_regen(s, c)
    ),
    metric!(
        "shieldSustainDps",
        "dps",
        Higher,
        Groups::Fixed(SHIELD),
        "Raw incoming DPS (context damage mix and SYS pips) the shields absorb indefinitely: sustained regeneration divided by the damage multiplier. Rewards high resistances, which multiply the value of every MJ regenerated.",
        |s, c| {
            let v = shield_view(s, c, c.sys);
            if v.raw == 0.0 {
                0.0
            } else {
                sustained_regen(s, c) / v.mult
            }
        }
    ),
    metric!(
        "shieldTimeUnderFire",
        "s",
        Higher,
        Groups::Fixed(SHIELD),
        "Seconds until shields break under context.incomingDps raw damage (context mix and SYS pips); regeneration counts for the context.regenUnderFire share of the time (default 0). 1e6 = never.",
        |s, c| shield_time_under_fire(s, c, false)
    ),
    metric!(
        "shieldTimeUnderFireWithCells",
        "s",
        Higher,
        Groups::Fixed(SHIELD_CELLS),
        "shieldTimeUnderFire with all shield cell bank MJ added to the pool (ignores spin-up and heat).",
        |s, c| shield_time_under_fire(s, c, true)
    ),
    metric!(
        "shieldEHPWorstType",
        "MJ",
        Higher,
        Groups::Fixed(SHIELD),
        "Effective shield HP against whichever of kinetic/thermal/explosive the shield resists least, at context SYS pips. Exposes the stock -20% thermal hole that a mixed average hides.",
        |s, c| shield_ehp_worst_type(s, c)
    ),
    metric!(
        "hullRaw",
        "HP",
        Higher,
        Groups::Fixed(HULL),
        "Armour (bulkheads + hull reinforcements), no resistances.",
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
        "shieldEHP + hullEHP: total effective damage absorbed before destruction (no cells).",
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
        "Total integrity of module reinforcement packages: the damage they absorb before they are used up (protection share is moduleProtection).",
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
        "Raw sustained DPS of all weapons including reloads (no target resistances).",
        |s, _| s.stats.total_sdps
    ),
    metric!(
        "burstDps",
        "dps",
        Higher,
        Groups::Weapons,
        "Raw burst DPS of all weapons (no reloads).",
        |s, _| s.stats.total_dps
    ),
    metric!(
        "dpsVsShields",
        "dps",
        Higher,
        Groups::Weapons,
        "Sustained DPS against the reference opponent's shields at context range (resistances, falloff).",
        |s, c| vs_target(s, c).0
    ),
    metric!(
        "dpsVsHull",
        "dps",
        Higher,
        Groups::Weapons,
        "Sustained DPS against the reference opponent's hull at context range (resistances, hardness/piercing, falloff, Corrosive Shell debuff).",
        |s, c| vs_target(s, c).1
    ),
    metric!(
        "pierceEfficiency",
        "fraction",
        Higher,
        Groups::Weapons,
        "DPS-weighted share of hull damage that survives the reference opponent's armour hardness (piercing/hardness, capped at 1). Rewards Focused engineering and piercing-100 weapons against big hulls.",
        |s, c| pierce_efficiency(s, c)
    ),
    metric!(
        "timeToKill",
        "s",
        Lower,
        Groups::Weapons,
        "Seconds to strip the reference opponent's shields (at its SYS pips) and then its hull at the context range; opponent regeneration ignored. 1e6 = cannot.",
        |s, c| time_to_kill(s, c)
    ),
    metric!(
        "boostInterval",
        "s",
        Lower,
        Groups::None,
        "Seconds between boosts at 4 ENG pips (distributor refill or the hull's boost cooldown, whichever is longer). 1e6 = cannot boost.",
        |s, _| boost_interval(s)
    ),
    metric!(
        "weaponSustain",
        "s",
        Higher,
        Groups::None,
        "Seconds of continuous fire before the WEP capacitor is empty at context WEP pips (capped at 1e6 = sustainable).",
        |s, c| bounded(calc::time_to_drain_wep(s, c.wep))
    ),
    metric!(
        "emptyHardpoints",
        "slots",
        Lower,
        Groups::None,
        "Weapon hardpoints left empty. Constrain to 0 for a fully armed ship.",
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
        "powerAvailable",
        "MW",
        Higher,
        Groups::None,
        "Power plant output.",
        |s, _| s.stats.power_available
    ),
    metric!(
        "combatPowerDraw",
        "MW",
        Lower,
        Groups::None,
        "Power drawn in a fight: every combat module with hardpoints deployed (non-combat modules shed by priority).",
        |s, _| combat_deployed_draw(s)
    ),
    metric!(
        "powerPlantHeat",
        "heat/s",
        Lower,
        Groups::None,
        "Heat the power plant generates in a fight: combat power draw x the plant's heat efficiency. Low Emissions, Armoured and Thermal Spread lower it; Overcharged raises it.",
        |s, _| power_plant_heat(s)
    ),
    metric!(
        "combatHeat",
        "heat/s",
        Lower,
        Groups::None,
        "Net heat per second in a fight: power plant heat plus weapon heat, minus what Thermal Vent beams remove while on target (context.ventHitRate, context.thermalVentFactor). Negative = the ship cools while firing. Thrusters and FSD charging are not included (no data); the hull's own dissipation is not modelled, so compare builds of the same hull.",
        |s, c| heat_budget(s, c).net
    ),
    metric!(
        "thermalVentRemoval",
        "heat/s",
        Higher,
        Groups::None,
        "Heat per second Thermal Vent beams remove while on target (assumed thermalVentFactor x their thermal load, reduced by falloff).",
        |s, c| heat_budget(s, c).vent_removal
    ),
    metric!(
        "thermalVentBreakEven",
        "x",
        Lower,
        Groups::None,
        "Thermal Vent strength (multiple of the vent beams' own thermal load) needed to cancel all other heat in a fight. If your ship reaches 0 heat with vents on target, the real factor is at least this. 1e6 = no vent beams.",
        |s, c| heat_budget(s, c).break_even
    ),
    metric!(
        "weaponTypes",
        "types",
        Lower,
        Groups::None,
        "Number of distinct weapon types fitted (e.g. plasma accelerators + multi-cannons = 2). Cap it to keep a loadout with matching ballistics that is easy to aim and manage.",
        |s, _| distinct(weapons(s).map(Module::grp))
    ),
    metric!(
        "mountTypes",
        "types",
        Lower,
        Groups::None,
        "Number of distinct mount types among weapons (fixed, gimballed, turreted). 1 = all weapons share one mount type.",
        |s, _| distinct(weapons(s).map(|m| m.def.mount.as_deref().unwrap_or("-")))
    ),
    metric!(
        "weaponHeat",
        "heat/s",
        Lower,
        Groups::None,
        "Heat generated per second by all weapons firing (Coriolis thermal load). Cap it at a proven build's value to avoid overheating loadouts.",
        |s, _| s.stats.total_hps
    ),
    metric!(
        "ammoEndurance",
        "s",
        Higher,
        Groups::None,
        "Seconds of continuous fire until the first ammunition-limited weapon runs completely dry (magazine + reserve; 1e6 = no ammo-limited weapons). Pair with DPS goals so finite ordnance is not mistaken for sustained damage.",
        |s, _| ammo_endurance(s)
    ),
    metric!(
        "axSustainedDps",
        "dps",
        Higher,
        Groups::ExperimentalWeapons,
        "Raw sustained DPS from AX/Guardian (experimental) weapons only — the damage that counts against Thargoid interceptors.",
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
        // Full tank, no cargo: dry mass + tank (reserve excluded by default), one maximum jump.
        let expected = calc::jump_range(
            &ship,
            ship.stats.dry_mass + fuel - ship.def.properties.reserve_fuel_capacity,
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
        assert_eq!(
            zero.kinetic + zero.thermal + zero.explosive + zero.absolute,
            0.0
        );
        assert!(DamageProfile::preset("pve").is_some() && DamageProfile::preset("x").is_none());
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
