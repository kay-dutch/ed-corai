// Typed façade over the upstream Coriolis shipyard engine (bundled from upstream/coriolis).
import * as E from '../engine/entry.js';

export const { Ships, Modules, Modifications, Calc, Constants, BlueprintFunctions, ModuleUtils, JournalUtils, CompanionApiUtils, LinkMigration } = E;
export const ModuleGroupToName: Record<string, string> = E.Constants.ModuleGroupToName;

// Memoise Module.get(): it recomputes engineered values from scratch on every call and dominates
// optimiser run time. A module's value depends only on its own base stats, `mods` (replaced
// wholesale or written via setModValue) and its blueprint/special, so those key the cache.
{
  const proto = E.Module.prototype;
  const origGet = proto.get;
  const origSet = proto.setModValue;
  proto.setModValue = function (this: any, ...args: unknown[]) {
    this.__ver = (this.__ver ?? 0) + 1;
    return origSet.apply(this, args);
  };
  proto.get = function (this: any, name: string, modified = true) {
    const bp = this.blueprint;
    let c = this.__cache;
    if (!c || c.mods !== this.mods || c.ver !== this.__ver || c.bp !== bp || c.special !== bp?.special) {
      c = this.__cache = { mods: this.mods, ver: this.__ver, bp, special: bp?.special, vals: new Map() };
    }
    const key = modified ? name : `!${name}`;
    if (c.vals.has(key)) return c.vals.get(key);
    const v = origGet.call(this, name, modified);
    c.vals.set(key, v);
    return v;
  };
}

export type Ship = any;
export type Mod = any;
export type SlotKind = 'standard' | 'bulkheads' | 'hardpoint' | 'utility' | 'internal';

export const STANDARD_KEYS = ['pp', 't', 'fsd', 'ls', 'pd', 's', 'ft'] as const;

export interface SlotRef {
  key: string;          // e.g. "standard:fsd", "bulkheads", "hardpoint:2", "utility:1", "internal:5"
  kind: SlotKind;
  index: number;        // index into the engine's slot array (bulkhead index for bulkheads)
  slot: any;            // the engine slot object
  maxClass: number;
  name?: string;        // restricted slot label, e.g. "Military", "Cargo", "Mining"
  eligible?: Record<string, number>;
}

export interface EngSpec {
  blueprint: string;    // Coriolis blueprint key, e.g. "FSD_LongRange"
  grade: number;
  special?: string | null; // special key, e.g. "special_fsd_heavy"
}

export function shipIds(): string[] {
  return Object.keys(Ships).sort();
}

export function shipData(shipId: string): any {
  const d = Ships[shipId];
  if (!d) throw new Error(`Unknown ship "${shipId}". Use list_ships to see valid ids.`);
  return d;
}

/** Stock (factory default) build. */
export function stockShip(shipId: string): Ship {
  const d = shipData(shipId);
  return new E.Ship(shipId, d.properties, d.slots).buildWith(d.defaults);
}

/** Parses a Coriolis outfit URL (or bare `code`) into a ship. */
export function shipFromCoriolis(urlOrCode: string, shipIdHint?: string): Ship {
  let shipId = shipIdHint;
  let code = urlOrCode.trim();
  const m = code.match(/\/outfit\/([a-z0-9_]+)\/?(?:\?(.*))?$/i);
  if (m) {
    shipId = m[1];
    const params = new URLSearchParams(m[2] ?? '');
    code = params.get('code') ?? '';
  }
  if (!shipId) throw new Error('Cannot tell the ship type: pass a full Coriolis outfit URL or a ship id with the code.');
  const d = shipData(shipId);
  const ship = new E.Ship(shipId, d.properties, d.slots);
  if (code) ship.buildFrom(code);
  else ship.buildWith(d.defaults);
  recalcAll(ship);
  return ship;
}

export function coriolisCode(ship: Ship): string {
  ship.updateModificationsString();
  return ship.toString();
}

export function coriolisUrl(ship: Ship, buildName?: string): string {
  let url = `https://coriolis.io/outfit/${ship.id}?code=${encodeURIComponent(coriolisCode(ship))}`;
  if (buildName) url += `&bn=${encodeURIComponent(buildName)}`;
  return url;
}

export function cloneShip(ship: Ship): Ship {
  return shipFromCoriolis(coriolisCode(ship), ship.id);
}

/** Recomputes every derived stat. The engine's incremental updates skip some dependencies. */
export function recalcAll(ship: Ship): Ship {
  return ship.updatePowerGenerated()
    .updatePowerUsed()
    .recalculateMass()
    .updateJumpStats()
    .recalculateShield()
    .recalculateShieldCells()
    .recalculateArmour()
    .recalculateDps()
    .recalculateEps()
    .recalculateHps()
    .updateMovement();
}

export function listSlots(ship: Ship): SlotRef[] {
  const out: SlotRef[] = [];
  ship.standard.forEach((slot: any, i: number) => {
    if (i < STANDARD_KEYS.length) out.push({ key: `standard:${STANDARD_KEYS[i]}`, kind: 'standard', index: i, slot, maxClass: slot.maxClass });
  });
  out.push({ key: 'bulkheads', kind: 'bulkheads', index: 0, slot: ship.bulkheads, maxClass: 0 });
  let h = 0, u = 0;
  ship.hardpoints.forEach((slot: any, i: number) => {
    const kind = slot.maxClass > 0 ? 'hardpoint' : 'utility';
    const key = kind === 'hardpoint' ? `hardpoint:${++h}` : `utility:${++u}`;
    out.push({ key, kind, index: i, slot, maxClass: slot.maxClass, name: slot.name, eligible: slot.eligible });
  });
  ship.internal.forEach((slot: any, i: number) => {
    out.push({ key: `internal:${i + 1}`, kind: 'internal', index: i, slot, maxClass: slot.maxClass, name: slot.name, eligible: slot.eligible });
  });
  return out;
}

export function findSlot(ship: Ship, key: string): SlotRef {
  const ref = listSlots(ship).find((s) => s.key === key);
  if (!ref) throw new Error(`Unknown slot "${key}" for ${ship.name}. Use ship_details to list slots.`);
  return ref;
}

export function bulkheadIndex(ship: Ship): number {
  const names: any[] = shipData(ship.id).bulkheads;
  const idx = names.findIndex((b) => b.id === ship.bulkheads.m?.id);
  return idx < 0 ? 0 : idx;
}

/** Puts a module (or null to empty) into a slot. Stats are fully recomputed. */
export function place(ship: Ship, ref: SlotRef, mdef: any | null, eng?: EngSpec | null): void {
  if (ref.kind === 'bulkheads') {
    ship.useBulkhead(mdef?.bulkheadIndex ?? 0, true);
  } else {
    ship.use(ref.slot, mdef, true);
  }
  const m = ref.kind === 'bulkheads' ? ship.bulkheads.m : ref.slot.m;
  if (m && eng) applyEngineering(ship, m, eng);
  recalcAll(ship);
}

export interface SlotSnapshot { m: any }

export function snapshot(ref: SlotRef): SlotSnapshot {
  return { m: ref.slot.m };
}

/** Restores the exact module instance (with its engineering) captured by snapshot(). */
export function restore(ship: Ship, ref: SlotRef, snap: SlotSnapshot): void {
  if (ref.kind === 'bulkheads') {
    const cur = ship.bulkheads.m;
    ship.bulkheads.m = snap.m;
    ship.bulkheads.discountedCost = snap.m.cost * ship.moduleCostMultiplier;
    ship.updateStats(ship.bulkheads, snap.m, cur, true);
  } else {
    ship.use(ref.slot, snap.m, true);
  }
  recalcAll(ship);
}

/** Applies a blueprint at full (100%) roll plus an optional experimental effect. Does not recalc. */
export function applyEngineering(ship: Ship, m: Mod, eng: EngSpec): void {
  const special = eng.special ? Modifications.specials[eng.special] : null;
  if (eng.special && !special) throw new Error(`Unknown experimental effect "${eng.special}".`);
  if (m.preEngineered) {
    // Pre-engineered modules keep their fixed blueprints; only an experimental may be added.
    if (special && m.preEngineered.canApplyExperimental && m.blueprint) m.blueprint.special = special;
    return;
  }
  const bp = BlueprintFunctions.getBlueprint(eng.blueprint, m);
  if (!bp || !bp.grades) throw new Error(`Unknown blueprint "${eng.blueprint}".`);
  if (!bp.grades[eng.grade]) throw new Error(`Blueprint ${eng.blueprint} has no grade ${eng.grade}.`);
  bp.grade = eng.grade;
  m.blueprint = bp;
  m.mods = {};
  BlueprintFunctions.setQualityCB(bp, 1, (feature: string, value: number) => ship.setModification(m, feature, value, false, true));
  if (special) m.blueprint.special = special;
}

export function engineeringOf(m: Mod): EngSpec | null {
  const bp = m?.blueprint;
  if (!bp || !bp.fdname) return null;
  const key = Object.keys(Modifications.blueprints).find((k) => Modifications.blueprints[k].fdname === bp.fdname) ?? bp.fdname;
  const specialKey = bp.special ? Object.keys(Modifications.specials).find((k) => Modifications.specials[k] === bp.special || Modifications.specials[k].edname === bp.special.edname) : null;
  return { blueprint: key, grade: bp.grade, special: specialKey ?? null };
}

export function moduleLabel(m: Mod): string {
  if (!m) return 'empty';
  const base = m.name || ModuleGroupToName[m.grp] || m.grp;
  const cr = m.class !== undefined && m.rating ? `${m.class}${m.rating} ` : '';
  const mount = m.mount ? ` (${({ F: 'fixed', G: 'gimballed', T: 'turret' } as Record<string, string>)[m.mount] ?? m.mount})` : '';
  const missile = m.missile ? ` [${m.missile === 'D' ? 'dumbfire' : m.missile === 'S' ? 'seeker' : m.missile}]` : '';
  return `${cr}${base}${mount}${missile}`.trim();
}

export function engineeringLabel(m: Mod): string | null {
  const bp = m?.blueprint;
  if (!bp || !bp.fdname) return null;
  const name = bp.name || bp.fdname;
  const special = bp.special ? ` + ${bp.special.name}` : '';
  return `${name} G${bp.grade}${special}${m.preEngineered ? ' (pre-engineered)' : ''}`;
}
