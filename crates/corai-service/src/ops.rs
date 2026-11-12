//! Tools that work on a build: evaluate, export, optimise one hull, compare hulls.

use std::time::{Duration, Instant};

use corai_engine::{Ship, interchange, journal};
use corai_opt::catalog::{Filters, SlotRef};
use corai_opt::metrics::{Metric, MetricContext, MetricError};
use corai_opt::search::{self, Constraint, Direction, Effort, Goal, Outcome, PowerCheck, Request};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::changes::{BuildChange, apply_change};
use crate::context::{ContextSpec, build_context};
use crate::error::ServiceError;
use crate::num::{json_num, round_metric};
use crate::ships::{load_base, resolve_ship_id, ship_ids, stock_ship};
use crate::summary::{SummaryOptions, summarize};

/// `evaluate_build` arguments.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvaluateArgs {
    pub ship: Option<String>,
    pub build: Option<String>,
    #[serde(default)]
    pub changes: Vec<BuildChange>,
    pub context: Option<ContextSpec>,
}

/// Full stats of a build, optionally after slot changes.
///
/// # Errors
/// Unknown ship, unreadable build, bad change or context.
pub fn evaluate_build(a: &EvaluateArgs) -> Result<Value, ServiceError> {
    let (mut ship, issues) = load_base(a.ship.as_deref(), a.build.as_deref())?;
    for c in &a.changes {
        apply_change(&mut ship, c)?;
    }
    let ctx = build_context(a.context.as_ref())?;
    Ok(
        serde_json::to_value(summarize(&ship, &ctx, &SummaryOptions::default(), &issues))
            .unwrap_or_default(),
    )
}

/// `export_build` arguments.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExportArgs {
    pub ship: Option<String>,
    pub build: Option<String>,
    /// `slef` (default) or `loadout`.
    pub format: Option<String>,
    pub name: Option<String>,
}

/// A build as SLEF or a journal Loadout event, plus Coriolis and EDSY links.
///
/// # Errors
/// Unknown ship or unreadable build.
pub fn export_build(a: &ExportArgs) -> Result<Value, ServiceError> {
    let (ship, _) = load_base(a.ship.as_deref(), a.build.as_deref())?;
    let name = a.name.as_deref().unwrap_or_default();
    let loadout = a.format.as_deref() == Some("loadout");
    let data = if loadout {
        journal::to_loadout(&ship, name)
    } else {
        interchange::to_slef(&ship, name)
    };
    Ok(json!({
        "format": a.format.as_deref().unwrap_or("slef"),
        "json": data.to_string(),
        "coriolisUrl": interchange::coriolis_url(&ship, a.name.as_deref()),
        "edsyUrl": interchange::edsy_url(&ship, name),
    }))
}

/// A goal as tool arguments give it.
#[derive(Debug, Clone, Deserialize)]
pub struct GoalSpec {
    pub metric: String,
    /// `max` or `min`; defaults to the metric's natural direction.
    pub direction: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConstraintSpec {
    pub metric: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterSpec {
    pub allow_powerplay: Option<bool>,
    pub allow_pre_engineered: Option<bool>,
    pub allow_community_goal: Option<bool>,
    pub allow_guardian: Option<bool>,
    pub allow_experimental: Option<bool>,
    #[serde(default)]
    pub exclude_groups: Vec<String>,
    #[serde(default)]
    pub exclude_modules: Vec<String>,
}

/// Search options shared by `optimize_build` and `compare_ships`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub objective: GoalSpec,
    #[serde(default)]
    pub tiebreakers: Vec<GoalSpec>,
    #[serde(default)]
    pub constraints: Vec<ConstraintSpec>,
    /// `full` (default) or `none`.
    pub engineering: Option<String>,
    /// `priority` (default), `deployed`, `retracted` or `ignore`.
    pub power: Option<String>,
    pub require_boost: Option<bool>,
    #[serde(default)]
    pub extra_groups: Vec<String>,
    pub filters: Option<FilterSpec>,
    pub context: Option<ContextSpec>,
    /// `quick`, `normal` (default) or `thorough`.
    pub effort: Option<String>,
    pub seed: Option<u64>,
}

fn metric(name: &str) -> Result<Metric, ServiceError> {
    name.parse().map_err(|e| match e {
        MetricError::UnknownGroup { metric, group } => {
            ServiceError::UnknownMetricGroup { metric, group }
        }
        _ => ServiceError::UnknownMetric(name.into()),
    })
}

fn goal(g: &GoalSpec) -> Result<Goal, ServiceError> {
    let m = metric(&g.metric)?;
    let natural = Goal::natural(m);
    Ok(match g.direction.as_deref() {
        Some("max") => Goal {
            direction: Direction::Max,
            ..natural
        },
        Some("min") => Goal {
            direction: Direction::Min,
            ..natural
        },
        _ => natural,
    })
}

/// A search request from tool arguments.
fn request(base: Ship, o: &SearchOptions, default_effort: Effort) -> Result<Request, ServiceError> {
    let mut req = Request::new(base, goal(&o.objective)?);
    req.tiebreakers = o.tiebreakers.iter().map(goal).collect::<Result<_, _>>()?;
    req.constraints = o
        .constraints
        .iter()
        .map(|c| {
            Ok(Constraint {
                metric: metric(&c.metric)?,
                min: c.min,
                max: c.max,
            })
        })
        .collect::<Result<_, ServiceError>>()?;
    req.engineering = o.engineering.as_deref() != Some("none");
    req.power = match o.power.as_deref() {
        Some("deployed") => PowerCheck::Deployed,
        Some("retracted") => PowerCheck::Retracted,
        Some("ignore") => PowerCheck::Ignore,
        _ => PowerCheck::Priority,
    };
    req.require_boost = o.require_boost.unwrap_or(true);
    req.extra_groups = o
        .extra_groups
        .iter()
        .map(|g| {
            corai_opt::metrics::group_name(g)
                .ok_or_else(|| ServiceError::UnknownSearchGroup(g.clone()))
        })
        .collect::<Result<_, _>>()?;
    if let Some(f) = &o.filters {
        req.filters = Filters {
            allow_powerplay: f.allow_powerplay.unwrap_or(true),
            allow_pre_engineered: f.allow_pre_engineered.unwrap_or(true),
            allow_community_goal: f.allow_community_goal.unwrap_or(true),
            allow_guardian: f.allow_guardian.unwrap_or(true),
            allow_experimental: f.allow_experimental.unwrap_or(true),
            exclude_groups: f.exclude_groups.clone(),
            exclude_modules: f.exclude_modules.clone(),
        };
    }
    req.effort = match o.effort.as_deref() {
        Some("quick") => Effort::Quick,
        Some("thorough") => Effort::Thorough,
        Some("normal") => Effort::Normal,
        _ => default_effort,
    };
    if let Some(seed) = o.seed {
        req.seed = seed;
    }
    Ok(req)
}

/// The objective, tiebreaker and constraint metrics of a result (`scoreSummary`).
fn key_figures(req: &Request, ship: &Ship, ctx: &MetricContext) -> IndexMap<String, Value> {
    let mut out = IndexMap::new();
    let metrics = std::iter::once(req.objective.metric)
        .chain(req.tiebreakers.iter().map(|g| g.metric))
        .chain(req.constraints.iter().map(|c| c.metric));
    for m in metrics {
        out.entry(m.name())
            .or_insert_with(|| json_num(round_metric(m.compute(ship, ctx))));
    }
    out
}

fn links(ship: &Ship) -> (String, String) {
    (
        interchange::coriolis_url(ship, None),
        interchange::edsy_url(ship, ""),
    )
}

/// `optimize_build` arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeArgs {
    pub ship: Option<String>,
    pub build: Option<String>,
    #[serde(default)]
    pub lock: Vec<String>,
    pub keep_base: Option<bool>,
    pub time_limit_seconds: Option<f64>,
    #[serde(flatten)]
    pub search: SearchOptions,
}

fn outcome_json(req: &Request, out: &Outcome, ctx: &MetricContext, ms: u128) -> Value {
    let alternatives: Vec<Value> = out
        .alternatives
        .iter()
        .map(|(ship, _)| {
            let (c, e) = links(ship);
            json!({ "key": key_figures(req, ship, ctx), "coriolisUrl": c, "edsyUrl": e })
        })
        .collect();
    json!({
        "feasible": out.feasible,
        "violations": out.violations,
        "key": key_figures(req, &out.ship, ctx),
        "best": serde_json::to_value(summarize(&out.ship, ctx, &SummaryOptions::default(), &[])).unwrap_or_default(),
        "alternatives": alternatives,
        "search": {
            "evaluations": out.stats.evaluations,
            "ms": ms,
            "starts": out.stats.starts,
            "timedOut": out.stats.timed_out,
            "searchedGroups": out.stats.searched_groups,
        },
    })
}

/// Best outfitting of one ship for an objective and constraints.
///
/// # Errors
/// Unknown ship, metric or group; unreadable build or context.
pub fn optimize_build(a: &OptimizeArgs) -> Result<Value, ServiceError> {
    let ctx = build_context(a.search.context.as_ref())?;
    let (base, _) = load_base(a.ship.as_deref(), a.build.as_deref())?;
    let mut req = request(base.clone(), &a.search, Effort::Normal)?;
    req.lock = SlotRef::locked_by(&base, &a.lock);
    req.keep_base = a.keep_base.unwrap_or(a.build.is_some());
    req.time_limit = Duration::from_secs_f64(a.time_limit_seconds.unwrap_or(60.0).max(0.1));
    let t0 = Instant::now();
    let out = search::optimize(&req, &ctx);
    Ok(outcome_json(&req, &out, &ctx, t0.elapsed().as_millis()))
}

/// `compare_ships` arguments.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareArgs {
    pub ships: Option<Vec<String>>,
    pub sizes: Option<Vec<u8>>,
    pub max_hull_cost: Option<f64>,
    pub top: Option<usize>,
    pub time_limit_seconds: Option<f64>,
    #[serde(flatten)]
    pub search: SearchOptions,
}

/// The same optimisation over many hulls (in parallel), ranked.
///
/// # Errors
/// Unknown ship, metric or group; unreadable context.
pub fn compare_ships(a: &CompareArgs) -> Result<Value, ServiceError> {
    let t0 = Instant::now();
    let ctx = build_context(a.search.context.as_ref())?;
    let mut ids: Vec<&'static str> = match &a.ships {
        Some(names) => names
            .iter()
            .map(|n| resolve_ship_id(n))
            .collect::<Result<_, _>>()?,
        None => ship_ids(),
    };
    let data = &*corai_data::DATA;
    if let Some(sizes) = &a.sizes {
        ids.retain(|id| sizes.contains(&data.ships[*id].properties.class));
    }
    if let Some(max) = a.max_hull_cost {
        ids.retain(|id| data.ships[*id].retail_cost <= max);
    }
    let budget = Duration::from_secs_f64(a.time_limit_seconds.unwrap_or(240.0).max(1.0));
    let reqs: Vec<Request> = ids
        .iter()
        .map(|id| {
            let mut r = request(stock_ship(id)?, &a.search, Effort::Quick)?;
            r.time_limit = budget.max(Duration::from_secs(2));
            Ok(r)
        })
        .collect::<Result<_, ServiceError>>()?;
    let outs = search::optimize_all(&reqs, &ctx);
    let mut rows: Vec<(search::Score, Value)> = reqs
        .iter()
        .zip(&outs)
        .map(|(req, out)| {
            let id = out.ship.id;
            let (c, e) = links(&out.ship);
            let mut row = Map::new();
            row.insert("ship".into(), id.into());
            row.insert("name".into(), data.ships[id].properties.name.clone().into());
            row.insert("feasible".into(), out.feasible.into());
            row.insert(
                "key".into(),
                serde_json::to_value(key_figures(req, &out.ship, &ctx)).unwrap_or_default(),
            );
            row.insert(
                "totalCost".into(),
                json_num(corai_engine::jsmath::js_round(out.ship.stats.total_cost)),
            );
            row.insert("coriolisUrl".into(), c.into());
            row.insert("edsyUrl".into(), e.into());
            if !out.feasible {
                row.insert("violations".into(), out.violations.clone().into());
            }
            if out.stats.timed_out {
                row.insert("timedOut".into(), true.into());
            }
            (out.score.clone(), Value::Object(row))
        })
        .collect();
    rows.sort_by(|x, y| y.0.compare(&x.0));
    let infeasible: Vec<Value> = rows
        .iter()
        .filter(|(_, r)| r["feasible"] != true)
        .map(|(_, r)| r["ship"].clone())
        .collect();
    let ranked: Vec<Value> = rows
        .into_iter()
        .take(a.top.unwrap_or(10))
        .enumerate()
        .map(|(i, (_, r))| {
            let mut row = Map::new();
            row.insert("rank".into(), (i + 1).into());
            if let Value::Object(fields) = r {
                row.extend(fields);
            }
            Value::Object(row)
        })
        .collect();
    Ok(
        json!({ "ranked": ranked, "infeasible": infeasible, "seconds": t0.elapsed().as_secs_f64().round() }),
    )
}
