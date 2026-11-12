// Shared by the differential-test oracles: a seeded random source and random engineered builds.
import { listSlots, place, recalcAll, stockShip, type EngSpec, type Ship } from './engine.js';
import { engineeringOptions, slotCandidates } from './catalog.js';

export interface SlotSpec { group: string; id: string; eng?: EngSpec | null }

/** mulberry32: deterministic for a seed, identical in every oracle run. */
export function makeRng(seed: number) {
  let state = seed >>> 0;
  const rand = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const pick = <T,>(xs: T[]): T => xs[Math.floor(rand() * xs.length)];
  return { rand, pick };
}

// Module id "0" (a tbsc variant) is the only non-two-character id; Coriolis share codes assume two
// characters, so builds containing it cannot round-trip through a URL. Leave it out.
export const ORACLE_FILTERS = { allowCommunityGoal: true, excludeModules: ['0'] };

/** A legal build with random modules (some slots left empty) and random engineering. */
export function randomBuild(shipId: string, rng: ReturnType<typeof makeRng>) {
  const { rand, pick } = rng;
  const ship: Ship = stockShip(shipId);
  const spec: Record<string, SlotSpec | null> = {};
  const fill = rand(); // how full this build is
  for (const ref of listSlots(ship)) {
    let cands = slotCandidates(ship, ref, ORACLE_FILTERS);
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
