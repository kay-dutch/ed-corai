// Differential-test oracle: random engineered builds evaluated by the upstream Coriolis engine.
// Output feeds crates/corai-engine/tests (`npm run oracle` regenerates the fixture).
//   node dist/oracle.cjs [count] [seed] > crates/corai-engine/tests/fixtures/oracle.json
import './stdio-guard.js';
import LZString from 'lz-string';
import { Calc, LinkMigration, Modules, Ships, coriolisCode, listSlots, shipFromCoriolis, stockShip, type Ship } from './engine.js';
import { makeRng, randomBuild as randomBuildWith } from './oracle-lib.js';

const [count = 400, seed = 7] = process.argv.slice(2).map(Number);
const rng = makeRng(seed);
const { rand, pick } = rng;
const opponent = stockShip('anaconda');
const fin = (x: number) => (Number.isFinite(x) ? x : null);
const randomBuild = (shipId: string) => randomBuildWith(shipId, rng);

// Per-module figures, so a mismatch in a ship total points at the module that causes it.
function moduleRecord(m: any) {
  const n = (x: any) => (typeof x === 'number' && Number.isFinite(x) ? x : null);
  return {
    grp: m.grp ?? 'bh', mass: n(m.getMass()), power: n(m.getPowerUsage()),
    damage: n(m.getDamage?.()), rof: n(m.getRoF?.()), dps: n(m.getDps?.()), sdps: n(m.getSDps?.()),
    eps: n(m.getEps?.()), hps: n(m.getHps?.()),
  };
}

function record(ship: Ship) {
  const sh = Calc.shieldMetrics(ship, 2);
  const ar = Calc.armourMetrics(ship);
  const vs = Calc.sustainedDps(ship, opponent, 2, 1000);
  const sum = (o: any) => o.absolute + o.explosive + o.kinetic + o.thermal;
  const typed = (o: any) => (o ? { generator: o.generator, boosters: o.boosters, total: o.total } : null);
  return {
    dryMass: ship.dryMass, unladenMass: ship.unladenMass, ladenMass: ship.ladenMass,
    fuelCapacity: ship.fuelCapacity, cargoCapacity: ship.cargoCapacity, passengerCapacity: ship.passengerCapacity,
    powerAvailable: ship.powerAvailable, powerRetracted: ship.powerRetracted, powerDeployed: ship.powerDeployed,
    topSpeed: ship.topSpeed, topBoost: ship.topBoost, topPitch: ship.topPitch, topRoll: ship.topRoll, topYaw: ship.topYaw,
    shield: ship.shield, shieldCells: ship.shieldCells, armour: ship.armour,
    totalDps: ship.totalDps, totalSDps: ship.totalSDps, totalEps: ship.totalEps, totalHps: ship.totalHps,
    totalCost: ship.totalCost,
    unladenRange: ship.unladenRange, fullTankRange: ship.fullTankRange, ladenRange: ship.ladenRange,
    canBoost: ship.canBoost(ship.cargoCapacity, ship.fuelCapacity),
    shieldMetrics: sh.generator ? {
      generator: sh.generator, boosters: sh.boosters, addition: sh.addition, summary: sh.summary,
      recover: fin(sh.recover), recharge: fin(sh.recharge),
      absolute: typed(sh.absolute), explosive: typed(sh.explosive), kinetic: typed(sh.kinetic), thermal: typed(sh.thermal),
    } : null,
    armourMetrics: {
      bulkheads: ar.bulkheads, reinforcement: ar.reinforcement, total: ar.total, moduleArmour: ar.modulearmour, moduleProtection: ar.moduleprotection,
      explosive: ar.explosive.total, kinetic: ar.kinetic.total, thermal: ar.thermal.total, caustic: ar.caustic.total,
    },
    vsAnaconda: { shields: sum(vs.shieldsdps), armour: sum(vs.armoursdps), eps: vs.eps },
    timeToDrainWep: fin(Calc.timeToDrainWep(ship, 2)),
    modules: Object.fromEntries(listSlots(ship).filter((r) => r.slot.m).map((r) => [r.key, moduleRecord(r.slot.m)])),
  };
}

const ids = Object.keys(Ships).sort();
const cases = [];
for (let i = 0; i < count; i++) {
  const shipId = ids[i % ids.length];
  const { ship, spec } = randomBuild(shipId);
  // The Coriolis share code, and what Coriolis itself rebuilds from it (values are rounded on save).
  const code = coriolisCode(ship);
  cases.push({ ship: shipId, slots: spec, expect: record(ship), code, decoded: record(shipFromCoriolis(code, shipId)) });
}
// Pre-v3 ('A') share codes for ships whose slot layout changed since: Coriolis decodes them
// against the old layout and remaps modules onto the current slots.
const flat = (groups: Record<string, any[]>) => Object.values(groups).flat().filter((m) => typeof m.id === 'string' && m.id.length === 2);
const internals = flat(Modules.internal);
const hardpointMods = flat(Modules.hardpoints);
const urlSafe = (x: string) => x.replace(/\//g, '-').replace(/\+/g, '_');
const legacy = [];
for (const shipId of Object.keys(Ships).sort()) {
  const oldInt = LinkMigration.OLD_INTERNAL_LAYOUTS[shipId];
  const oldHp = LinkMigration.OLD_HARDPOINT_LAYOUTS[shipId];
  if (!oldInt && !oldHp) continue;
  const stock = stockShip(shipId);
  const current = coriolisCode(stock).split('.')[0];
  const std = current.slice(2, 2 + 2 * stock.standard.length);
  const hpLayout: number[] = oldHp ?? stock.hardpoints.map((s: any) => s.maxClass);
  const intLayout: number[] = oldInt ? oldInt.map((s) => s.class) : stock.internal.map((s: any) => s.maxClass);
  for (let n = 0; n < 6; n++) {
    const hp = hpLayout.map((c) => {
      const fits = hardpointMods.filter((m) => (c === 0 ? m.class === 0 : m.class > 0 && m.class <= c));
      return rand() < 0.25 || !fits.length ? '-' : pick(fits).id;
    }).join('');
    const int = intLayout.map((c) => {
      const fits = internals.filter((m) => m.class <= c);
      return rand() < 0.25 || !fits.length ? '-' : pick(fits).id;
    }).join('');
    const len = 1 + stock.standard.length + hpLayout.length + intLayout.length;
    const flags = (alphabet: string) => urlSafe(LZString.compressToBase64(Array.from({ length: len }, () => pick([...alphabet])).join('')));
    const code = `A${Math.floor(rand() * 3)}${std}${hp}${int}.${flags('01')}.${flags('01234')}.`;
    legacy.push({ ship: shipId, code, decoded: record(shipFromCoriolis(code, shipId)) });
  }
}
process.stdout.write(JSON.stringify({ generator: 'corai src/oracle.ts', seed, count, cases, legacy }) + '\n');
