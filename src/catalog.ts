// Which modules may legally go in which slot, and which engineering can be applied to them.
import { Modules, Modifications, ModuleGroupToName, STANDARD_KEYS, shipData, type EngSpec, type Ship, type SlotRef } from './engine.js';

/** Everything is allowed by default; each flag set to false excludes that kind of module. */
export interface CatalogFilters {
  allowPowerplay?: boolean;        // modules that require Powerplay pledging (default true)
  allowPreEngineered?: boolean;    // pre-engineered modules sold at tech brokers / outfitting (default true)
  allowCommunityGoal?: boolean;    // community-goal reward modules (default true)
  allowGuardian?: boolean;         // Guardian tech-broker modules (default true)
  allowExperimental?: boolean;     // AX / Guardian "experimental" weapons (default true)
  excludeGroups?: string[];        // never use these module groups
  excludeModules?: string[];       // never use these module ids
}

// Only one module of each family may be fitted (in-game limits; Coriolis enforces the same list).
// One shield of any type, and one of each: refinery, scoop, hangar, FSD booster, docking computer,
// weapon stabiliser, multi-limpet controller, SRV bay, supercruise assist, interdictor, surface
// scanner, and each scanner utility (cargo, wake, kill-warrant, xeno, pulse wave).
const UNIQUE_FAMILIES: string[][] = [
  ['sg', 'bsg', 'psg'], ['rf'], ['fs'], ['fh'], ['gfsb'], ['dc'], ['ews'], ['mlc'], ['pv'], ['sua'],
  ['fi'], ['ss'], ['cs'], ['ws'], ['kw'], ['xs'], ['pwa'],
];
export const MAX_EXPERIMENTAL_WEAPONS = 4;
const GUARDIAN_GROUPS = new Set(['gpc', 'ggc', 'gsc', 'gsrp', 'gfsb', 'ghrp', 'gmrp', 'gpp', 'gpd']);
const MINING_HARDPOINT_GROUPS = new Set(['ml', 'scl', 'sdm', 'abl', 'mvr', 'pwa']);

export function uniqueFamily(grp: string): string[] | undefined {
  return UNIQUE_FAMILIES.find((f) => f.includes(grp));
}

export type GroupCategory = 'standard' | 'hardpoint' | 'utility' | 'internal';

export interface GroupInfo { group: string; name: string; category: GroupCategory; count: number }

let groupCache: GroupInfo[] | undefined;
export function listGroups(): GroupInfo[] {
  if (groupCache) return groupCache;
  const out: GroupInfo[] = [];
  for (const g of Object.keys(Modules.standard)) out.push({ group: g, name: ModuleGroupToName[g] ?? g, category: 'standard', count: Modules.standard[g].length });
  for (const g of Object.keys(Modules.hardpoints)) {
    const util = Modules.hardpoints[g].every((m) => m.class === 0);
    out.push({ group: g, name: ModuleGroupToName[g] ?? Modules.hardpoints[g][0]?.ukName ?? g, category: util ? 'utility' : 'hardpoint', count: Modules.hardpoints[g].length });
  }
  for (const g of Object.keys(Modules.internal)) out.push({ group: g, name: ModuleGroupToName[g] ?? Modules.internal[g][0]?.ukName ?? g, category: 'internal', count: Modules.internal[g].length });
  groupCache = out;
  return out;
}

export function groupCategory(grp: string): GroupCategory | undefined {
  return listGroups().find((g) => g.group === grp)?.category;
}

/** Groups whose modules deal damage (used to decide which groups matter for DPS metrics). */
export function weaponGroups(): string[] {
  return Object.keys(Modules.hardpoints).filter((g) => Modules.hardpoints[g].some((m) => m.class > 0 && m.damage));
}

/** "Unrecognised …" import placeholders (rating Z, Int_Missing_* / Hpt_Missing_*): zero mass, not real modules. */
function isPlaceholder(m: any): boolean {
  if ((m.id === '0z' || m.id === '1z') && !m.name && !m.ukName) return true;
  // Coriolis share codes assume two-character ids; a module without one cannot be linked.
  if (String(m.id).length !== 2) return true;
  if (m.rating === 'Z' || /_Missing_/.test(m.symbol ?? '') || m.grp === 'mh' || m.grp === 'mm') return true;
  const name = String(m.name ?? '').toLowerCase();
  return name.includes('error') || name.includes('placeholder');
}

function passesFilters(m: any, f: CatalogFilters): boolean {
  if (isPlaceholder(m)) return false;
  if (m.pp && f.allowPowerplay === false) return false;
  if (m.preEngineered) {
    if (f.allowPreEngineered === false) return false;
    if (m.preEngineered.availability === 'CG' && f.allowCommunityGoal === false) return false;
  }
  if (isGuardian(m) && f.allowGuardian === false) return false;
  // Pre-engineered modules whose blueprints the data cannot resolve would be applied with no effect.
  if (m.preEngineered?.blueprints?.some((b: string) => !blueprintExists(b.trim()))) return false;
  if (m.experimental && f.allowExperimental === false) return false;
  if (f.excludeGroups?.includes(m.grp)) return false;
  if (f.excludeModules?.includes(m.id)) return false;
  return true;
}

/** Mirrors Coriolis' canMount() plus its restricted-slot rules. */
/** Ship-restricted modules list one ship id or several. */
function allowedOnShip(m: any, shipId: string): boolean {
  if (m.ship === undefined || m.ship === null) return true;
  return (Array.isArray(m.ship) ? m.ship : [m.ship]).includes(shipId);
}

/**
 * Core-module rules from Coriolis' outfitting list (ModuleSet): life support and sensors must
 * match the slot class, SCO frame shift drives must match it too, thrusters must be at least the
 * smallest class able to move the hull, and modules with a max mass must carry hull + 6.5 t.
 */
function fitsStandard(ship: Ship, ref: SlotRef, m: any): boolean {
  const grp = STANDARD_KEYS[ref.index];
  const mass = shipData(ship.id).properties.hullMass + 6.5;
  if (m.class > ref.maxClass) return false;
  if (grp === 'ls' || grp === 's') return m.class === ref.maxClass && (grp === 'ls' || m.maxmass === undefined || mass <= m.maxmass);
  if (grp === 'fsd' && String(m.name ?? '').includes('(SCO)')) return m.class === ref.maxClass;
  if (grp === 't') {
    const minClass = Modules.standard.t.reduce((c: number, t: any) => (t.maxmass >= mass && t.class < c ? t.class : c), ref.maxClass);
    return m.class >= minClass;
  }
  return m.maxmass === undefined || mass <= m.maxmass;
}

export function fitsSlot(ship: Ship, ref: SlotRef, m: any): boolean {
  if (!allowedOnShip(m, ship.id)) return false;
  if (ref.kind === 'standard') return fitsStandard(ship, ref, m);
  const slotName = (ref.name ?? '').toLowerCase();
  // ModuleSet lists only modules whose max mass (shield generators, ...) carries hull + 6.5 t.
  if (m.maxmass !== undefined && shipData(ship.id).properties.hullMass + 6.5 > m.maxmass) return false;
  if (ref.kind === 'internal') {
    if (m.class > ref.maxClass) return false;
    if (ref.eligible && !ref.eligible[m.grp]) return false;
    if ((m.id === '5L' || m.id === '5M' || m.grp === 'crl') && slotName !== 'cargo') return false;
    if ((m.id === '4C' || m.id === '4F' || m.grp === 'pas') && slotName !== 'planetaryapproachsuite') return false;
    if (m.id === '7W' && slotName !== 'limpets') return false;
    if (m.grp === 'pcq' && !ship.luxuryCabins) return false;
    if (m.grp === 'fh' && !ship.fighterHangars) return false;
    if (m.grp === 'fhmkii' && !ship.vesselHangarsMkII) return false;
    if (m.grp === 'lpvmkii' && !ship.vehicleHangarsMkII) return false;
    return true;
  }
  if (ref.kind === 'hardpoint') {
    if (m.class === 0 || m.class > ref.maxClass) return false;
    if (ref.eligible && !ref.eligible[m.grp]) return false;
    // The Anaconda Mk II takes the Mining Volley Repeater only on its two flagged hardpoints.
    if (m.grp === 'mvr' && ship.id === 'anaconda2' && !ref.slot.mvr) return false;
    if (slotName === 'mining' && !MINING_HARDPOINT_GROUPS.has(m.grp)) return false;
    return true;
  }
  if (ref.kind === 'utility') {
    if (m.class !== 0) return false;
    if (ref.eligible && !ref.eligible[m.grp]) return false;
    return true;
  }
  return m.class <= ref.maxClass;
}

/**
 * All module definitions that may go into `ref` on `ship`, restricted to `groups`
 * (undefined = every group). Standard slots and bulkheads ignore `groups`.
 */
export function slotCandidates(ship: Ship, ref: SlotRef, filters: CatalogFilters, groups?: Set<string>): any[] {
  if (ref.kind === 'bulkheads') {
    return shipData(ship.id).bulkheads.map((b: any, i: number) => ({ ...b, grp: 'bh', bulkheadIndex: i }));
  }
  if (ref.kind === 'standard') {
    const grp = STANDARD_KEYS[ref.index];
    return Modules.standard[grp].filter((m) => fitsSlot(ship, ref, m) && passesFilters(m, filters));
  }
  const source = ref.kind === 'internal' ? Modules.internal : Modules.hardpoints;
  const out: any[] = [];
  for (const grp of Object.keys(source)) {
    if (groups && !groups.has(grp)) continue;
    for (const m of source[grp]) {
      if (fitsSlot(ship, ref, m) && passesFilters(m, filters)) out.push(m);
    }
  }
  return out;
}

export function allCandidateGroups(ship: Ship, ref: SlotRef, filters: CatalogFilters): Set<string> {
  return new Set(slotCandidates(ship, ref, filters).map((m) => m.grp));
}

/** Highest grade a module group can take for a blueprint. */
export function maxBlueprintGrade(grp: string, bpKey: string): number {
  const grades = Modifications.modules[grp]?.blueprints?.[bpKey]?.grades ?? Modifications.blueprints[bpKey]?.grades ?? {};
  const nums = Object.keys(grades).map(Number).filter((n) => !Number.isNaN(n));
  return nums.length ? Math.max(...nums) : 0;
}

/** Blueprints no engineer offers (e.g. CargoRack_IncreasedCapacity, CG pre-engineered only) are not obtainable. */
function hasEngineers(grp: string, bpKey: string, grade: number): boolean {
  const engineers = Modifications.modules[grp]?.blueprints?.[bpKey]?.grades?.[grade]?.engineers;
  return !Array.isArray(engineers) || engineers.length > 0;
}

/**
 * Engineering options worth considering for a module: every blueprint at its max grade,
 * each with and without each compatible experimental effect.
 */
/** Whether Coriolis' blueprint lookup (`getBlueprint`) resolves a name: case-insensitive substring. */
export function blueprintExists(name: string): boolean {
  const needle = name.toLowerCase().replace(/(outfittingfieldtype_|persecond)/g, '');
  return Object.keys(Modifications.blueprints).some((k) => k.toLowerCase().includes(needle));
}

/**
 * Coriolis' rule for whether a module can take engineering: its `engineering` flag is not false,
 * its group has modifiable attributes, and it is not Guardian technology (Guardian modules are
 * flagged per group in the data, but the hybrid power plant and distributor live in the normal
 * `pp`/`pd` groups and are excluded by name/symbol in the Coriolis UI).
 */
export function canEngineer(m: any): boolean {
  if (!m || m.engineering === false || m.engineering === 'False') return false;
  if (isGuardian(m)) return false;
  const mods = Modifications.modules[m.grp];
  return Boolean(mods && (mods.modifications?.length || Object.keys(mods.blueprints ?? {}).length));
}

export function isGuardian(m: any): boolean {
  return GUARDIAN_GROUPS.has(m.grp) || /guardian/i.test(String(m.symbol ?? '')) || /^Guardian /.test(String(m.name ?? ''));
}

export function engineeringOptions(m: any): EngSpec[] {
  if (!m || !canEngineer(m)) return [];
  const grp = m.grp;
  const mods = Modifications.modules[grp];
  if (!mods) return [];
  const specials = (mods.specials ?? []).filter((s) => Modifications.specials[s]);
  if (m.preEngineered) {
    if (!m.preEngineered.canApplyExperimental) return [];
    return specials.map((s) => ({ blueprint: m.preEngineered.blueprints[0], grade: 5, special: s }));
  }
  const out: EngSpec[] = [];
  for (const bp of Object.keys(mods.blueprints ?? {})) {
    if (!Modifications.blueprints[bp]) continue;
    const grade = maxBlueprintGrade(grp, bp);
    if (!grade || !hasEngineers(grp, bp, grade)) continue;
    out.push({ blueprint: bp, grade, special: null });
    for (const s of specials) out.push({ blueprint: bp, grade, special: s });
  }
  return out;
}

export function describeEngineering(grp: string) {
  const mods = Modifications.modules[grp];
  if (!mods) return { blueprints: [], specials: [] };
  return {
    blueprints: Object.keys(mods.blueprints ?? {}).filter((bp) => Modifications.blueprints[bp]).map((bp) => {
      const def = Modifications.blueprints[bp];
      const grade = maxBlueprintGrade(grp, bp);
      return { key: bp, name: def.name ?? bp, maxGrade: grade, features: def.grades?.[grade]?.features };
    }),
    specials: (mods.specials ?? []).filter((s) => Modifications.specials[s]).map((s) => ({ key: s, name: Modifications.specials[s].name, description: Modifications.specials[s].description })),
  };
}
