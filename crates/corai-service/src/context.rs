//! The `context` argument shared by the tools: how the player flies and what they fight.

use corai_engine::{Ship, interchange};
use corai_opt::metrics::{ContextOptions, DamageProfile, MetricContext};
use serde::Deserialize;

use crate::error::ServiceError;
use crate::ships::stock_ship;

/// Relative weights of incoming damage; missing weights come from the preset.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DamageSpec {
    pub kinetic: Option<f64>,
    pub thermal: Option<f64>,
    pub explosive: Option<f64>,
    pub absolute: Option<f64>,
}

/// Assumptions the metrics are evaluated under (all optional; defaults as documented).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSpec {
    pub sys_pips: Option<f64>,
    pub wep_pips: Option<f64>,
    pub recovery_sys_pips: Option<f64>,
    pub damage: Option<DamageSpec>,
    /// Reference target: a ship id (stock) or any build (link, SLEF).
    pub opponent: Option<String>,
    pub opponent_sys_pips: Option<f64>,
    pub range_meters: Option<f64>,
    pub incoming_dps: Option<f64>,
    pub damage_preset: Option<String>,
    pub regen_under_fire: Option<f64>,
    pub vent_hit_rate: Option<f64>,
    pub thermal_vent_factor: Option<f64>,
    pub corrosive_hull_multiplier: Option<f64>,
    pub corrosive_piercing_bonus: Option<f64>,
    pub reserve_fuel_in_jump_mass: Option<bool>,
}

fn resolve_opponent(spec: &str) -> Result<Ship, ServiceError> {
    if spec.contains(['/', '{', '[']) || spec.contains("code=") {
        Ok(interchange::parse_build(spec, None)?.ship)
    } else {
        stock_ship(spec)
    }
}

/// The metric context for a tool call (`buildContext`).
///
/// # Errors
/// Unknown damage preset or opponent.
pub fn build_context(spec: Option<&ContextSpec>) -> Result<MetricContext, ServiceError> {
    let empty = ContextSpec::default();
    let c = spec.unwrap_or(&empty);
    let preset_name = c.damage_preset.as_deref().unwrap_or("even");
    let preset = DamageProfile::preset(preset_name)
        .ok_or_else(|| ServiceError::UnknownPreset(preset_name.into()))?;
    // `{ ...preset, ...damage }`: given weights replace the preset's one by one.
    let damage = c.damage.as_ref().map(|d| DamageProfile {
        kinetic: d.kinetic.unwrap_or(preset.kinetic),
        thermal: d.thermal.unwrap_or(preset.thermal),
        explosive: d.explosive.unwrap_or(preset.explosive),
        absolute: d.absolute.unwrap_or(preset.absolute),
    });
    let opponent = c
        .opponent
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(resolve_opponent)
        .transpose()?;
    MetricContext::from_options(ContextOptions {
        sys: c.sys_pips,
        wep: c.wep_pips,
        recovery_sys: c.recovery_sys_pips,
        damage_preset: Some(preset_name.to_string()),
        damage,
        opponent,
        opponent_sys: c.opponent_sys_pips,
        range: c.range_meters,
        incoming_dps: c.incoming_dps,
        regen_under_fire: c.regen_under_fire,
        reserve_fuel_in_jump_mass: c.reserve_fuel_in_jump_mass,
        vent_hit_rate: c.vent_hit_rate,
        thermal_vent_factor: c.thermal_vent_factor,
        corrosive_hull_multiplier: c.corrosive_hull_multiplier,
        corrosive_piercing_bonus: c.corrosive_piercing_bonus,
    })
    .map_err(|_| ServiceError::UnknownPreset(preset_name.into()))
}
