// Post-search arrangement: power priorities and slot placement. Neither changes any stat the
// optimiser scores; both make the build look and behave like one a player would set up.
import { listSlots, recalcAll, type Ship, type SlotRef } from './engine.js';
import { fitsSlot } from './catalog.js';

/**
 * Modules that do nothing in a fight. With priority-aware power they sit in the lowest priority
 * group, which the game sheds first when hardpoints are deployed.
 */
const NON_COMBAT_GROUPS = new Set(['fs', 'dc', 'sua', 'pv', 'rf', 'cc', 'pc', 'mlc', 'fx', 'hb', 'ss', 'sc', 'pce', 'pci', 'pcm', 'pcq']);

export function isNonCombat(m: any): boolean {
  return Boolean(m && NON_COMBAT_GROUPS.has(m.grp));
}

/** Power the build needs in a fight: everything except non-combat modules, hardpoints deployed. */
export function combatDeployedDraw(ship: Ship): number {
  let draw = 0;
  for (const slot of [...ship.standard, ...ship.internal, ...ship.hardpoints]) {
    if (slot.m && slot.enabled && !isNonCombat(slot.m)) draw += slot.m.getPowerUsage() || 0;
  }
  return draw;
}

/** Puts combat modules in priority group 1 and non-combat modules (and the cargo hatch) in 5. */
export function assignPriorities(ship: Ship): void {
  for (const slot of [...ship.standard, ...ship.internal, ...ship.hardpoints]) {
    slot.priority = isNonCombat(slot.m) ? 4 : 0;
  }
  ship.cargoHatch.priority = 4;
  ship.updatePowerUsed();
  ship.updatePowerPrioritesString();
  recalcAll(ship);
}

/** Minimum-cost assignment (Hungarian algorithm) of n rows to m >= n columns. */
function assign(cost: number[][]): number[] {
  const n = cost.length;
  const m = cost[0]?.length ?? 0;
  const INF = 1e18;
  const u = new Array(n + 1).fill(0), v = new Array(m + 1).fill(0);
  const p = new Array(m + 1).fill(0), way = new Array(m + 1).fill(0);
  for (let i = 1; i <= n; i++) {
    p[0] = i;
    let j0 = 0;
    const minv = new Array(m + 1).fill(INF);
    const used = new Array(m + 1).fill(false);
    do {
      used[j0] = true;
      const i0 = p[j0];
      let delta = INF, j1 = 0;
      for (let j = 1; j <= m; j++) {
        if (used[j]) continue;
        const cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
        if (cur < minv[j]) { minv[j] = cur; way[j] = j0; }
        if (minv[j] < delta) { delta = minv[j]; j1 = j; }
      }
      for (let j = 0; j <= m; j++) {
        if (used[j]) { u[p[j]] += delta; v[j] -= delta; } else minv[j] -= delta;
      }
      j0 = j1;
    } while (p[j0] !== 0);
    do { const j1 = way[j0]; p[j0] = p[j1]; j0 = j1; } while (j0);
  }
  const rowToCol = new Array(n).fill(-1);
  for (let j = 1; j <= m; j++) if (p[j]) rowToCol[p[j] - 1] = j - 1;
  return rowToCol;
}

/**
 * Moves internal modules into the smallest slots they fit (respecting restricted and military
 * slots), so a size-1 module does not sit in a size-5 slot while a size-1 slot is free. Locked slots
 * stay put. Returns true when anything moved.
 */
export function tidyInternalSlots(ship: Ship, isLocked: (ref: SlotRef) => boolean): boolean {
  const refs = listSlots(ship).filter((r) => r.kind === 'internal' && !isLocked(r));
  const mods = refs.map((r) => r.slot.m).filter(Boolean);
  if (mods.length < 2) return false;
  const BAD = 1e9;
  // Cost: squared unused slot size, so one size-1 module in a size-7 slot costs more than spreading
  // the gap (when every slot is filled the plain sum is constant). A hair for moving at all keeps
  // the current layout on ties.
  const cost = mods.map((m: any) => refs.map((r) => (fitsSlot(ship, r, m) ? (r.maxClass - m.class) ** 2 + (r.slot.m === m ? 0 : 1e-3) : BAD)));
  const plan = assign(cost);
  if (plan.some((c, i) => c < 0 || cost[i][c] >= BAD)) return false;
  if (plan.every((c, i) => refs[c].slot.m === mods[i])) return false;
  const state = refs.map((r) => ({ m: r.slot.m, enabled: r.slot.enabled, priority: r.slot.priority }));
  for (const r of refs) ship.use(r.slot, null, true);
  plan.forEach((c, i) => {
    const from = state[refs.findIndex((r, k) => state[k].m === mods[i])];
    ship.use(refs[c].slot, mods[i], true);
    refs[c].slot.enabled = from.enabled;
    refs[c].slot.priority = from.priority;
  });
  ship.serialized.internal = null;
  recalcAll(ship);
  return true;
}
