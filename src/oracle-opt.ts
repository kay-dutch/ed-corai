// Optimiser benchmark: realistic requests run through the Node optimiser. The Rust search must
// match or beat each result (same objective and constraints) with a legal build.
//   node dist/oracle-opt.cjs > crates/corai-opt/tests/fixtures/optimize.json
import './stdio-guard.js';
import { coriolisCode, shipFromCoriolis, stockShip } from './engine.js';
import { getMetric, makeContext } from './metrics.js';
import { optimize, type Constraint, type Goal } from './optimizer.js';
import { validateBuild } from './validate.js';
import { makeRng, randomBuild } from './oracle-lib.js';

interface Scenario {
  name: string;
  ship: string;
  /** Start from this Coriolis code instead of the stock ship. */
  code?: string;
  objective: Goal;
  constraints?: Constraint[];
  lock?: string[];
  keepBase?: boolean;
  power?: 'priority' | 'deployed' | 'retracted' | 'ignore';
  context?: Record<string, unknown>;
}

const pve = { damagePreset: 'pve', incomingDps: 300 };
const improve = randomBuild('anaconda2', makeRng(5)).ship;

const SCENARIOS: Scenario[] = [
  { name: 'pve anaconda mk ii shields', ship: 'anaconda2', objective: { metric: 'shieldTimeUnderFire' }, context: pve,
    constraints: [{ metric: 'sustainedDps', min: 100 }, { metric: 'emptyHardpoints', max: 0 }, { metric: 'weaponTypes', max: 2 }] },
  { name: 'corvette total ehp', ship: 'federal_corvette', objective: { metric: 'totalEHP' }, context: pve,
    constraints: [{ metric: 'sustainedDps', min: 100 }] },
  { name: 'krait explorer', ship: 'krait_mkii', objective: { metric: 'unladenRange' },
    constraints: [{ metric: 'count:fs', min: 1 }, { metric: 'count:am', min: 1 }] },
  { name: 'dbx explorer', ship: 'diamondback_explorer', objective: { metric: 'unladenRange' },
    constraints: [{ metric: 'count:fs', min: 1 }, { metric: 'count:am', min: 1 }, { metric: 'count:pv', min: 1 }] },
  { name: 'type 9 hauler', ship: 'type_9_heavy', objective: { metric: 'cargoCapacity' },
    constraints: [{ metric: 'ladenRange', min: 15 }] },
  { name: 'python trader', ship: 'python', objective: { metric: 'cargoCapacity' },
    constraints: [{ metric: 'ladenRange', min: 20 }, { metric: 'count:sg', min: 1 }] },
  { name: 'fdl hull killer', ship: 'fer_de_lance', objective: { metric: 'dpsVsHull' },
    constraints: [{ metric: 'shieldEHP', min: 2000 }, { metric: 'weaponTypes', max: 1 }, { metric: 'mountTypes', max: 1 }] },
  { name: 'mamba shields killer', ship: 'mamba', objective: { metric: 'dpsVsShields' },
    constraints: [{ metric: 'weaponTypes', max: 1 }, { metric: 'shieldEHP', min: 1000 }] },
  { name: 'cobra retracted ehp', ship: 'cobra_mk_iii', objective: { metric: 'totalEHP' }, power: 'retracted' },
  { name: 'cutter boostable hauler', ship: 'imperial_cutter', objective: { metric: 'cargoCapacity' },
    constraints: [{ metric: 'topBoost', min: 300 }, { metric: 'count:sg', min: 1 }] },
  { name: 'improve build: cooler, same shields', ship: 'anaconda2', code: coriolisCode(improve), keepBase: true,
    lock: ['hardpoint', 'standard:fsd'], objective: { metric: 'combatHeat' }, context: pve,
    constraints: [{ metric: 'shieldEHP', min: 0 }, { metric: 'unladenRange', min: 0 }] },
  { name: 'chieftain ax', ship: 'alliance_chieftain', objective: { metric: 'axSustainedDps' },
    constraints: [{ metric: 'hullEHP', min: 1500 }] },
];

const out = SCENARIOS.map((s) => {
  const ctx = makeContext(s.context as any);
  const base = s.code ? shipFromCoriolis(s.code, s.ship) : stockShip(s.ship);
  const t0 = Date.now();
  const r = optimize({
    base, objective: s.objective, constraints: s.constraints, lock: s.lock, keepBase: s.keepBase,
    power: s.power, ctx, effort: 'normal', timeLimitMs: 600_000,
  });
  const ms = Date.now() - t0;
  const objective = getMetric(s.objective.metric).compute(r.ship, ctx);
  process.stderr.write(`${s.name}: ${objective.toFixed(3)} feasible=${r.feasible} ${ms} ms\n`);
  return {
    ...s, code: s.code, base: coriolisCode(base),
    node: { feasible: r.feasible, objective, violations: r.violations, legal: validateBuild(r.ship).length === 0, code: coriolisCode(r.ship), ms },
  };
});
process.stdout.write(JSON.stringify({ generator: 'corai src/oracle-opt.ts', scenarios: out }) + '\n');
