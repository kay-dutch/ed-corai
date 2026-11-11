// Constraint-aware local search over ship outfittings, scored by the Coriolis engine.
//
// Algorithm (per start point):
//   repeat passes until no slot improves:
//     for each unlocked slot (random order):
//       A. screen every legal relevant module (with the best engineering seen so far for its group)
//       B. try every blueprint × experimental on the top-K screened modules
//       C. "repair" moves: for promising modules that break power / thrust / boost, also swap the
//          power plant / thrusters / distributor in the same move
//       keep the best strictly-improving change
// Start points: the base (or stock) build, a max-A-rated core, a lightest core, then iterated
// perturbations of the incumbent (re-randomise ~30% of slots and descend again).
import {
  cloneShip, coriolisCode, listSlots, place, restore, snapshot, STANDARD_KEYS,
  type EngSpec, type Ship, type SlotRef, type SlotSnapshot,
} from './engine.js';
import { engineeringOptions, fitsSlot, groupCategory, MAX_EXPERIMENTAL_WEAPONS, slotCandidates, uniqueFamily, type CatalogFilters } from './catalog.js';
import { getMetric, type MetricContext, type MetricDef } from './metrics.js';
import { validateBuild } from './validate.js';
import { assignPriorities, combatDeployedDraw, tidyInternalSlots } from './arrange.js';

export interface Goal { metric: string; direction?: 'max' | 'min' }
export interface Constraint { metric: string; min?: number; max?: number }
export type Effort = 'quick' | 'normal' | 'thorough';

export interface OptimizeRequest {
  base: Ship;                       // starting build (stock or a parsed Coriolis build)
  objective: Goal;
  tiebreakers?: Goal[];
  /**
   * Keep the starting build's modules when scores tie (true when improving a player's build).
   * Without it ties go to the cheaper fit, which is right from a stock hull but silently
   * downgrades modules the objective does not care about.
   */
  keepBase?: boolean;
  constraints?: Constraint[];
  lock?: string[];                  // slot keys ("standard:fsd", "internal:3") or kinds ("hardpoint", "utility", ...)
  engineering?: 'none' | 'full';
  /** priority (default): combat modules powered with hardpoints out, everything powered retracted. */
  power?: 'priority' | 'deployed' | 'retracted' | 'ignore';
  requireBoost?: boolean;
  extraGroups?: string[];           // module groups to consider even if no metric asks for them
  filters?: CatalogFilters;
  ctx: MetricContext;
  effort?: Effort;
  seed?: number;
  timeLimitMs?: number;
}

export interface Score { violation: number; values: number[] }

/** A module to try in a slot; `displace` is where a relocated unique module currently sits. */
interface Cand { m: any | null; displace?: SlotRef }

export interface OptimizeResult {
  ship: Ship;
  score: Score;
  feasible: boolean;
  violations: string[];
  alternatives: { ship: Ship; score: Score }[];
  stats: { evaluations: number; ms: number; starts: number; timedOut: boolean; searchedGroups: string[] };
}

const EFFORT = {
  quick: { k: 2, maxPasses: 4, perturbations: 0, repairs: 2 },
  normal: { k: 4, maxPasses: 6, perturbations: 3, repairs: 3 },
  thorough: { k: 8, maxPasses: 10, perturbations: 10, repairs: 6 },
} as const;

const EPS = 1e-9;

function mulberry32(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Module identity including engineering, so a re-engineered module counts as changed. */
function moduleSignature(m: any): string {
  const bp = m.blueprint;
  return [m.id, bp?.fdname ?? '', bp?.grade ?? '', bp?.special?.edname ?? ''].join('|');
}

function signatures(ship: Ship): string[] {
  return listSlots(ship).map((r) => r.slot.m).filter(Boolean).map(moduleSignature);
}

/**
 * Number of the base build's modules still fitted (moving a module between slots keeps it).
 * Used as a tiebreak so equal-scoring changes do not replace the player's modules.
 */
function keptModulesMetric(base: Ship): MetricDef {
  const want = new Map<string, number>();
  for (const sig of signatures(base)) want.set(sig, (want.get(sig) ?? 0) + 1);
  return {
    name: 'keptModules', unit: 'modules', better: 'higher', groups: [],
    description: 'Modules of the starting build still fitted.',
    compute: (ship) => {
      const left = new Map(want);
      let kept = 0;
      for (const sig of signatures(ship)) {
        const n = left.get(sig) ?? 0;
        if (n > 0) { kept++; left.set(sig, n - 1); }
      }
      return kept;
    },
  };
}

export function compareScores(a: Score, b: Score): number {
  if (Math.abs(a.violation - b.violation) > EPS) return a.violation < b.violation ? 1 : -1;
  for (let i = 0; i < Math.max(a.values.length, b.values.length); i++) {
    const x = a.values[i] ?? 0, y = b.values[i] ?? 0;
    const tol = EPS * Math.max(1, Math.abs(x), Math.abs(y));
    if (Math.abs(x - y) > tol) return x > y ? 1 : -1;
  }
  return 0;
}

interface Resolved { def: MetricDef; sign: number }

export class Optimizer {
  private readonly req: OptimizeRequest;
  private readonly cfg: (typeof EFFORT)[Effort];
  private readonly goals: Resolved[];
  private readonly constraints: { def: MetricDef; min?: number; max?: number }[];
  private readonly groups: Set<string>;
  private readonly rng: () => number;
  private readonly engGuess = new Map<string, EngSpec | null>();
  private readonly deadline: number;
  evaluations = 0;
  timedOut = false;

  constructor(req: OptimizeRequest) {
    this.req = req;
    this.cfg = EFFORT[req.effort ?? 'normal'];
    this.rng = mulberry32(req.seed ?? 1);
    this.deadline = Date.now() + (req.timeLimitMs ?? 60_000);
    const resolve = (g: Goal): Resolved => {
      const def = getMetric(g.metric);
      const dir = g.direction ?? (def.better === 'higher' ? 'max' : 'min');
      return { def, sign: dir === 'max' ? 1 : -1 };
    };
    this.goals = [req.objective, ...(req.tiebreakers ?? [])].map(resolve);
    if (req.keepBase) this.goals.push({ def: keptModulesMetric(req.base), sign: 1 });
    // On exact ties prefer the cheaper fit (drops modules that contribute nothing).
    if (!this.goals.some((g) => g.def.name === 'totalCost')) this.goals.push(resolve({ metric: 'totalCost', direction: 'min' }));
    this.constraints = (req.constraints ?? []).map((c) => ({ def: getMetric(c.metric), min: c.min, max: c.max }));

    // Only module groups that can push a requested metric in the wanted direction are searched.
    const groups = new Set<string>(req.extraGroups ?? []);
    for (const g of this.goals) g.def.groups.forEach((x) => groups.add(x));
    for (const c of this.constraints) {
      const needsModules = c.min !== undefined || (c.def.better === 'lower' && c.max !== undefined);
      if (needsModules) c.def.groups.forEach((x) => groups.add(x));
    }
    for (const g of groups) if (!groupCategory(g)) throw new Error(`Unknown module group "${g}".`);
    this.groups = groups;
  }

  get searchedGroups(): string[] {
    return [...this.groups].sort();
  }

  // ---------- scoring ----------

  score(ship: Ship): Score {
    this.evaluations++;
    const value = (g: Resolved) => { const v = g.sign * g.def.compute(ship, this.req.ctx); return Number.isFinite(v) ? v : -1e12; };
    return { violation: this.violation(ship).total, values: this.goals.map(value) };
  }

  violation(ship: Ship): { total: number; hard: { power: number; thrust: number; boost: number }; messages: string[] } {
    const messages: string[] = [];
    const hard = { power: 0, thrust: 0, boost: 0 };
    const mode = this.req.power ?? 'priority';
    const budget = Math.max(ship.powerAvailable, 1);
    const over = (draw: number, what: string) => {
      if (draw <= ship.powerAvailable + EPS) return;
      hard.power += (draw - ship.powerAvailable) / budget;
      messages.push(`${what} ${draw.toFixed(2)} MW exceeds power plant ${ship.powerAvailable.toFixed(2)} MW`);
    };
    if (mode === 'priority') {
      // The game sheds the lowest priority groups first: non-combat modules may go dark in a fight.
      over(combatDeployedDraw(ship), 'combat draw with hardpoints deployed');
      over(ship.powerRetracted, 'draw with hardpoints retracted');
    } else if (mode === 'deployed') {
      over(ship.powerDeployed, 'draw with hardpoints deployed');
    } else if (mode === 'retracted') {
      over(ship.powerRetracted, 'draw with hardpoints retracted');
    }
    const t = ship.standard[1].m;
    const maxMass = t ? t.getMaxMass() : 0;
    if (ship.ladenMass >= maxMass) {
      hard.thrust = (ship.ladenMass - maxMass) / Math.max(maxMass, 1) + 0.01;
      messages.push(`laden mass ${ship.ladenMass.toFixed(1)} t exceeds thruster max mass ${maxMass.toFixed(1)} t`);
    }
    if (this.req.requireBoost !== false) {
      const engcap = ship.standard[4].m ? ship.standard[4].m.getEnginesCapacity() : 0;
      if (engcap < ship.boostEnergy) {
        hard.boost = (ship.boostEnergy - engcap) / ship.boostEnergy;
        messages.push(`distributor ENG capacity ${engcap.toFixed(1)} MJ cannot boost (needs ${ship.boostEnergy} MJ)`);
      }
    }
    let soft = 0;
    for (const c of this.constraints) {
      const x = c.def.compute(ship, this.req.ctx);
      // An undefined value (NaN) must never satisfy a bound.
      if (!Number.isFinite(x)) {
        soft += 1;
        messages.push(`${c.def.name} is undefined for this build`);
        continue;
      }
      if (c.min !== undefined && x < c.min - EPS) {
        soft += (c.min - x) / Math.max(Math.abs(c.min), 1);
        messages.push(`${c.def.name} = ${x.toFixed(2)} < required min ${c.min}`);
      }
      if (c.max !== undefined && x > c.max + EPS) {
        soft += (x - c.max) / Math.max(Math.abs(c.max), 1);
        messages.push(`${c.def.name} = ${x.toFixed(2)} > allowed max ${c.max}`);
      }
    }
    return { total: hard.power + hard.thrust + hard.boost + soft, hard, messages };
  }

  // ---------- slot handling ----------

  private isLocked(ref: SlotRef): boolean {
    const lock = this.req.lock ?? [];
    return lock.includes(ref.key) || lock.includes(ref.kind) || (ref.kind === 'standard' && lock.includes(ref.key.split(':')[1]));
  }

  private searchSlots(ship: Ship): SlotRef[] {
    return listSlots(ship).filter((r) => !this.isLocked(r));
  }

  /**
   * Legal candidates for a slot given what the rest of the ship carries. A unique module
   * (shield, scoop, ...) already fitted elsewhere yields a relocate candidate: it takes this
   * slot and the module it displaces moves into the slot the unique module came from.
   */
  private candidates(ship: Ship, ref: SlotRef): Cand[] {
    const list: (any | null)[] = slotCandidates(ship, ref, this.req.filters ?? {}, this.groups);
    if (ref.kind !== 'standard' && ref.kind !== 'bulkheads') list.push(null);
    const others = listSlots(ship).filter((r) => (r.kind === 'internal' || r.kind === 'hardpoint' || r.kind === 'utility') && r.slot !== ref.slot && r.slot.m);
    const expCount = others.filter((r) => r.slot.m.experimental).length;
    const out: Cand[] = [];
    for (const m of list) {
      if (!m) { out.push({ m: null }); continue; }
      if (m.experimental && expCount >= MAX_EXPERIMENTAL_WEAPONS) continue;
      const fam = uniqueFamily(m.grp);
      const holder = fam && others.find((r) => fam.includes(r.slot.m.grp));
      if (!holder) out.push({ m });
      else if (ref.kind === 'internal' && holder.kind === 'internal' && !this.isLocked(holder)) out.push({ m, displace: holder });
    }
    return out;
  }

  private guessFor(m: any): EngSpec | null {
    if (!m || (this.req.engineering ?? 'full') === 'none') return null;
    const g = this.engGuess.get(m.grp);
    if (!g) return null;
    return engineeringOptions(m).some((o) => o.blueprint === g.blueprint && o.special === g.special) ? g : null;
  }

  /** Fits candidate c into ref; `prev` is the module ref held before (moved on relocate). */
  private apply(ship: Ship, ref: SlotRef, c: Cand, eng: EngSpec | null, prev: any): void {
    place(ship, ref, c.m, eng);
    if (c.displace) place(ship, c.displace, prev && fitsSlot(ship, c.displace, prev) ? prev : null, null);
  }

  /**
   * Scores candidate c in ref, leaving ref overwritten but any relocated slot restored.
   * `hard` is the power/thrust/boost part of the violation (repairable by core swaps).
   */
  private trial(ship: Ship, ref: SlotRef, c: Cand, eng: EngSpec | null, prev: any): Score & { hard: number } {
    const dispSnap = c.displace && snapshot(c.displace);
    try {
      this.apply(ship, ref, c, eng, prev);
      const h = this.violation(ship).hard;
      return { ...this.score(ship), hard: h.power + h.thrust + h.boost };
    } catch {
      return { violation: Infinity, values: [], hard: 0 };
    } finally {
      if (c.displace && dispSnap) restore(ship, c.displace, dispSnap);
    }
  }

  /** Best module for one slot; applies it and returns true when strictly better than the current fit. */
  private improveSlot(ship: Ship, ref: SlotRef): boolean {
    const snap = snapshot(ref);
    let best: Score = this.score(ship);
    let bestMove: (() => void) | null = null;

    // A. screening
    const screened: { c: Cand; eng: EngSpec | null; score: Score; objScore: Score }[] = [];
    for (const c of this.candidates(ship, ref)) {
      if (this.timeUp()) break;
      const eng = this.guessFor(c.m);
      const s = this.trial(ship, ref, c, eng, snap.m);
      screened.push({ c, eng, score: s, objScore: { violation: s.violation - s.hard, values: s.values } });
    }
    screened.sort((a, b) => compareScores(b.score, a.score));
    const top = screened[0];
    if (top && compareScores(top.score, best) > 0) {
      best = top.score;
      bestMove = () => this.apply(ship, ref, top.c, top.eng, snap.m);
    }

    // B. full engineering on the top-K distinct modules
    if ((this.req.engineering ?? 'full') === 'full') {
      for (const t of screened.slice(0, this.cfg.k)) {
        if (!t.c.m || this.timeUp()) continue;
        for (const eng of engineeringOptions(t.c.m)) {
          const s = this.trial(ship, ref, t.c, eng, snap.m);
          if (compareScores(s, best) > 0) {
            best = s;
            const c = t.c;
            bestMove = () => { this.apply(ship, ref, c, eng, snap.m); this.engGuess.set(c.m.grp, eng); };
          }
        }
      }
    }

    // Each trial above simply overwrote the slot; put the original module back once.
    restore(ship, ref, snap);

    // C. repair moves: modules that would win on objective but break power/thrust/boost
    if (ref.kind !== 'standard') {
      const promising = screened.filter((t) => t.c.m && t.score.violation > t.objScore.violation + EPS)
        .sort((a, b) => compareScores(b.objScore, a.objScore)).slice(0, this.cfg.repairs);
      for (const t of promising) {
        if (this.timeUp()) break;
        const move = this.repair(ship, ref, snap, t.c, t.eng, best);
        if (move) { best = move.score; bestMove = move.apply; }
      }
    }

    if (bestMove) {
      bestMove();
      return true;
    }
    return false;
  }

  /** Fits candidate c in ref and searches the core slot that fixes the resulting hard violation. */
  private repair(ship: Ship, ref: SlotRef, snap: SlotSnapshot, c: Cand, eng: EngSpec | null, toBeat: Score) {
    const dispSnap = c.displace && snapshot(c.displace);
    this.apply(ship, ref, c, eng, snap.m);
    const hard = this.violation(ship).hard;
    const fixKeys = [hard.power > 0 && 'pp', hard.thrust > 0 && 't', hard.boost > 0 && 'pd'].filter(Boolean) as string[];
    let result: { score: Score; apply: () => void } | null = null;
    for (const key of fixKeys) {
      const core = this.searchSlots(ship).find((r) => r.key === `standard:${key}`);
      if (!core) continue;
      const coreSnap = snapshot(core);
      for (const cc of this.candidates(ship, core)) {
        const cEng = this.guessFor(cc.m);
        const s = this.trial(ship, core, cc, cEng, coreSnap.m);
        if (compareScores(s, result?.score ?? toBeat) > 0) {
          result = { score: s, apply: () => { this.apply(ship, ref, c, eng, snap.m); place(ship, core, cc.m, cEng); } };
        }
      }
      restore(ship, core, coreSnap);
    }
    if (c.displace && dispSnap) restore(ship, c.displace, dispSnap);
    restore(ship, ref, snap);
    return result;
  }

  private timeUp(): boolean {
    if (Date.now() > this.deadline) this.timedOut = true;
    return this.timedOut;
  }

  private descend(ship: Ship): Score {
    for (let pass = 0; pass < this.cfg.maxPasses && !this.timeUp(); pass++) {
      const slots = this.searchSlots(ship);
      for (let i = slots.length - 1; i > 0; i--) {
        const j = Math.floor(this.rng() * (i + 1));
        [slots[i], slots[j]] = [slots[j], slots[i]];
      }
      let improved = false;
      for (const ref of slots) {
        if (this.timeUp()) break;
        if (this.improveSlot(ship, ref)) improved = true;
      }
      if (!improved) break;
    }
    return this.score(ship);
  }

  // ---------- start points ----------

  private coreSeed(kind: 'max' | 'light'): Ship {
    const ship = cloneShip(this.req.base);
    for (const ref of this.searchSlots(ship)) {
      if (ref.kind === 'standard') {
        const grp = STANDARD_KEYS[ref.index];
        if (kind === 'max' && !['pp', 't', 'pd', 'fsd'].includes(grp)) continue;
        const opts = this.candidates(ship, ref).map((c) => c.m).filter((m) => m && !m.preEngineered);
        if (!opts.length) continue;
        const pick = kind === 'max'
          ? opts.filter((m) => m.class === Math.max(...opts.map((o) => o.class))).sort((a, b) => a.rating.localeCompare(b.rating))[0]
          : [...opts].sort((a, b) => a.mass - b.mass || b.class - a.class)[0];
        place(ship, ref, pick, null);
      } else if (ref.kind !== 'bulkheads' && ref.slot.m) {
        place(ship, ref, null, null);
      }
    }
    return ship;
  }

  private perturb(from: Ship): Ship {
    const ship = cloneShip(from);
    for (const ref of this.searchSlots(ship)) {
      if (this.rng() > 0.3) continue;
      const opts = this.candidates(ship, ref);
      if (!opts.length) continue;
      const c = opts[Math.floor(this.rng() * opts.length)];
      this.apply(ship, ref, c, this.guessFor(c.m), ref.slot.m);
    }
    return ship;
  }

  run(): OptimizeResult {
    const t0 = Date.now();
    const locals: { ship: Ship; score: Score }[] = [];
    const consider = (ship: Ship) => {
      const score = this.descend(ship);
      locals.push({ ship, score });
    };

    const starts: (() => Ship)[] = [() => cloneShip(this.req.base), () => this.coreSeed('max'), () => this.coreSeed('light')];
    for (const start of starts) {
      if (this.timeUp()) break;
      consider(start());
    }
    for (let i = 0; i < this.cfg.perturbations && !this.timeUp(); i++) {
      const incumbent = locals.reduce((a, b) => (compareScores(b.score, a.score) > 0 ? b : a));
      consider(this.perturb(incumbent.ship));
    }

    // Arrange like a player would: priority groups for power, small modules in small slots. Neither
    // may change the score; if one ever did, keep the unarranged build.
    for (const l of locals) {
      const before = this.score(l.ship);
      const backup = cloneShip(l.ship);
      if ((this.req.power ?? 'priority') === 'priority') assignPriorities(l.ship);
      tidyInternalSlots(l.ship, (r) => this.isLocked(r));
      if (compareScores(this.score(l.ship), before) !== 0) l.ship = backup;
    }

    // Canonicalise: rebuild from the Coriolis code so reported numbers match coriolis.io exactly.
    const ranked = locals
      .map((l) => { const ship = cloneShip(l.ship); return { ship, score: this.score(ship) }; })
      .sort((a, b) => compareScores(b.score, a.score));
    const seen = new Set<string>();
    const distinct = ranked.filter((l) => { const c = coriolisCode(l.ship); if (seen.has(c)) return false; seen.add(c); return true; });
    const best = distinct[0];
    const v = this.violation(best.ship);
    // Defence in depth: anything Coriolis itself would not let you fit is reported, never hidden.
    v.messages.push(...validateBuild(best.ship).map((p) => `illegal: ${p}`));
    return {
      ship: best.ship,
      score: best.score,
      feasible: v.total <= EPS && !v.messages.some((m) => m.startsWith('illegal:')),
      violations: v.messages,
      alternatives: distinct.slice(1, 3),
      stats: { evaluations: this.evaluations, ms: Date.now() - t0, starts: locals.length, timedOut: this.timedOut, searchedGroups: this.searchedGroups },
    };
  }
}

export function optimize(req: OptimizeRequest): OptimizeResult {
  return new Optimizer(req).run();
}

