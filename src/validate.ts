// Independent legality check: every fitted module must appear in Coriolis' own outfitting list for
// its slot (ModuleSet), and every blueprint in Coriolis' engineering menu for that module. This is
// deliberately not derived from catalog.ts, so a gap in the optimiser's rules shows up here.
import { Modifications, ModuleUtils, listSlots, shipData, STANDARD_KEYS, type Ship } from './engine.js';
import { canEngineer } from './catalog.js';

/**
 * The game and Coriolis name the same blueprint differently per module family (journal
 * `Weapon_Overcharged` is Coriolis `MC_Overcharged` on multi-cannons; `Misc_LightWeight` is
 * `LifeSupport_LightWeight`), so compare the part after the family prefix, as Coriolis' lookup does.
 */
function blueprintOffered(grp: string, name: string): boolean {
  const offered = Object.keys(Modifications.modules[grp]?.blueprints ?? {});
  const suffix = (n: string) => n.toLowerCase().split('_').slice(1).join('_') || n.toLowerCase();
  return offered.some((k) => k === name || Modifications.blueprints[k]?.fdname === name || suffix(k) === suffix(name));
}

/** Modules Coriolis allows once per ship (shield generators of every kind count together). */
const UNIQUE = [['sg', 'bsg', 'psg'], ['rf'], ['fs'], ['fh'], ['gfsb'], ['dc'], ['ews'], ['mlc']];

const label = (m: any) => `${m.class}${m.rating} ${m.name ?? m.grp}`;

export function validateBuild(ship: Ship): string[] {
  const problems: string[] = [];
  const set = ModuleUtils.forShip(ship.id);
  // ModuleSet reads ship flags (hangars, cabins) and the ship id (slot-specific rules).
  const props = { ...shipData(ship.id).properties, id: ship.id };
  for (const ref of listSlots(ship)) {
    const m = ref.slot.m;
    if (!m || ref.kind === 'bulkheads') continue;
    let offered: any[];
    if (ref.kind === 'standard') {
      offered = set.standard[ref.index] ?? [];
    } else if (ref.kind === 'internal') {
      offered = Object.values(set.getInts(props, ref.maxClass, ref.eligible) as Record<string, any[]>).flat();
    } else {
      offered = Object.values(set.getHps(props, ref.maxClass, ref.eligible, ref.slot) as Record<string, any[]>).flat();
    }
    if (!offered.some((o) => o.id === m.id && o.grp === m.grp)) {
      problems.push(`${ref.key}: ${label(m)} is not offered by Coriolis for this slot`);
    }
    const bp = m.blueprint;
    if (bp?.fdname && !m.preEngineered) {
      if (!canEngineer(m)) problems.push(`${ref.key}: ${label(m)} cannot be engineered but carries ${bp.fdname}`);
      else if (!blueprintOffered(m.grp, bp.fdname)) problems.push(`${ref.key}: ${bp.fdname} is not an engineering option for ${m.grp}`);
    }
    const shipsAllowed: string[] = m.ship ? (Array.isArray(m.ship) ? m.ship : [m.ship]) : [];
    if (shipsAllowed.length && !shipsAllowed.includes(ship.id)) problems.push(`${ref.key}: ${label(m)} is restricted to another ship`);
  }
  for (const family of UNIQUE) {
    const slots = listSlots(ship).filter((r) => r.kind === 'internal' && r.slot.m && family.includes(r.slot.m.grp));
    if (slots.length > 1) problems.push(`${slots.map((r) => r.key).join(', ')}: only one ${family.join('/')} allowed`);
  }
  const experimental = ship.hardpoints.filter((h: any) => h.m?.experimental).length;
  if (experimental > 4) problems.push(`${experimental} experimental weapons fitted (max 4)`);
  return problems;
}
