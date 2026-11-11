// Build interchange: Frontier journal "Loadout" JSON, SLEF, and links for Coriolis and EDSY.
//
// Slot names follow the Frontier journal (which both Coriolis and EDSY import): standard slots by
// name, hardpoints numbered per size, internals as SlotNN_SizeM with the hull-specific gaps the
// game uses.
import { gunzipSync, inflateSync, unzipSync, deflateSync } from 'node:zlib';
import {
  BlueprintFunctions, CompanionApiUtils, JournalUtils, Modifications, coriolisUrl, recalcAll, shipData, shipFromCoriolis, type Ship,
} from './engine.js';

import { BUILD } from './version.js';
const STANDARD_SLOTS = ['PowerPlant', 'MainEngines', 'FrameShiftDrive', 'LifeSupport', 'PowerDistributor', 'Radar', 'FuelTank'];
const HARDPOINT_SIZES = ['Tiny', 'Small', 'Medium', 'Large', 'Huge'];
const NAMED_INTERNALS: Record<string, string> = { Military: 'Military', Cargo: 'Cargo', Limpets: 'LimpetController', Fighter: 'FighterBay' };

export interface LoadoutModule {
  Slot: string;
  Item: string;
  On: boolean;
  Priority: number;
  Engineering?: { BlueprintName: string; Level: number; Quality: number; ExperimentalEffect?: string };
}

export interface Loadout {
  event: 'Loadout';
  Ship: string;
  ShipName: string;
  ShipIdent: string;
  Modules: LoadoutModule[];
}

export type Slef = { header: { appName: string; appVersion: string; appURL: string }; data: Loadout }[];

let fdNames: Map<string, string> | undefined;
/** Frontier ship name (e.g. Krait_MkII) for a ship id. */
function fdShipName(shipId: string): string {
  fdNames ??= new Map(Object.entries(CompanionApiUtils.SHIP_FD_NAME_TO_CORIOLIS_NAME).map(([fd, id]) => [id, fd]));
  return fdNames.get(shipId) ?? shipId;
}

function engineering(m: any): LoadoutModule['Engineering'] | undefined {
  const bp = m?.blueprint;
  if (!bp || !bp.fdname) return undefined;
  const percent = BlueprintFunctions.getPercent(m);
  const out: NonNullable<LoadoutModule['Engineering']> = {
    BlueprintName: bp.fdname,
    Level: Number(bp.grade) || 1,
    Quality: typeof percent === 'number' && Number.isFinite(percent) ? Math.round(percent * 100) / 10000 : 1,
  };
  if (bp.special?.edname) out.ExperimentalEffect = bp.special.edname;
  return out;
}

function entry(slot: string, m: any, enabled: boolean, priority: number): LoadoutModule {
  const e: LoadoutModule = { Slot: slot, Item: String(m.symbol).toLowerCase(), On: enabled !== false, Priority: priority || 0 };
  const eng = engineering(m);
  if (eng) e.Engineering = eng;
  return e;
}

/** Frontier slot names for a hull's hardpoints and internals, in Coriolis slot order. */
export function journalSlotNames(shipId: string): { hardpoints: string[]; internal: (string | null)[] } {
  const template = shipData(shipId);
  const counters: Record<number, number> = {};
  const hardpoints = template.slots.hardpoints.map((def: any) => {
    const cls = typeof def === 'object' ? def.class : def;
    const prefix = typeof def === 'object' && def.name ? def.name : '';
    counters[cls] = (counters[cls] ?? 0) + 1;
    if (shipId === 'type_8_transport' && cls === 1 && counters[cls] === 3) counters[cls]++;
    return `${HARDPOINT_SIZES[cls]}${prefix}Hardpoint${counters[cls]}`;
  });
  let n = shipId === 'type_9_heavy' ? 0 : 1;
  const named: Record<string, number> = {};
  const internal = template.slots.internal.map((def: any) => {
    const name = typeof def === 'object' ? def.name : null;
    if (name === 'PlanetaryApproachSuite') return 'PlanetaryApproachSuite';
    if (name && NAMED_INTERNALS[name]) {
      named[name] = (named[name] ?? 0) + 1;
      return `${NAMED_INTERNALS[name]}0${named[name]}`;
    }
    // The game skips slot numbers on some hulls.
    if ((n === 11 && shipId === 'anaconda') || (n === 9 && (shipId === 'type_9_heavy' || shipId === 'type_10_defender'))) n += 2;
    else if (n === 4 && shipId === 'vulture') n++;
    const size = typeof def === 'object' ? def.class : def;
    return `Slot${String(n++).padStart(2, '0')}_Size${size}`;
  });
  return { hardpoints, internal };
}

function bulkheadItem(ship: Ship): string {
  const all = shipData(ship.id).bulkheads;
  const index = Math.max(0, all.findIndex((b: any) => b.id === ship.bulkheads.m?.id));
  const suffixes = all.length > 5
    ? ['grade1_default', 'grade1', 'grade2', 'grade3', 'mirrored', 'reactive']
    : ['grade1', 'grade2', 'grade3', 'mirrored', 'reactive'];
  return `${fdShipName(ship.id).toLowerCase()}_armour_${suffixes[index] ?? 'grade1'}`;
}

/** The build as a Frontier journal Loadout event. */
export function toLoadout(ship: Ship, name = ''): Loadout {
  const names = journalSlotNames(ship.id);
  const modules: LoadoutModule[] = [];
  const bh: LoadoutModule = { Slot: 'Armour', Item: bulkheadItem(ship), On: true, Priority: 1 };
  const bhEng = engineering(ship.bulkheads.m);
  if (bhEng) bh.Engineering = bhEng;
  modules.push(bh);
  modules.push({ Slot: 'CargoHatch', Item: 'modularcargobaydoor', On: ship.cargoHatch.enabled !== false, Priority: ship.cargoHatch.priority || 0 });
  ship.standard.forEach((slot: any, i: number) => {
    if (slot.m?.symbol && STANDARD_SLOTS[i]) modules.push(entry(STANDARD_SLOTS[i], slot.m, slot.enabled, slot.priority));
  });
  ship.hardpoints.forEach((slot: any, i: number) => {
    if (slot.m?.symbol) modules.push(entry(names.hardpoints[i], slot.m, slot.enabled, slot.priority));
  });
  ship.internal.forEach((slot: any, i: number) => {
    // Planetary approach suites are left out: they carry no stats and not every tool accepts them.
    if (!slot.m?.symbol || slot.m.grp === 'pas' || names.internal[i] === 'PlanetaryApproachSuite') return;
    modules.push(entry(names.internal[i]!, slot.m, slot.enabled, slot.priority));
  });
  return { event: 'Loadout', Ship: fdShipName(ship.id).toLowerCase(), ShipName: name, ShipIdent: '', Modules: modules };
}

/** The build in EDCD's Ship Loadout Exchange Format. */
export function toSlef(ship: Ship, name = ''): Slef {
  return [{ header: { appName: 'ED Corai', appVersion: BUILD.version, appURL: coriolisUrl(ship, name || undefined) }, data: toLoadout(ship, name) }];
}

const b64url = (buf: Buffer) => buf.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

/** Opens the build in EDSY through its import endpoint. */
export function edsyUrl(ship: Ship, name = ''): string {
  return `https://edsy.org/import?import=${b64url(deflateSync(JSON.stringify(toSlef(ship, name))))}`;
}

// ---------------------------------------------------------------- import

function inflateAny(b64: string): string {
  const buf = Buffer.from(b64.replace(/-/g, '+').replace(/_/g, '/'), 'base64');
  for (const f of [unzipSync, inflateSync, gunzipSync]) {
    try { return f(buf).toString('utf8'); } catch { /* try the next container */ }
  }
  throw new Error('Could not decompress the build data in the link.');
}

/** Coriolis attribute(s) a journal modifier label maps to (via the data's modifierActions table). */
function labelAttributes(label: string): string[] {
  const wanted = ('outfittingfieldtype_' + label).toLowerCase();
  const key = Object.keys(Modifications.modifierActions).find((k) => k.toLowerCase() === wanted);
  return key ? Object.keys(Modifications.modifierActions[key]) : [];
}

/** The imported module in the journal's slot (several slots often hold the same item). */
function findImported(ship: Ship, mod: any): any {
  const slot = String(mod.Slot).toLowerCase();
  if (slot === 'armour') return ship.bulkheads.m;
  const std = STANDARD_SLOTS.findIndex((n) => n.toLowerCase() === slot);
  if (std >= 0) return ship.standard[std]?.m;
  const names = journalSlotNames(ship.id);
  const hp = names.hardpoints.findIndex((n) => n.toLowerCase() === slot);
  if (hp >= 0) return ship.hardpoints[hp]?.m;
  const int = names.internal.findIndex((n) => n?.toLowerCase() === slot);
  if (int >= 0) return ship.internal[int]?.m;
  return undefined;
}

/**
 * Compares every engineered value in the Loadout's `Modifiers` with what the imported module now
 * reports. Journal values are final in-game numbers, so any difference is an import defect.
 */
export function importDiscrepancies(ship: Ship, loadout: any): string[] {
  const out: string[] = [];
  for (const mod of loadout.Modules ?? []) {
    for (const mf of mod.Engineering?.Modifiers ?? []) {
      const m = findImported(ship, mod);
      if (!m) continue;
      for (const attr of labelAttributes(mf.Label)) {
        const got = m.get(attr);
        if (typeof mf.Value !== 'number' || typeof got !== 'number') continue;
        if (Math.abs(got - mf.Value) > 1e-3 * Math.max(1, Math.abs(mf.Value))) {
          out.push(`${mod.Slot} ${mf.Label}: game ${mf.Value}, imported ${Math.round(got * 1e4) / 1e4}`);
        }
      }
    }
  }
  return out;
}

/**
 * Coriolis' journal importer misreads several modifier units (resistances arrive 100x too small,
 * shield-booster strength and some fire rates are wrong, some blueprints are dropped). The journal
 * carries the game's final values, so set each engineered attribute to exactly that value.
 */
function applyJournalModifiers(ship: Ship, loadout: any): void {
  for (const mod of loadout.Modules ?? []) {
    const eng = mod.Engineering;
    if (!eng?.Modifiers?.length) continue;
    const m = findImported(ship, mod);
    if (!m) continue;
    if (!m.blueprint?.fdname && eng.BlueprintName) {
      const bp = BlueprintFunctions.getBlueprint(eng.BlueprintName, m);
      if (bp?.fdname) {
        bp.grade = Number(eng.Level) || bp.grade;
        m.blueprint = bp;
      }
    }
    if (eng.ExperimentalEffect && m.blueprint && !m.blueprint.special) {
      const special = Modifications.specials[eng.ExperimentalEffect];
      if (special) m.blueprint.special = special;
    }
    for (const mf of eng.Modifiers) {
      if (typeof mf.Value !== 'number') continue;
      for (const attr of labelAttributes(mf.Label)) {
        if (attr === 'rof' && !m.get('burst') && mf.Value > 0) {
          // Coriolis' set('rof') computes 0 x Infinity for weapons without bursts. Its stored rof
          // modifier x acts as rate = base / (1 + x), so x = base / target - 1.
          const base = m.get('rof', false);
          if (base) m.setModValue('rof', (base / mf.Value - 1) * 10000, true);
          continue;
        }
        // The journal value already includes any experimental effect.
        m.set(attr, mf.Value, true);
      }
    }
  }
}

function fromLoadout(loadout: any): Ship {
  if (!loadout || !Array.isArray(loadout.Modules)) throw new Error('Not a Loadout: missing "Modules".');
  let ship: Ship;
  try {
    ship = JournalUtils.shipFromLoadoutJSON(loadout);
  } catch (e) {
    throw new Error(typeof e === 'string' ? e : (e as Error).message);
  }
  applyJournalModifiers(ship, loadout);
  recalcAll(ship);
  ship.importIssues = importDiscrepancies(ship, loadout);
  return ship;
}

function fromJsonText(text: string): Ship {
  const trimmed = text.trim();
  try {
    const json = JSON.parse(trimmed);
    if (Array.isArray(json)) {
      const first = json.find((x) => x?.data?.Modules);
      if (!first) throw new Error('SLEF contains no build.');
      return fromLoadout(first.data);
    }
    return fromLoadout(json.data?.Modules ? json.data : json);
  } catch (e) {
    // A journal file: one JSON event per line; take the last Loadout.
    const loadouts = trimmed.split('\n').map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter((x) => x?.event === 'Loadout');
    if (loadouts.length) return fromLoadout(loadouts[loadouts.length - 1]);
    throw e instanceof SyntaxError ? new Error('Not a recognised build: expected a Coriolis or EDSY link, SLEF, or a journal Loadout.') : e;
  }
}

/**
 * Reads a build from anything a player is likely to paste: a Coriolis outfit or import link,
 * an EDSY import link, SLEF JSON, a journal Loadout event, or journal lines.
 */
export function parseBuild(input: string, shipHint?: string): Ship {
  const text = input.trim();
  // JSON first: a SLEF header itself contains a Coriolis link (appURL).
  if (/^[[{]/.test(text)) return fromJsonText(text);
  if (/coriolis\.io\/outfit\//i.test(text) || (!text.includes('{') && !/^https?:/i.test(text) && shipHint)) {
    return shipFromCoriolis(text, shipHint);
  }
  const coriolisImport = text.match(/coriolis\.io\/import\/([^?#\s]+)/i);
  if (coriolisImport) return fromJsonText(inflateAny(decodeURIComponent(coriolisImport[1])));
  if (/edsy\.org|edshipyard/i.test(text)) {
    if (/#\/L=/.test(text)) {
      throw new Error('EDSY short links (#/L=) cannot be read. In EDSY use Export → SLEF and paste that text instead.');
    }
    const data = text.match(/#\/I=([^#\s]+)/)?.[1] ?? new URL(text).searchParams.get('import');
    if (!data) throw new Error('This EDSY link carries no build data.');
    const decoded = decodeURIComponent(data);
    return fromJsonText(/^\s*[[{]/.test(decoded) ? decoded : inflateAny(decoded));
  }
  return fromJsonText(text);
}
