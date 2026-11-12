//! Constraint-aware local search over outfittings, scored with the engine.
//!
//! Per start point, repeat passes over the unlocked slots (random order) until none improves:
//!   A. screen every legal, relevant module (with the best engineering seen so far for its group)
//!   B. try every blueprint × experimental on the top-K screened modules
//!   C. repair moves: modules that win on objective but break power / thrust / boost also try
//!      every power plant / thruster / distributor in the same move
//! A unique module (shield, scoop, ...) fitted elsewhere is offered as a relocate move: it takes
//! the slot and the displaced module moves into the slot it came from.
//! Start points: the base build, a max-A-rated core, a lightest core, then perturbations of the
//! incumbent. Budgets are deterministic; the time limit is only a safety net.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use corai_data::ModuleDef;
use corai_engine::jsmath::num;
use corai_engine::{Module, Ship, codec};
use rayon::prelude::*;

use crate::arrange::{assign_priorities, combat_deployed_draw, tidy_internal_slots};
use crate::catalog::{
    EngSpec, Filters, MAX_EXPERIMENTAL_WEAPONS, SlotRef, candidates, engineering_options, fits,
    unique_family,
};
use crate::metrics::{Better, Metric, MetricContext};
use crate::validate::validate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Max,
    Min,
}

#[derive(Debug, Clone, Copy)]
pub struct Goal {
    pub metric: Metric,
    pub direction: Direction,
}

impl Goal {
    /// The metric in its natural direction.
    pub fn natural(metric: Metric) -> Self {
        let direction = if metric.better() == Better::Higher {
            Direction::Max
        } else {
            Direction::Min
        };
        Self { metric, direction }
    }

    fn sign(&self) -> f64 {
        if self.direction == Direction::Max {
            1.0
        } else {
            -1.0
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Constraint {
    pub metric: Metric,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Which power budget must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCheck {
    /// Combat modules powered with hardpoints deployed (non-combat modules are shed by priority, as
    /// in game), and everything powered with hardpoints retracted.
    Priority,
    /// Everything on with hardpoints deployed.
    Deployed,
    /// Hardpoints retracted (explorers, traders).
    Retracted,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Quick,
    Normal,
    Thorough,
}

#[derive(Debug, Clone, Copy)]
struct Budget {
    top_k: usize,
    max_passes: usize,
    perturbations: usize,
    repairs: usize,
}

impl Effort {
    fn budget(self) -> Budget {
        match self {
            Effort::Quick => Budget {
                top_k: 2,
                max_passes: 4,
                perturbations: 2,
                repairs: 2,
            },
            Effort::Normal => Budget {
                top_k: 4,
                max_passes: 8,
                perturbations: 8,
                repairs: 3,
            },
            Effort::Thorough => Budget {
                top_k: 8,
                max_passes: 12,
                perturbations: 30,
                repairs: 6,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    /// Starting build (stock or decoded from a Coriolis URL).
    pub base: Ship,
    pub objective: Goal,
    /// Secondary goals, used only to break exact ties. Lower cost is always the last tiebreaker.
    pub tiebreakers: Vec<Goal>,
    /// On equal scores keep the base build's modules instead of the cheaper fit (improving a
    /// player's build: without it ties silently downgrade modules the objective ignores).
    pub keep_base: bool,
    pub constraints: Vec<Constraint>,
    /// Slots kept exactly as in `base`.
    pub lock: Vec<SlotRef>,
    /// Try grade-5 max-roll blueprints and experimentals.
    pub engineering: bool,
    pub power: PowerCheck,
    pub require_boost: bool,
    /// Module groups to consider even if no metric asks for them.
    pub extra_groups: Vec<&'static str>,
    pub filters: Filters,
    pub effort: Effort,
    pub seed: u64,
    pub time_limit: Duration,
}

impl Request {
    pub fn new(base: Ship, objective: Goal) -> Self {
        Self {
            base,
            objective,
            tiebreakers: Vec::new(),
            keep_base: false,
            constraints: Vec::new(),
            lock: Vec::new(),
            engineering: true,
            power: PowerCheck::Priority,
            require_boost: true,
            extra_groups: Vec::new(),
            filters: Filters::default(),
            effort: Effort::Normal,
            seed: 1,
            time_limit: Duration::from_secs(60),
        }
    }
}

/// Lexicographic score: lower violation first, then goal values (sign-adjusted, higher is better).
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub violation: f64,
    pub values: Vec<f64>,
}

const EPS: f64 = 1e-9;

impl Score {
    /// `Greater` means `self` is better.
    pub fn compare(&self, other: &Score) -> Ordering {
        if (self.violation - other.violation).abs() > EPS {
            return other
                .violation
                .partial_cmp(&self.violation)
                .unwrap_or(Ordering::Equal);
        }
        for (a, b) in self.values.iter().zip(&other.values) {
            let tol = EPS * 1f64.max(a.abs()).max(b.abs());
            if (a - b).abs() > tol {
                return a.partial_cmp(b).unwrap_or(Ordering::Equal);
            }
        }
        Ordering::Equal
    }

    fn beats(&self, other: &Score) -> bool {
        self.compare(other) == Ordering::Greater
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchStats {
    pub evaluations: u64,
    pub elapsed: Duration,
    pub starts: usize,
    pub timed_out: bool,
    pub searched_groups: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub ship: Ship,
    pub score: Score,
    pub feasible: bool,
    /// Human-readable reasons the best build misses hard requirements or constraints.
    pub violations: Vec<String>,
    /// Other distinct local optima, best first.
    pub alternatives: Vec<(Ship, Score)>,
    pub stats: SearchStats,
}

/// What can go into a slot.
#[derive(Debug, Clone, Copy)]
enum Pick {
    Empty,
    Module(&'static ModuleDef),
    Bulkhead(usize),
}

impl PartialEq for Pick {
    /// Module definitions are unique static entries, so identity is equality.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Pick::Empty, Pick::Empty) => true,
            (Pick::Module(a), Pick::Module(b)) => std::ptr::eq(*a, *b),
            (Pick::Bulkhead(a), Pick::Bulkhead(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Cand {
    pick: Pick,
    /// Slot currently holding the same unique family; it receives the displaced module.
    displace: Option<SlotRef>,
}

/// A core-module swap that accompanies a repair move.
type CoreMove = (SlotRef, Pick, Option<EngSpec>);
/// A complete move for one slot: candidate, its engineering, and an optional core swap.
type Move = (Cand, Option<EngSpec>, Option<CoreMove>);

/// Exact contents of a slot, for undoing trial moves.
#[derive(Debug, Clone)]
enum Saved {
    Slot(Option<Module>, bool),
    Bulkhead(usize, Module),
}

#[derive(Debug, Default)]
struct Hard {
    power: f64,
    thrust: f64,
    boost: f64,
}

impl Hard {
    fn total(&self) -> f64 {
        self.power + self.thrust + self.boost
    }
}

struct Trial {
    score: Score,
    hard: f64,
}

/// Small deterministic PRNG (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

/// Module identity including engineering, so a re-engineered module counts as changed.
type Signature = (usize, Option<&'static str>, u8, Option<&'static str>);

fn signatures(ship: &Ship) -> Vec<Signature> {
    std::iter::once(&ship.bulkhead)
        .chain(ship.all_slots().filter_map(|s| s.module.as_ref()))
        .map(|m| {
            let (bp, special) = m.engineering();
            (
                std::ptr::from_ref(m.def) as usize,
                bp.map(|b| b.0),
                bp.map_or(0, |b| b.1),
                special,
            )
        })
        .collect()
}

/// Number of the base build's modules still fitted (moving a module between slots keeps it).
fn kept_modules(base: &[Signature], ship: &Ship) -> f64 {
    let mut left: HashMap<Signature, usize> = HashMap::new();
    for s in base {
        *left.entry(*s).or_default() += 1;
    }
    let mut kept = 0;
    for s in signatures(ship) {
        if let Some(n) = left.get_mut(&s).filter(|n| **n > 0) {
            *n -= 1;
            kept += 1;
        }
    }
    f64::from(kept)
}

struct Searcher<'a> {
    req: &'a Request,
    ctx: &'a MetricContext,
    /// Objective and tiebreakers (without the cost tiebreak).
    goals: Vec<Goal>,
    /// The base build's module signatures when ties should keep them.
    keep: Option<Vec<Signature>>,
    /// Cheaper fit as the final tiebreak, unless a goal is already about cost.
    cost: Option<Goal>,
    budget: Budget,
    groups: HashSet<&'static str>,
    locked: HashSet<SlotRef>,
    rng: Rng,
    /// Engineered module variants, built once and cloned (their stat caches stay warm).
    variants: HashMap<(usize, Option<EngSpec>), Module>,
    /// Base candidate lists per slot (before uniqueness / experimental limits).
    pools: HashMap<SlotRef, Vec<&'static ModuleDef>>,
    /// Best engineering seen so far per module group, used when screening.
    eng_guess: HashMap<&'static str, EngSpec>,
    evaluations: u64,
    deadline: Instant,
    timed_out: bool,
}

impl<'a> Searcher<'a> {
    fn new(req: &'a Request, ctx: &'a MetricContext) -> Self {
        let mut goals = vec![req.objective];
        goals.extend(req.tiebreakers.iter().copied());
        let cost_metric: Metric = "totalCost".parse().expect("totalCost is a built-in metric");
        let cost = (!goals.iter().any(|g| g.metric.name() == "totalCost")).then_some(Goal {
            metric: cost_metric,
            direction: Direction::Min,
        });
        // Only groups that can push a requested metric the wanted way are searched: a module that
        // contributes nothing can only add mass, power draw and cost.
        let mut groups: HashSet<&'static str> = req.extra_groups.iter().copied().collect();
        for g in goals.iter().chain(&cost) {
            groups.extend(g.metric.groups());
        }
        for c in &req.constraints {
            if c.min.is_some() || (c.metric.better() == Better::Lower && c.max.is_some()) {
                groups.extend(c.metric.groups());
            }
        }
        Self {
            req,
            ctx,
            keep: req.keep_base.then(|| signatures(&req.base)),
            cost,
            goals,
            budget: req.effort.budget(),
            groups,
            locked: req.lock.iter().copied().collect(),
            rng: Rng(req.seed),
            variants: HashMap::new(),
            pools: HashMap::new(),
            eng_guess: HashMap::new(),
            evaluations: 0,
            deadline: Instant::now() + req.time_limit,
            timed_out: false,
        }
    }

    fn time_up(&mut self) -> bool {
        if !self.timed_out && Instant::now() > self.deadline {
            self.timed_out = true;
        }
        self.timed_out
    }

    // ------------------------------------------------------------ scoring

    fn hard(&self, ship: &Ship) -> (Hard, Vec<String>) {
        let s = &ship.stats;
        let mut h = Hard::default();
        let mut why = Vec::new();
        let budget = s.power_available.max(1.0);
        let mut over = |draw: f64, what: &str| {
            if draw <= s.power_available + EPS {
                return;
            }
            h.power += (draw - s.power_available) / budget;
            why.push(format!(
                "{what} {draw:.2} MW exceeds power plant {:.2} MW",
                s.power_available
            ));
        };
        match self.req.power {
            PowerCheck::Priority => {
                // The game sheds the lowest priority groups first: non-combat modules may go dark.
                over(
                    combat_deployed_draw(ship),
                    "combat draw with hardpoints deployed",
                );
                over(s.power_retracted, "draw with hardpoints retracted");
            }
            PowerCheck::Deployed => over(s.power_deployed, "draw with hardpoints deployed"),
            PowerCheck::Retracted => over(s.power_retracted, "draw with hardpoints retracted"),
            PowerCheck::Ignore => {}
        }
        let max_mass = num(ship.thrusters().and_then(|t| t.r().max_mass));
        if s.laden_mass >= max_mass {
            h.thrust = (s.laden_mass - max_mass) / max_mass.max(1.0) + 0.01;
            why.push(format!(
                "laden mass {:.1} t exceeds thruster max mass {max_mass:.1} t",
                s.laden_mass
            ));
        }
        if self.req.require_boost {
            let engcap = num(ship.power_distributor().and_then(|p| p.r().engcap));
            let need = ship.def.properties.boost_energy;
            if engcap < need {
                h.boost = (need - engcap) / need;
                why.push(format!(
                    "distributor ENG capacity {engcap:.1} MJ cannot boost (needs {need} MJ)"
                ));
            }
        }
        (h, why)
    }

    fn soft(&self, ship: &Ship) -> (f64, Vec<String>) {
        let mut v = 0.0;
        let mut why = Vec::new();
        for c in &self.req.constraints {
            let x = c.metric.compute(ship, self.ctx);
            // An undefined value (NaN) must never satisfy a bound.
            if !x.is_finite() {
                v += 1.0;
                why.push(format!("{} is undefined for this build", c.metric.name()));
                continue;
            }
            if let Some(min) = c.min.filter(|m| x < m - EPS) {
                v += (min - x) / min.abs().max(1.0);
                why.push(format!("{} = {x:.2} < required min {min}", c.metric.name()));
            }
            if let Some(max) = c.max.filter(|m| x > m + EPS) {
                v += (x - max) / max.abs().max(1.0);
                why.push(format!("{} = {x:.2} > allowed max {max}", c.metric.name()));
            }
        }
        (v, why)
    }

    fn evaluate(&mut self, ship: &Ship) -> Trial {
        self.evaluations += 1;
        let (hard, _) = self.hard(ship);
        let (soft, _) = self.soft(ship);
        let finite = |v: f64| if v.is_finite() { v } else { -1e12 };
        let mut values: Vec<f64> = self
            .goals
            .iter()
            .map(|g| finite(g.sign() * g.metric.compute(ship, self.ctx)))
            .collect();
        if let Some(base) = &self.keep {
            values.push(kept_modules(base, ship));
        }
        if let Some(cost) = &self.cost {
            values.push(finite(cost.sign() * cost.metric.compute(ship, self.ctx)));
        }
        Trial {
            score: Score {
                violation: hard.total() + soft,
                values,
            },
            hard: hard.total(),
        }
    }

    // ------------------------------------------------------------ slots and moves

    fn searchable(&self, ship: &Ship) -> Vec<SlotRef> {
        SlotRef::all(ship)
            .into_iter()
            .filter(|r| !self.locked.contains(r))
            .collect()
    }

    fn pool(&mut self, ship: &Ship, r: SlotRef) -> Vec<&'static ModuleDef> {
        if let Some(p) = self.pools.get(&r) {
            return p.clone();
        }
        let p = match r {
            SlotRef::Bulkheads => Vec::new(),
            SlotRef::Standard(i) | SlotRef::Hardpoint(i) | SlotRef::Internal(i) => {
                let slot = r.slot(ship).expect("slot refs come from this ship");
                candidates(ship, slot, i, &self.req.filters, Some(&self.groups))
            }
        };
        self.pools.insert(r, p.clone());
        p
    }

    /// Legal candidates for `r` given what the rest of the ship carries.
    fn candidates(&mut self, ship: &Ship, r: SlotRef) -> Vec<Cand> {
        if r == SlotRef::Bulkheads {
            return (0..ship.def.bulkheads.len())
                .map(|i| Cand {
                    pick: Pick::Bulkhead(i),
                    displace: None,
                })
                .collect();
        }
        let others: Vec<(SlotRef, &Module)> = SlotRef::all(ship)
            .into_iter()
            .filter(|o| *o != r && !matches!(o, SlotRef::Standard(_) | SlotRef::Bulkheads))
            .filter_map(|o| o.slot(ship).and_then(|s| s.module.as_ref()).map(|m| (o, m)))
            .collect();
        let experimental = others.iter().filter(|(_, m)| m.def.experimental).count();
        let mut out: Vec<Cand> = Vec::new();
        for def in self.pool(ship, r) {
            if def.experimental && experimental >= MAX_EXPERIMENTAL_WEAPONS {
                continue;
            }
            match unique_family(&def.grp)
                .and_then(|fam| others.iter().find(|(_, m)| fam.contains(&m.grp())))
            {
                None => out.push(Cand {
                    pick: Pick::Module(def),
                    displace: None,
                }),
                Some((holder, _)) => {
                    if matches!((r, holder), (SlotRef::Internal(_), SlotRef::Internal(_)))
                        && !self.locked.contains(holder)
                    {
                        out.push(Cand {
                            pick: Pick::Module(def),
                            displace: Some(*holder),
                        });
                    }
                }
            }
        }
        if !matches!(r, SlotRef::Standard(_)) {
            out.push(Cand {
                pick: Pick::Empty,
                displace: None,
            });
        }
        out
    }

    fn def_of(ship: &Ship, pick: Pick) -> Option<&'static ModuleDef> {
        match pick {
            Pick::Empty => None,
            Pick::Module(d) => Some(d),
            Pick::Bulkhead(i) => ship.def.bulkheads.get(i),
        }
    }

    fn guess(&self, ship: &Ship, pick: Pick) -> Option<EngSpec> {
        if !self.req.engineering {
            return None;
        }
        let def = Self::def_of(ship, pick)?;
        let g = self.eng_guess.get(def.grp.as_str())?;
        engineering_options(def).contains(g).then_some(*g)
    }

    /// The module for a pick with engineering applied, from the variant cache.
    fn variant(&mut self, def: &'static ModuleDef, eng: Option<EngSpec>) -> Module {
        let key = (std::ptr::from_ref(def) as usize, eng);
        self.variants
            .entry(key)
            .or_insert_with(|| {
                let mut m = Module::new(def);
                m.init_pre_engineered();
                if let Some(e) = eng {
                    if def.pre_engineered.is_some() {
                        if m.blueprint().is_some() {
                            let _ = m.set_special(e.special);
                        }
                    } else if m.apply_blueprint(e.blueprint, e.grade, 1.0).is_ok() {
                        let _ = m.set_special(e.special);
                    }
                }
                let _ = m.r();
                m
            })
            .clone()
    }

    fn save(ship: &Ship, r: SlotRef) -> Saved {
        match r.slot(ship) {
            Some(s) => Saved::Slot(s.module.clone(), s.enabled),
            None => Saved::Bulkhead(ship.bulkhead_index, ship.bulkhead.clone()),
        }
    }

    fn restore(ship: &mut Ship, r: SlotRef, saved: Saved) {
        match (r.slot_mut(ship), saved) {
            (Some(slot), Saved::Slot(m, enabled)) => {
                slot.module = m;
                slot.enabled = enabled;
            }
            (None, Saved::Bulkhead(i, m)) => {
                ship.bulkhead_index = i;
                ship.bulkhead = m;
            }
            _ => {}
        }
    }

    /// Puts candidate `c` into `r` (without recalculating). `prev` is what `r` held before,
    /// which a relocate move carries over to the vacated slot.
    fn put(
        &mut self,
        ship: &mut Ship,
        r: SlotRef,
        c: Cand,
        eng: Option<EngSpec>,
        prev: Option<&Module>,
    ) {
        match c.pick {
            Pick::Bulkhead(i) => {
                let def = &ship.def.bulkheads[i];
                ship.bulkhead = self.variant(def, eng);
                ship.bulkhead_index = i;
            }
            Pick::Empty | Pick::Module(_) => {
                let m = Self::def_of(ship, c.pick).map(|d| self.variant(d, eng));
                if let Some(slot) = r.slot_mut(ship) {
                    slot.module = m;
                    slot.enabled = true;
                }
            }
        }
        if let Some(d) = c.displace {
            let moved = prev
                .filter(|m| d.slot(ship).is_some_and(|s| fits(ship, s, m.def)))
                .cloned();
            if let Some(slot) = d.slot_mut(ship) {
                slot.module = moved;
                slot.enabled = true;
            }
        }
    }

    /// Scores `c` in `r`, then puts back any relocated slot (`r` itself stays overwritten).
    fn trial(
        &mut self,
        ship: &mut Ship,
        r: SlotRef,
        c: Cand,
        eng: Option<EngSpec>,
        prev: Option<&Module>,
    ) -> Trial {
        let disp = c.displace.map(|d| (d, Self::save(ship, d)));
        self.put(ship, r, c, eng, prev);
        ship.recalc();
        let t = self.evaluate(ship);
        if let Some((d, s)) = disp {
            Self::restore(ship, d, s);
        }
        t
    }

    /// Finds the best change for one slot; applies it and returns true if strictly better.
    fn improve_slot(&mut self, ship: &mut Ship, r: SlotRef) -> bool {
        let saved = Self::save(ship, r);
        let prev = match &saved {
            Saved::Slot(m, _) => m.clone(),
            Saved::Bulkhead(..) => None,
        };
        let mut best = self.evaluate(ship).score;
        let mut best_move: Option<Move> = None;

        // A. screening
        let mut screened: Vec<(Cand, Option<EngSpec>, Trial)> = Vec::new();
        for c in self.candidates(ship, r) {
            if self.time_up() {
                break;
            }
            let eng = self.guess(ship, c.pick);
            let t = self.trial(ship, r, c, eng, prev.as_ref());
            screened.push((c, eng, t));
        }
        screened.sort_by(|a, b| b.2.score.compare(&a.2.score));
        if let Some((c, eng, t)) = screened.first()
            && t.score.beats(&best)
        {
            best = t.score.clone();
            best_move = Some((*c, *eng, None));
        }

        // B. full engineering on the top-K screened modules
        if self.req.engineering {
            let top: Vec<Cand> = screened
                .iter()
                .filter(|(c, _, _)| c.pick != Pick::Empty)
                .take(self.budget.top_k)
                .map(|(c, _, _)| *c)
                .collect();
            for c in top {
                let Some(def) = Self::def_of(ship, c.pick) else {
                    continue;
                };
                for eng in engineering_options(def) {
                    if self.time_up() {
                        break;
                    }
                    let t = self.trial(ship, r, c, Some(eng), prev.as_ref());
                    if t.score.beats(&best) {
                        best = t.score;
                        best_move = Some((c, Some(eng), None));
                    }
                }
            }
        }
        Self::restore(ship, r, saved.clone());
        ship.recalc();

        // C. repair moves for promising candidates that break power, thrust or boost
        if !matches!(r, SlotRef::Standard(_)) {
            let mut promising: Vec<&(Cand, Option<EngSpec>, Trial)> = screened
                .iter()
                .filter(|(c, _, t)| c.pick != Pick::Empty && t.hard > EPS)
                .collect();
            let objective_only = |t: &Trial| Score {
                violation: t.score.violation - t.hard,
                values: t.score.values.clone(),
            };
            promising.sort_by(|a, b| objective_only(&b.2).compare(&objective_only(&a.2)));
            let promising: Vec<(Cand, Option<EngSpec>)> = promising
                .into_iter()
                .take(self.budget.repairs)
                .map(|(c, e, _)| (*c, *e))
                .collect();
            for (c, eng) in promising {
                if self.time_up() {
                    break;
                }
                if let Some((score, core)) = self.repair(ship, r, c, eng, prev.as_ref(), &best) {
                    best = score;
                    best_move = Some((c, eng, Some(core)));
                }
            }
        }

        let Some((c, eng, core)) = best_move else {
            return false;
        };
        self.put(ship, r, c, eng, prev.as_ref());
        if let Some(e) = eng
            && let Some(def) = Self::def_of(ship, c.pick)
        {
            self.eng_guess.insert(def.grp.as_str(), e);
        }
        if let Some((core_ref, pick, core_eng)) = core {
            self.put(
                ship,
                core_ref,
                Cand {
                    pick,
                    displace: None,
                },
                core_eng,
                None,
            );
        }
        ship.recalc();
        true
    }

    /// Fits `c` and searches the core slot that resolves its hard violation.
    fn repair(
        &mut self,
        ship: &mut Ship,
        r: SlotRef,
        c: Cand,
        eng: Option<EngSpec>,
        prev: Option<&Module>,
        to_beat: &Score,
    ) -> Option<(Score, CoreMove)> {
        let saved_r = Self::save(ship, r);
        let saved_d = c.displace.map(|d| (d, Self::save(ship, d)));
        self.put(ship, r, c, eng, prev);
        ship.recalc();
        let (hard, _) = self.hard(ship);
        let cores: Vec<usize> = [
            (hard.power > 0.0, 0),
            (hard.thrust > 0.0, 1),
            (hard.boost > 0.0, 4),
        ]
        .into_iter()
        .filter(|(broken, _)| *broken)
        .map(|(_, i)| i)
        .collect();
        let mut result: Option<(Score, CoreMove)> = None;
        for i in cores {
            let core = SlotRef::Standard(i);
            if self.locked.contains(&core) {
                continue;
            }
            let saved_core = Self::save(ship, core);
            for cc in self.candidates(ship, core) {
                let ce = self.guess(ship, cc.pick);
                let t = self.trial(ship, core, cc, ce, None);
                if t.score.beats(result.as_ref().map_or(to_beat, |(s, _)| s)) {
                    result = Some((t.score, (core, cc.pick, ce)));
                }
            }
            Self::restore(ship, core, saved_core);
        }
        if let Some((d, s)) = saved_d {
            Self::restore(ship, d, s);
        }
        Self::restore(ship, r, saved_r);
        ship.recalc();
        result
    }

    fn descend(&mut self, ship: &mut Ship) -> Score {
        for _ in 0..self.budget.max_passes {
            if self.time_up() {
                break;
            }
            let mut slots = self.searchable(ship);
            for i in (1..slots.len()).rev() {
                let j = self.rng.below(i + 1);
                slots.swap(i, j);
            }
            let mut improved = false;
            for r in slots {
                if self.time_up() {
                    break;
                }
                improved |= self.improve_slot(ship, r);
            }
            if !improved {
                break;
            }
        }
        self.evaluate(ship).score
    }

    // ------------------------------------------------------------ start points

    /// Core modules at maximum class / best rating (`max`) or lightest (`!max`); other unlocked
    /// optional slots emptied.
    fn core_seed(&mut self, max: bool) -> Ship {
        let mut ship = self.req.base.clone();
        for r in self.searchable(&ship) {
            match r {
                SlotRef::Standard(i) => {
                    if max && ![0, 1, 2, 4].contains(&i) {
                        continue;
                    }
                    let opts: Vec<&'static ModuleDef> = self
                        .pool(&ship, r)
                        .into_iter()
                        .filter(|d| d.pre_engineered.is_none())
                        .collect();
                    let pick = if max {
                        let top = opts.iter().map(|d| d.class).max().unwrap_or(0);
                        opts.iter()
                            .filter(|d| d.class == top)
                            .min_by(|a, b| a.rating.cmp(&b.rating))
                            .copied()
                    } else {
                        opts.iter()
                            .min_by(|a, b| {
                                num(a.stat("mass"))
                                    .total_cmp(&num(b.stat("mass")))
                                    .then(b.class.cmp(&a.class))
                            })
                            .copied()
                    };
                    if let Some(d) = pick {
                        self.put(
                            &mut ship,
                            r,
                            Cand {
                                pick: Pick::Module(d),
                                displace: None,
                            },
                            None,
                            None,
                        );
                    }
                }
                SlotRef::Bulkheads => {}
                _ => {
                    if let Some(slot) = r.slot_mut(&mut ship) {
                        slot.module = None;
                    }
                }
            }
        }
        ship.recalc();
        ship
    }

    fn perturb(&mut self, from: &Ship) -> Ship {
        let mut ship = from.clone();
        for r in self.searchable(&ship) {
            if !self.rng.chance(0.3) {
                continue;
            }
            let cands = self.candidates(&ship, r);
            if cands.is_empty() {
                continue;
            }
            let c = cands[self.rng.below(cands.len())];
            let prev = r.slot(&ship).and_then(|s| s.module.clone());
            let eng = self.guess(&ship, c.pick);
            self.put(&mut ship, r, c, eng, prev.as_ref());
        }
        ship.recalc();
        ship
    }

    fn run(mut self) -> Outcome {
        let t0 = Instant::now();
        let mut locals: Vec<(Ship, Score)> = Vec::new();
        let starts: [fn(&mut Self) -> Ship; 3] = [
            |s| s.req.base.clone(),
            |s| s.core_seed(true),
            |s| s.core_seed(false),
        ];
        for start in starts {
            if self.time_up() {
                break;
            }
            let mut ship = start(&mut self);
            let score = self.descend(&mut ship);
            locals.push((ship, score));
        }
        for _ in 0..self.budget.perturbations {
            if self.time_up() {
                break;
            }
            let incumbent = locals
                .iter()
                .max_by(|a, b| a.1.compare(&b.1))
                .map(|(s, _)| s.clone())
                .expect("at least one start ran");
            let mut ship = self.perturb(&incumbent);
            let score = self.descend(&mut ship);
            locals.push((ship, score));
        }

        // Arrange like a player would: priority groups for power, small modules in small slots.
        // Neither may change the score; if one ever did, keep the unarranged build.
        let locked = self.locked.clone();
        let locals: Vec<(Ship, Score)> = locals
            .into_iter()
            .map(|(ship, score)| {
                let before = self.evaluate(&ship).score;
                let mut arranged = ship.clone();
                if self.req.power == PowerCheck::Priority {
                    assign_priorities(&mut arranged);
                }
                tidy_internal_slots(&mut arranged, |r| locked.contains(&r));
                if self.evaluate(&arranged).score.compare(&before) == Ordering::Equal {
                    (arranged, score)
                } else {
                    (ship, score)
                }
            })
            .collect();

        // Canonicalise through the Coriolis code so reported numbers match coriolis.io exactly.
        let mut ranked: Vec<(Ship, Score)> = locals
            .into_iter()
            .map(|(ship, _)| {
                let canonical = codec::decode(ship.id, &codec::encode(&ship)).unwrap_or(ship);
                let score = self.evaluate(&canonical).score;
                (canonical, score)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.compare(&a.1));
        let mut seen = HashSet::new();
        ranked.retain(|(s, _)| seen.insert(codec::encode(s)));
        let starts = seen.len();

        let (best, score) = ranked.remove(0);
        let (hard, mut violations) = self.hard(&best);
        let (soft, soft_why) = self.soft(&best);
        violations.extend(soft_why);
        // Defence in depth: anything Coriolis itself would not let you fit is reported, never hidden.
        let illegal = validate(&best);
        let legal = illegal.is_empty();
        violations.extend(illegal.into_iter().map(|p| format!("illegal: {p}")));
        let mut searched_groups: Vec<&'static str> = self.groups.iter().copied().collect();
        searched_groups.sort_unstable();
        Outcome {
            feasible: hard.total() + soft <= EPS && legal,
            violations,
            ship: best,
            score,
            alternatives: ranked.into_iter().take(2).collect(),
            stats: SearchStats {
                evaluations: self.evaluations,
                elapsed: t0.elapsed(),
                starts,
                timed_out: self.timed_out,
                searched_groups,
            },
        }
    }
}

/// Optimises one ship.
pub fn optimize(req: &Request, ctx: &MetricContext) -> Outcome {
    Searcher::new(req, ctx).run()
}

/// Optimises several requests (typically one per hull) in parallel; results keep input order.
pub fn optimize_all(reqs: &[Request], ctx: &MetricContext) -> Vec<Outcome> {
    reqs.par_iter().map(|r| optimize(r, ctx)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(name: &str) -> Goal {
        Goal::natural(name.parse().expect("metric"))
    }

    fn constraint(name: &str, min: Option<f64>, max: Option<f64>) -> Constraint {
        Constraint {
            metric: name.parse().expect("metric"),
            min,
            max,
        }
    }

    fn quick(ship: &str, objective: &str) -> Request {
        let mut r = Request::new(Ship::stock(ship).expect("ship"), goal(objective));
        r.effort = Effort::Quick;
        r
    }

    #[test]
    fn score_ordering() {
        let a = Score {
            violation: 0.0,
            values: vec![1.0],
        };
        let b = Score {
            violation: 0.1,
            values: vec![100.0],
        };
        let c = Score {
            violation: 0.0,
            values: vec![2.0],
        };
        assert!(a.beats(&b), "feasibility first");
        assert!(c.beats(&a));
        assert_eq!(a.compare(&a.clone()), Ordering::Equal);
        assert!(
            !Score {
                violation: f64::INFINITY,
                values: vec![]
            }
            .beats(&a)
        );
    }

    #[test]
    fn jump_build_beats_stock_and_respects_hard_limits() {
        let ctx = MetricContext::default();
        let req = quick("python", "unladenRange");
        let stock = goal("unladenRange").metric.compute(&req.base, &ctx);
        let out = optimize(&req, &ctx);
        assert!(out.feasible, "{:?}", out.violations);
        let best = goal("unladenRange").metric.compute(&out.ship, &ctx);
        assert!(best > stock * 2.0, "{best} vs stock {stock}");
        let s = &out.ship.stats;
        assert!(
            s.power_deployed <= s.power_available
                && out.ship.can_boost(s.cargo_capacity, s.fuel_capacity)
        );
    }

    #[test]
    fn required_modules_are_fitted() {
        let ctx = MetricContext::default();
        let mut req = quick("diamondback_explorer", "unladenRange");
        req.constraints = vec![
            constraint("count:fs", Some(1.0), None),
            constraint("count:am", Some(1.0), None),
        ];
        let out = optimize(&req, &ctx);
        assert!(out.feasible, "{:?}", out.violations);
        for c in &req.constraints {
            assert!(c.metric.compute(&out.ship, &ctx) >= 1.0);
        }
    }

    #[test]
    fn impossible_constraints_are_reported_not_hidden() {
        let ctx = MetricContext::default();
        let mut req = quick("sidewinder", "cargoCapacity");
        req.constraints = vec![constraint("cargoCapacity", Some(10_000.0), None)];
        let out = optimize(&req, &ctx);
        assert!(!out.feasible);
        assert!(out.violations.iter().any(|v| v.contains("cargoCapacity")));
    }

    #[test]
    fn locked_slots_are_untouched() {
        let ctx = MetricContext::default();
        let mut req = quick("python", "unladenRange");
        req.lock = SlotRef::all(&req.base)
            .into_iter()
            .filter(|r| matches!(r, SlotRef::Hardpoint(_)))
            .collect();
        let out = optimize(&req, &ctx);
        let ids = |s: &Ship| {
            s.hardpoints
                .iter()
                .map(|h| h.module.as_ref().map(|m| m.def.id.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&out.ship), ids(&req.base));
    }

    #[test]
    fn same_request_same_answer() {
        let ctx = MetricContext::default();
        let req = quick("cobra_mk_iii", "shieldEHP");
        assert_eq!(
            codec::encode(&optimize(&req, &ctx).ship),
            codec::encode(&optimize(&req, &ctx).ship)
        );
    }

    #[test]
    fn unique_modules_stay_unique() {
        let ctx = MetricContext::default();
        let out = optimize(&quick("anaconda", "shieldEHP"), &ctx);
        let shields = out
            .ship
            .internal
            .iter()
            .filter_map(|s| s.module.as_ref())
            .filter(|m| m.is_shield_generator())
            .count();
        assert_eq!(shields, 1);
    }
}
