// Differential-test oracle for what corai adds on top of Coriolis: metrics, legality checks, build
// summaries and build import/export. Builds are referenced by their Coriolis code, which the Rust
// engine already decodes exactly. `npm run oracle` regenerates the fixtures.
import './stdio-guard.js';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import {
  BlueprintFunctions, Modules, Serializer, Ships, coriolisCode, findSlot, listSlots, place, recalcAll,
  shipFromCoriolis, stockShip, type Ship,
} from './engine.js';
import { METRICS, getMetric, makeContext } from './metrics.js';
import { validateBuild } from './validate.js';
import { summarize } from './describe.js';
import { parseBuild, toLoadout, toSlef, edsyUrl } from './interchange.js';
import { engineeringOptions, isGuardian, slotCandidates } from './catalog.js';
import { makeRng, randomBuild } from './oracle-lib.js';
import { assignPriorities, tidyInternalSlots } from './arrange.js';

const rng = makeRng(11);
const { rand, pick } = rng;
const shipIds = Object.keys(Ships).sort();
const fin = (x: number) => (Number.isFinite(x) ? x : null);

// ---------------------------------------------------------------- builds

const builds: { ship: string; code: string; label: string }[] = [];
const add = (ship: Ship, label: string) => builds.push({ ship: ship.id, code: coriolisCode(ship), label });
for (const id of shipIds) add(stockShip(id), 'stock');
for (let i = 0; i < 2 * shipIds.length; i++) add(randomBuild(shipIds[i % shipIds.length], rng).ship, 'random');
// Builds that exercise the corai-only weapon effects: a Thermal Vent beam and a Corrosive Shell
// multi-cannon in random hardpoints of random builds.
const fitWeapon = (ship: Ship, grp: string, eng: { blueprint: string; grade: number; special: string }) => {
  const refs = listSlots(ship).filter((r) => r.kind === 'hardpoint');
  const r = refs.length ? pick(refs) : undefined;
  const m = r && (Modules.hardpoints[grp] ?? []).filter((x: any) => x.class <= r.maxClass && !x.preEngineered && x.id.length === 2);
  if (r && m?.length) place(ship, r, pick(m), eng);
};
for (let i = 0; i < 24; i++) {
  const ship = randomBuild(shipIds[(i * 7) % shipIds.length], rng).ship;
  if (rand() < 0.8) fitWeapon(ship, 'bl', { blueprint: 'Weapon_LongRange', grade: 5, special: 'special_thermal_vent' });
  if (rand() < 0.8) fitWeapon(ship, 'mc', { blueprint: 'Weapon_HighCapacity', grade: 4, special: 'special_corrosive_shell' });
  recalcAll(ship);
  add(ship, 'vent+corrosive');
}
const load = (b: { ship: string; code: string }) => shipFromCoriolis(b.code, b.ship);

// ---------------------------------------------------------------- metrics

/** Contexts as plain data (opponent by ship id) so the Rust tests can rebuild them. */
const CONTEXTS: Record<string, any> = {
  default: {},
  pveRecovery: {
    damagePreset: 'pve', sys: 4, wep: 2, recoverySys: 2, incomingDps: 300, regenUnderFire: 0.5,
    range: 2000, reserveFuelInJumpMass: true, opponent: 'federal_corvette', opponentSys: 4,
  },
  weaponsHot: {
    damage: { kinetic: 2, thermal: 1, explosive: 0.5, absolute: 0.2 }, sys: 0, wep: 0, range: 500,
    ventHitRate: 0.6, thermalVentFactor: 4.3, corrosiveHullMultiplier: 1.4, corrosivePiercingBonus: 30,
  },
};
const contexts = Object.fromEntries(Object.entries(CONTEXTS).map(([k, o]) => [k, makeContext({ ...o, opponent: o.opponent ? stockShip(o.opponent) : undefined })]));
const DYNAMIC = ['count:fs', 'count:fh', 'count:hs', 'count:sb', 'count:scb', 'class:sg', 'class:fs', 'class:cr'];
const metricDefs = [...METRICS, ...DYNAMIC.map(getMetric)];

const metricCases = builds.map((b) => {
  const ship = load(b);
  const values: Record<string, Record<string, number | null>> = {};
  for (const [name, ctx] of Object.entries(contexts)) {
    values[name] = Object.fromEntries(metricDefs.map((d) => [d.name, fin(d.compute(ship, ctx))]));
  }
  return { ...b, values };
});

// ---------------------------------------------------------------- legality

/** Places a module definition in a slot without any legality check (Coriolis' own `use`). */
function force(ship: Ship, key: string, mdef: any) {
  ship.use(findSlot(ship, key).slot, mdef, true);
}
const allOf = (groups: Record<string, any[]>) => Object.values(groups).flat().filter((m) => typeof m.id === 'string' && m.id.length === 2);
const internalMods = allOf(Modules.internal);
const hardpointMods = allOf(Modules.hardpoints);

type Mutation = { name: string; apply: (ship: Ship) => boolean };
const MUTATIONS: Mutation[] = [
  { name: 'life support downsized', apply: (s) => downsize(s, 3, 'ls') },
  { name: 'sensors downsized', apply: (s) => downsize(s, 5, 's') },
  { name: 'oversized internal', apply: (s) => {
    const r = listSlots(s).find((x) => x.kind === 'internal' && !x.name && x.maxClass < 8);
    const m = r && internalMods.find((x) => x.grp === 'cr' && x.class === r.maxClass + 1);
    if (!r || !m) return false;
    force(s, r.key, m);
    return true;
  } },
  { name: 'two shield generators', apply: (s) => {
    const free = listSlots(s).filter((x) => x.kind === 'internal' && !x.name && x.maxClass >= 3);
    const sg = internalMods.find((x) => x.grp === 'sg' && x.class === 3);
    if (free.length < 2 || !sg) return false;
    force(s, free[0].key, sg);
    force(s, free[1].key, sg);
    return true;
  } },
  { name: 'cargo rack in a military slot', apply: (s) => {
    const r = listSlots(s).find((x) => x.kind === 'internal' && x.name === 'Military');
    const m = r && internalMods.find((x) => x.grp === 'cr' && x.class <= r.maxClass);
    if (!r || !m) return false;
    force(s, r.key, m);
    return true;
  } },
  { name: 'five experimental weapons', apply: (s) => {
    const hps = listSlots(s).filter((x) => x.kind === 'hardpoint');
    const exp = hardpointMods.filter((x) => x.experimental && x.class >= 1);
    if (hps.length < 5) return false;
    let n = 0;
    for (const r of hps) {
      const m = exp.find((x) => x.class <= r.maxClass && !x.ship);
      if (m) { force(s, r.key, m); n++; }
    }
    return n >= 5;
  } },
  { name: 'engineered Guardian weapon', apply: (s) => {
    const r = listSlots(s).find((x) => x.kind === 'hardpoint');
    const m = r && hardpointMods.find((x) => isGuardian(x) && !x.preEngineered && x.class <= r.maxClass && x.class >= 1);
    if (!r || !m) return false;
    force(s, r.key, m);
    const placed = findSlot(s, r.key).slot.m;
    const bp = BlueprintFunctions.getBlueprint('Weapon_Overcharged', placed);
    if (!bp?.grades) return false;
    bp.grade = 5;
    s.setModuleBlueprint(placed, bp);
    BlueprintFunctions.setQualityCB(bp, 1, (name: string, value: number) => s.setModification(placed, name, value, false, true, true));
    return true;
  } },
  { name: 'module of another ship', apply: (s) => {
    const r = listSlots(s).find((x) => x.kind === 'internal' && !x.name);
    const m = r && internalMods.find((x) => {
      const only = Array.isArray(x.ship) ? x.ship : x.ship ? [x.ship] : [];
      return only.length && !only.includes(s.id) && x.class <= r.maxClass;
    });
    if (!r || !m) return false;
    force(s, r.key, m);
    return true;
  } },
];

function downsize(ship: Ship, index: number, grp: string): boolean {
  const slot = ship.standard[index];
  const cls = slot.maxClass;
  const m = (Modules.standard[grp] ?? []).find((x: any) => x.class < cls);
  if (!m) return false;
  ship.use(slot, m, true);
  return true;
}

const legalityCases: any[] = [];
for (const b of builds) {
  const ship = load(b);
  legalityCases.push({ ship: b.ship, code: b.code, label: b.label, violations: validateBuild(ship) });
}
for (const mut of MUTATIONS) {
  let made = 0;
  for (const id of shipIds) {
    if (made >= 4) break;
    const ship = randomBuild(id, rng).ship;
    if (!mut.apply(ship)) continue;
    recalcAll(ship);
    const code = coriolisCode(ship);
    // Keep only mutations the share code preserves.
    const again = shipFromCoriolis(code, id);
    legalityCases.push({ ship: id, code, label: mut.name, violations: validateBuild(again) });
    made++;
  }
}

// ---------------------------------------------------------------- summaries

const summaryCases = builds.filter((_, i) => i % 3 === 0).map((b) => {
  const ship = load(b);
  return { ship: b.ship, code: b.code, summary: summarize(ship, contexts.default, { buildName: 'Oracle' }) };
});

// ---------------------------------------------------------------- import / export

/** Coriolis' own SLEF (with journal-style Modifiers), values nudged as real rolls would be. */
function coriolisSlef(ship: Ship): any {
  const slef = Serializer.toSLEF('Oracle', ship);
  const data = Array.isArray(slef) ? slef[0].data : slef.data ?? slef;
  for (const mod of data.Modules ?? []) {
    for (const mf of mod.Engineering?.Modifiers ?? []) {
      if (typeof mf.Value !== 'number' || typeof mf.OriginalValue !== 'number') continue;
      const roll = 0.6 + 0.4 * rand();
      mf.Value = Math.round((mf.OriginalValue + (mf.Value - mf.OriginalValue) * roll) * 1e6) / 1e6;
    }
  }
  return slef;
}

const interchangeCases = builds.filter((_, i) => i % 2 === 0).map((b) => {
  const ship = load(b);
  const slef = toSlef(ship, 'Oracle');
  const loadout = toLoadout(ship, 'Oracle');
  const journalLine = JSON.stringify({ timestamp: '3312-11-11T12:00:00Z', ...loadout, ShipID: 7 });
  const rolled = coriolisSlef(ship);
  const inputs: Record<string, string> = {
    slef: JSON.stringify(slef, null, 2),
    loadout: JSON.stringify(loadout),
    journalLine,
    coriolisUrl: `https://coriolis.io/outfit/${b.ship}?code=${encodeURIComponent(b.code)}`,
    edsyUrl: edsyUrl(ship, 'Oracle'),
    coriolisSlefRolled: JSON.stringify(rolled),
  };
  const imports: Record<string, any> = {};
  for (const [kind, input] of Object.entries(inputs)) {
    try {
      const got = parseBuild(input);
      imports[kind] = { ship: got.id, code: coriolisCode(got), issues: got.importIssues ?? [] };
    } catch (e) {
      imports[kind] = { error: e instanceof Error ? e.message : String(e) };
    }
  }
  return { ship: b.ship, code: b.code, exports: { slef, loadout }, inputs, imports };
});

// ---------------------------------------------------------------- catalog

// What the search may put in every slot of every hull (default filters: everything allowed), and
// the engineering options of every module that appears.
const catalogShips: Record<string, Record<string, string[]>> = {};
const engineering: Record<string, string[]> = {};
for (const id of shipIds) {
  const ship = stockShip(id);
  const slots: Record<string, string[]> = {};
  for (const ref of listSlots(ship)) {
    const cands = slotCandidates(ship, ref, {});
    slots[ref.key] = cands.map((m: any) => (ref.kind === 'bulkheads' ? `bh/${m.bulkheadIndex}` : `${m.grp}/${m.id}`));
    for (const m of cands) {
      if (ref.kind === 'bulkheads') continue;
      engineering[`${m.grp}/${m.id}`] ??= engineeringOptions(m).map((e) => `${e.blueprint}:${e.grade}:${e.special ?? ''}`);
    }
  }
  catalogShips[id] = slots;
}

// ---------------------------------------------------------------- arrange

// Power priorities and internal slot tidying on random builds, with and without locked slots.
const arrangeCases = builds.filter((b) => b.label !== 'stock').map((b) => {
  const out: Record<string, { code: string; moved: boolean }> = {};
  for (const [name, locked] of [['free', () => false], ['everyThirdLocked', (r: any) => r.kind === 'internal' && r.index % 3 === 0]] as const) {
    const ship = load(b);
    assignPriorities(ship);
    const moved = tidyInternalSlots(ship, locked as (r: any) => boolean);
    out[name] = { code: coriolisCode(ship), moved };
  }
  return { ship: b.ship, code: b.code, ...out };
});

// ---------------------------------------------------------------- write

const write = (path: string, data: unknown) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify({ generator: 'corai src/oracle-corai.ts', ...data as object }) + '\n');
};
write('crates/corai-opt/tests/fixtures/metrics.json', { contexts: CONTEXTS, metrics: metricDefs.map((d) => d.name), cases: metricCases });
write('crates/corai-opt/tests/fixtures/legality.json', { cases: legalityCases });
write('crates/corai-opt/tests/fixtures/summary.json', { cases: summaryCases });
write('crates/corai-opt/tests/fixtures/catalog.json', { ships: catalogShips, engineering });
write('crates/corai-opt/tests/fixtures/arrange.json', { cases: arrangeCases });
write('crates/corai-engine/tests/fixtures/interchange.json', { cases: interchangeCases });
process.stderr.write(`oracle-corai: ${metricCases.length} metric builds, ${legalityCases.length} legality, ${summaryCases.length} summaries, ${interchangeCases.length} interchange\n`);
