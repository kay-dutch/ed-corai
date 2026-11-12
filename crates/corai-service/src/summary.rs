//! Compact, model-friendly summary of a build (port of `src/describe.ts`). Serialises to the same
//! JSON as the Node server.

use corai_engine::{Ship, calc, interchange};
use corai_opt::catalog::SlotRef;
use corai_opt::metrics::{Metric, MetricContext};
use corai_opt::validate::validate;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use crate::labels::{engineering_label, module_label};
use crate::num::{json_num, round_metric, to_fixed};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSummary {
    /// Ship display name, e.g. `Anaconda Mk II`.
    pub ship: String,
    pub ship_id: String,
    pub coriolis_url: String,
    pub edsy_url: String,
    pub modules: Vec<ModuleRow>,
    /// Every metric, rounded for display (`null` when undefined for this build).
    pub metrics: IndexMap<String, Value>,
    pub checks: Checks,
    /// Modules or engineering Coriolis itself would not allow, and import discrepancies.
    pub issues: Vec<String>,
    /// Resistances in % as the outfitting panel shows them (0 SYS pips).
    pub resistances: Resistances,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleRow {
    pub slot: String,
    /// Slot size: class (with restriction), `U` for utility mounts, `-` for bulkheads.
    pub size: String,
    pub module: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engineering: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mass: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checks {
    pub power_ok: bool,
    pub power_ok_retracted: bool,
    pub can_thrust_laden: bool,
    pub can_boost: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resistances {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shield: Option<Typed>,
    pub hull: Typed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Typed {
    pub kinetic: Value,
    pub thermal: Value,
    pub explosive: Value,
}

/// Options for [`summarize`].
#[derive(Debug, Clone, Default)]
pub struct SummaryOptions<'a> {
    /// List empty slots too (ship details).
    pub include_empty: bool,
    /// Build name for the links.
    pub build_name: Option<&'a str>,
}

fn slot_size(ship: &Ship, r: SlotRef) -> String {
    match r {
        SlotRef::Bulkheads => "-".into(),
        _ => {
            let slot = r.slot(ship).expect("slot refs come from this ship");
            if r.kind_name(ship) == "utility" {
                "U".into()
            } else {
                match slot.name {
                    Some(n) => format!("{} ({n})", slot.max_class),
                    None => slot.max_class.to_string(),
                }
            }
        }
    }
}

fn resistance(mult: f64) -> Value {
    json_num(corai_engine::jsmath::js_round((1.0 - mult) * 1000.0) / 10.0)
}

/// The summary every tool returns for a build.
pub fn summarize(
    ship: &Ship,
    ctx: &MetricContext,
    opts: &SummaryOptions<'_>,
    import_issues: &[String],
) -> BuildSummary {
    let mut modules = Vec::new();
    for r in SlotRef::all(ship) {
        let m = match r {
            SlotRef::Bulkheads => Some(&ship.bulkhead),
            _ => r.slot(ship).and_then(|s| s.module.as_ref()),
        };
        if m.is_none() && !opts.include_empty {
            continue;
        }
        let module = match r {
            SlotRef::Bulkheads => ship.bulkhead.def.name.clone().unwrap_or_else(|| {
                ship.def
                    .bulkheads
                    .first()
                    .and_then(|b| b.name.clone())
                    .unwrap_or_default()
            }),
            _ => module_label(m),
        };
        modules.push(ModuleRow {
            slot: r.key(ship),
            size: slot_size(ship, r),
            module,
            engineering: m.and_then(engineering_label),
            power: m
                .and_then(|m| m.get("power"))
                .filter(|p| *p != 0.0)
                .map(|p| json_num(to_fixed(p, 2))),
            mass: m
                .and_then(|m| m.get("mass"))
                .map(|x| json_num(to_fixed(x, 2))),
        });
    }
    let metrics = Metric::catalog()
        .map(|m| (m.name(), json_num(round_metric(m.compute(ship, ctx)))))
        .collect();
    let s = &ship.stats;
    let max_mass = ship.thrusters().and_then(|t| t.r().max_mass).unwrap_or(0.0);
    let mut issues = validate(ship);
    issues.extend(import_issues.iter().map(|i| format!("import: {i}")));
    let shield = calc::shield_metrics(ship, 0.0)
        .filter(|m| m.generator != 0.0)
        .map(|m| Typed {
            kinetic: resistance(m.kinetic.total),
            thermal: resistance(m.thermal.total),
            explosive: resistance(m.explosive.total),
        });
    let a = calc::armour_metrics(ship);
    let name = opts.build_name.unwrap_or_default();
    BuildSummary {
        ship: ship.def.properties.name.clone(),
        ship_id: ship.id.to_string(),
        coriolis_url: interchange::coriolis_url(ship, opts.build_name),
        edsy_url: interchange::edsy_url(ship, name),
        modules,
        metrics,
        checks: Checks {
            power_ok: s.power_deployed <= s.power_available,
            power_ok_retracted: s.power_retracted <= s.power_available,
            can_thrust_laden: s.laden_mass < max_mass,
            can_boost: ship.can_boost(s.cargo_capacity, s.fuel_capacity),
        },
        issues,
        resistances: Resistances {
            shield,
            hull: Typed {
                kinetic: resistance(a.kinetic.total),
                thermal: resistance(a.thermal.total),
                explosive: resistance(a.explosive.total),
            },
        },
    }
}
