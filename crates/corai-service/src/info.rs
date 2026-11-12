//! Read-only tools: ships, slot layouts, module groups, module stats, metrics. Data values pass
//! through from the raw data set so numbers and key order match the Node server.

use corai_data::{DATA, RAW};
use corai_opt::catalog::SlotRef;
use corai_opt::metrics::{Better, Metric};
use serde_json::{Map, Value, json};

use crate::changes::max_blueprint_grade;
use crate::context::{ContextSpec, build_context};
use crate::error::ServiceError;
use crate::names::group_name;
use crate::ships::{load_base, ship_ids, stock_ship};
use crate::summary::{SummaryOptions, summarize};

fn raw_ship(id: &str) -> &'static Value {
    &RAW["ships"][id]
}

/// Slot layout of a stock hull: hardpoint and internal sizes (with restrictions), utility count,
/// standard slots as group+class.
fn slot_layout(id: &str) -> Result<Value, ServiceError> {
    let ship = stock_ship(id)?;
    let sizes = |kind: &str| -> Vec<Value> {
        SlotRef::all(&ship)
            .into_iter()
            .filter(|r| r.kind_name(&ship) == kind)
            .filter_map(|r| r.slot(&ship))
            .map(|s| {
                Value::from(format!(
                    "{}{}",
                    s.max_class,
                    s.name.map(|n| format!("({n})")).unwrap_or_default()
                ))
            })
            .collect()
    };
    let standard: Vec<Value> = SlotRef::all(&ship)
        .into_iter()
        .filter(|r| matches!(r, SlotRef::Standard(_)))
        .filter_map(|r| {
            let s = r.slot(&ship)?;
            Some(Value::from(format!(
                "{}{}",
                r.key(&ship).trim_start_matches("standard:"),
                s.max_class
            )))
        })
        .collect();
    Ok(json!({
        "hardpoints": sizes("hardpoint"),
        "utility": sizes("utility").len(),
        "internal": sizes("internal"),
        "standard": standard,
    }))
}

/// `list_ships`.
///
/// # Errors
/// Never in practice (every listed ship exists).
pub fn list_ships() -> Result<Value, ServiceError> {
    let mut out = Vec::new();
    for id in ship_ids() {
        let raw = raw_ship(id);
        let p = &raw["properties"];
        let mut row = Map::new();
        row.insert("id".into(), id.into());
        for (to, from) in [
            ("name", "name"),
            ("manufacturer", "manufacturer"),
            ("size", "class"),
        ] {
            row.insert(to.into(), p[from].clone());
        }
        row.insert("hullCost".into(), raw["retailCost"].clone());
        for (to, from) in [
            ("speed", "speed"),
            ("boost", "boost"),
            ("hullMass", "hullMass"),
            ("masslock", "masslock"),
            ("baseShield", "baseShieldStrength"),
            ("baseArmour", "baseArmour"),
        ] {
            row.insert(to.into(), p[from].clone());
        }
        if let Value::Object(layout) = slot_layout(id)? {
            row.extend(layout);
        }
        out.push(Value::Object(row));
    }
    Ok(Value::Array(out))
}

/// `ship_details`: properties, bulkheads, every slot key with size and restriction, and the full
/// summary of the stock (or given) build.
///
/// # Errors
/// Unknown ship, unreadable build or context.
pub fn ship_details(
    ship: Option<&str>,
    build: Option<&str>,
    context: Option<&ContextSpec>,
) -> Result<Value, ServiceError> {
    let (s, issues) = load_base(ship, build)?;
    let ctx = build_context(context)?;
    let raw = raw_ship(s.id);
    let bulkheads: Vec<Value> = raw["bulkheads"]
        .as_array()
        .map(|bs| {
            bs.iter()
                .enumerate()
                .map(|(i, b)| {
                    let mut row = Map::new();
                    row.insert("index".into(), i.into());
                    for k in ["name", "mass", "hullboost"] {
                        if let Some(v) = b.get(k) {
                            row.insert(k.into(), v.clone());
                        }
                    }
                    Value::Object(row)
                })
                .collect()
        })
        .unwrap_or_default();
    let slots: Vec<Value> = SlotRef::all(&s)
        .into_iter()
        .map(|r| {
            let mut row = Map::new();
            row.insert("slot".into(), r.key(&s).into());
            let slot = r.slot(&s);
            row.insert("size".into(), slot.map_or(0, |x| x.max_class).into());
            if let Some(slot) = slot.filter(|_| !matches!(r, SlotRef::Standard(_))) {
                if let Some(n) = slot.name {
                    row.insert("restriction".into(), n.into());
                }
                if let Some(e) = slot.eligible {
                    row.insert(
                        "eligibleGroups".into(),
                        e.keys().cloned().collect::<Vec<_>>().into(),
                    );
                }
            }
            Value::Object(row)
        })
        .collect();
    let summary = summarize(
        &s,
        &ctx,
        &SummaryOptions {
            include_empty: true,
            build_name: None,
        },
        &issues,
    );
    Ok(json!({
        "properties": raw["properties"].clone(),
        "bulkheads": bulkheads,
        "slots": slots,
        "build": serde_json::to_value(summary).unwrap_or_default(),
    }))
}

/// One module group: code, display name, category and number of modules.
fn groups() -> Vec<Value> {
    let data = &*DATA;
    let mut out = Vec::new();
    let name = |g: &str, first: Option<&corai_data::ModuleDef>, fallback_uk: bool| -> String {
        group_name(g)
            .map(str::to_string)
            .or_else(|| {
                fallback_uk
                    .then(|| first.and_then(|m| m.uk_name.clone()))
                    .flatten()
            })
            .unwrap_or_else(|| g.to_string())
    };
    for (g, ms) in &data.modules.standard {
        out.push(json!({ "group": g, "name": name(g, ms.first(), false), "category": "standard", "count": ms.len() }));
    }
    for (g, ms) in &data.modules.hardpoints {
        let category = if ms.iter().all(|m| m.class == 0) {
            "utility"
        } else {
            "hardpoint"
        };
        out.push(json!({ "group": g, "name": name(g, ms.first(), true), "category": category, "count": ms.len() }));
    }
    for (g, ms) in &data.modules.internal {
        out.push(json!({ "group": g, "name": name(g, ms.first(), true), "category": "internal", "count": ms.len() }));
    }
    out
}

/// `list_module_groups`, optionally one category (standard, hardpoint, utility, internal).
pub fn list_module_groups(category: Option<&str>) -> Value {
    Value::Array(
        groups()
            .into_iter()
            .filter(|g| category.is_none_or(|c| g["category"] == c))
            .collect(),
    )
}

/// Whether a module group code exists (any category).
pub fn group_exists(grp: &str) -> bool {
    let d = &DATA.modules;
    d.standard.contains_key(grp) || d.hardpoints.contains_key(grp) || d.internal.contains_key(grp)
}

const MODULE_FIELDS: &[&str] = &[
    "id",
    "class",
    "rating",
    "mount",
    "missile",
    "name",
    "mass",
    "power",
    "cost",
    "integrity",
    "pgen",
    "optmass",
    "maxmass",
    "maxfuel",
    "fuelmul",
    "engcap",
    "engrate",
    "syscap",
    "sysrate",
    "wepcap",
    "weprate",
    "shieldboost",
    "explres",
    "kinres",
    "thermres",
    "hullreinforcement",
    "damage",
    "fireint",
    "clip",
    "reload",
    "distdraw",
    "thermload",
    "range",
    "falloff",
    "piercing",
    "cargo",
    "passengers",
    "fuel",
    "bins",
    "jumpboost",
    "shieldreinforcement",
    "shieldaddition",
    "experimental",
    "pp",
    "preEngineered",
];

/// `describeEngineering`: blueprints (max grade and its feature ranges) and experimentals of a group.
fn describe_engineering(grp: &str) -> Value {
    let mods = &RAW["modifications"];
    let Some(group) = mods["modules"].get(grp) else {
        return json!({ "blueprints": [], "specials": [] });
    };
    let blueprints: Vec<Value> = group["blueprints"]
        .as_object()
        .map(|bps| {
            bps.keys()
                .filter(|bp| mods["blueprints"].get(bp.as_str()).is_some())
                .map(|bp| {
                    let def = &mods["blueprints"][bp.as_str()];
                    let grade = max_blueprint_grade(grp, bp);
                    let mut row = Map::new();
                    row.insert("key".into(), bp.as_str().into());
                    row.insert(
                        "name".into(),
                        def.get("name")
                            .cloned()
                            .unwrap_or_else(|| bp.as_str().into()),
                    );
                    row.insert("maxGrade".into(), grade.into());
                    if let Some(f) = def["grades"]
                        .get(grade.to_string())
                        .and_then(|g| g.get("features"))
                    {
                        row.insert("features".into(), f.clone());
                    }
                    Value::Object(row)
                })
                .collect()
        })
        .unwrap_or_default();
    let specials: Vec<Value> = group["specials"]
        .as_array()
        .map(|ss| {
            ss.iter()
                .filter_map(Value::as_str)
                .filter_map(|s| mods["specials"].get(s).map(|def| (s, def)))
                .map(|(s, def)| {
                    let mut row = Map::new();
                    row.insert("key".into(), s.into());
                    row.insert("name".into(), def["name"].clone());
                    if let Some(d) = def.get("description") {
                        row.insert("description".into(), d.clone());
                    }
                    Value::Object(row)
                })
                .collect()
        })
        .unwrap_or_default();
    json!({ "blueprints": blueprints, "specials": specials })
}

/// `module_info`: stock stats of every module in a group (optionally one class) and its engineering.
///
/// # Errors
/// Unknown group.
pub fn module_info(group: &str, class: Option<u64>) -> Result<Value, ServiceError> {
    let modules = &RAW["modules"];
    let all: Vec<&Value> = ["standard", "hardpoints", "internal"]
        .iter()
        .filter_map(|c| modules[*c].get(group).and_then(Value::as_array))
        .flatten()
        .collect();
    if all.is_empty() {
        return Err(ServiceError::UnknownGroup(group.into()));
    }
    let rows: Vec<Value> = all
        .into_iter()
        .filter(|m| class.is_none_or(|c| m["class"].as_u64() == Some(c)))
        .map(|m| {
            let mut row = Map::new();
            for k in MODULE_FIELDS {
                if let Some(v) = m.get(*k) {
                    let v = if *k == "preEngineered" {
                        v["description"].clone()
                    } else {
                        v.clone()
                    };
                    row.insert((*k).into(), v);
                }
            }
            Value::Object(row)
        })
        .collect();
    Ok(json!({ "modules": rows, "engineering": describe_engineering(group) }))
}

/// `list_metrics`: every metric with unit, direction and meaning, plus the assumptions in effect.
pub fn list_metrics() -> Value {
    let metrics: Vec<Value> = Metric::catalog()
        .map(|m| {
            json!({
                "name": m.name(),
                "unit": m.unit(),
                "better": if m.better() == Better::Higher { "higher" } else { "lower" },
                "description": m.description(),
            })
        })
        .collect();
    json!({
        "metrics": metrics,
        "parametrised": ["count:<group> = number of fitted modules of a group", "class:<group> = sum of classes of fitted modules of a group"],
        "context": "Effective-HP metrics use context.damage/damagePreset and sysPips; shieldTimeUnderFire uses incomingDps and regenUnderFire; dpsVs*, pierceEfficiency and timeToKill use context.opponent and rangeMeters; weaponSustain uses wepPips; jump ranges use reserveFuelInJumpMass.",
        "assumptions": "Defaults that no published measurement settles: the pve damage mix, regenUnderFire=0, thermalVentFactor=1, the Corrosive Shell 1.25 hull multiplier and +20 piercing (community figures), reserve fuel excluded from jump mass, Coriolis hull-resistance stacking. Rerun with the alternative when a ranking is close.",
    })
}
