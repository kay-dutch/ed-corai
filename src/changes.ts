// Slot-level edits to a build ("what if I swap this module / blueprint?"), shared by the MCP
// server and the CLI.
import { findSlot, place, shipData, type EngSpec, type Ship } from './engine.js';
import { maxBlueprintGrade, slotCandidates } from './catalog.js';

export interface BuildChange {
  slot: string;
  empty?: boolean;
  moduleId?: string;
  group?: string;
  class?: number;
  rating?: string;
  mount?: 'F' | 'G' | 'T';
  bulkheadIndex?: number;
  blueprint?: string;
  grade?: number;
  special?: string;
}

export function applyChange(ship: Ship, c: BuildChange): void {
  const ref = findSlot(ship, c.slot);
  if (c.empty) { place(ship, ref, null); return; }
  let mdef: any;
  if (ref.kind === 'bulkheads') {
    const idx = c.bulkheadIndex ?? 0;
    mdef = { ...shipData(ship.id).bulkheads[idx], grp: 'bh', bulkheadIndex: idx };
  } else if (c.moduleId || c.group) {
    const cands = slotCandidates(ship, ref, {});
    mdef = cands.find((m) => (c.moduleId ? m.id === c.moduleId : m.grp === c.group && (c.class === undefined || m.class === c.class) && (c.rating === undefined || m.rating === c.rating) && (c.mount === undefined || m.mount === c.mount)));
    if (!mdef) throw new Error(`No module matching ${JSON.stringify(c)} fits ${c.slot}.`);
  } else {
    mdef = ref.slot.m ? { grp: ref.slot.m.grp, id: ref.slot.m.id } : null; // re-engineer current module
    if (!mdef) throw new Error(`${c.slot} is empty; give moduleId or group.`);
  }
  const eng: EngSpec | null = c.blueprint
    ? { blueprint: c.blueprint, grade: c.grade ?? maxBlueprintGrade(mdef.grp, c.blueprint), special: c.special ?? null }
    : null;
  place(ship, ref, mdef, eng);
}
