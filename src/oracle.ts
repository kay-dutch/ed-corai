// Differential-test oracle: random engineered builds evaluated by the upstream Coriolis engine.
// Output feeds crates/corai-engine/tests (`npm run oracle` regenerates the fixture).
//   node dist/oracle.cjs [count] [seed] > crates/corai-engine/tests/fixtures/oracle.json
import './stdio-guard.js';
import { Calc, Ships, coriolisCode, listSlots, place, recalcAll, shipFromCoriolis, stockShip, type EngSpec, type Ship } from './engine.js';
import { engineeringOptions, slotCandidates } from './catalog.js';

const [count = 400, seed = 7] = process.argv.slice(2).map(Number);

let state = seed >>> 0;
const rand = () => {
  state = (state + 0x6d2b79f5) >>> 0;
  let t = state;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const pick = <T,>(xs: T[]): T => xs[Math.floor(rand() * xs.length)];

interface SlotSpec { group: string; id: string; eng?: EngSpec | null }

const opponent = stockShip('anaconda');
// Module id "0" (a tbsc variant) is the only non-two-character id; Coriolis share codes assume two
// characters, so builds containing it cannot round-trip through a URL. Leave it out.
const filters = { allowCommunityGoal: true, excludeModules: ['0'] };
const fin = (x: number) => (Number.isFinite(x) ? x : null);

function randomBuild(shipId: string) {
  const ship: Ship = stockShip(shipId);
  const spec: Record<string, SlotSpec | null> = {};
  const fill = rand(); // how full this build is
  for (const ref of listSlots(ship)) {
    let cands = slotCandidates(ship, ref, filters);
    if (ref.kind !== 'standard' && ref.kind !== 'bulkheads' && rand() > fill) cands = [];
    const m = cands.length ? pick(cands) : null;
    let eng: EngSpec | null = null;
    if (m && rand() < 0.7) {
      const opts = engineeringOptions(m);
      if (opts.length) eng = pick(opts);
    }
    place(ship, ref, m, eng);
    spec[ref.key] = m ? { group: m.grp, id: ref.kind === 'bulkheads' ? String(m.bulkheadIndex) : m.id, eng } : null;
  }
  recalcAll(ship);
  return { ship, spec };
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
process.stdout.write(JSON.stringify({ generator: 'corai src/oracle.ts', seed, count, cases }) + '\n');
