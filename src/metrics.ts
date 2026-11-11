// Named, documented, numeric metrics computed from a ship by the Coriolis engine.
import { Calc, Modules, stockShip, type Ship } from './engine.js';
import { weaponGroups, groupCategory } from './catalog.js';
import { combatDeployedDraw } from './arrange.js';

export interface DamageProfile { kinetic?: number; thermal?: number; explosive?: number; absolute?: number }

/**
 * Named incoming-damage mixes. `even` is the neutral default. `pve` leans thermal (NPC lasers and
 * incendiary rounds) and is an assumption, not a measurement: no published test gives the real split.
 */
export const DAMAGE_PRESETS: Record<string, Required<DamageProfile>> = {
  even: { kinetic: 1, thermal: 1, explosive: 1, absolute: 0 },
  pve: { kinetic: 0.30, thermal: 0.60, explosive: 0.05, absolute: 0.05 },
};

export interface MetricContext {
  sys: number;              // SYS pips while fighting 0..4 (shield resistance and regen under fire)
  wep: number;              // WEP pips while fighting 0..4 (weapon capacitor recharge)
  recoverySys: number;      // SYS pips while recovering 0..4 (shield recharge / rebuild times)
  damage: Required<DamageProfile>; // incoming damage mix used for effective HP, normalised to sum 1
  opponent: Ship;           // reference target for DPS-vs-target metrics
  opponentSys: number;
  range: number;            // engagement range in metres
  incomingDps: number;      // raw incoming DPS for time-under-fire metrics
  /** Share of time shields regenerate while under fire (0 = regen pauses on every hit, 1 = continuous). Unmeasured. */
  regenUnderFire: number;
  /** Count the reserve tank in jump mass (Coriolis does; EDSY and the journal's MaxJumpRange do not). */
  reserveFuelInJumpMass: boolean;
  /** Share of time Thermal Vent beams are on target. */
  ventHitRate: number;
  /** Heat a hitting Thermal Vent beam removes, as a multiple of its own thermal load. Unmeasured; 1 is an assumption. */
  thermalVentFactor: number;
  /** Hull damage multiplier on a target hit by Corrosive Shell (all weapons, does not stack). Community figure 1.25. */
  corrosiveHullMultiplier: number;
  /** Armour piercing added to the Corrosive Shell weapon itself. Community figure 20. */
  corrosivePiercingBonus: number;
}

export interface MetricDef {
  name: string;
  unit: string;
  description: string;
  better: 'higher' | 'lower';
  /** Module groups whose presence can improve this metric (beyond standard slots and bulkheads). */
  groups: string[];
  compute(ship: Ship, ctx: MetricContext): number;
}

const SHIELD_GROUPS = ['sg', 'bsg', 'psg', 'sb', 'gsrp'];
const HULL_GROUPS = ['hr', 'ghrp', 'mahr'];
const CAP = 1e6; // stand-in for "infinite" so that comparisons stay numeric

export function makeContext(opts: {
  sys?: number; wep?: number; recoverySys?: number; damage?: DamageProfile; damagePreset?: keyof typeof DAMAGE_PRESETS; opponent?: Ship; opponentSys?: number;
  range?: number; incomingDps?: number; regenUnderFire?: number; reserveFuelInJumpMass?: boolean;
  ventHitRate?: number; thermalVentFactor?: number; corrosiveHullMultiplier?: number; corrosivePiercingBonus?: number;
} = {}): MetricContext {
  const d = { ...DAMAGE_PRESETS[opts.damagePreset ?? 'even'], ...(opts.damage ?? {}) };
  const sum = d.kinetic + d.thermal + d.explosive + d.absolute || 1;
  return {
    // Typical PvE pip use: fight on 2 SYS / 4 WEP, recover shields on 4 SYS.
    sys: opts.sys ?? 2,
    wep: opts.wep ?? 4,
    recoverySys: opts.recoverySys ?? 4,
    damage: { kinetic: d.kinetic / sum, thermal: d.thermal / sum, explosive: d.explosive / sum, absolute: d.absolute / sum },
    opponent: opts.opponent ?? defaultOpponent(),
    opponentSys: opts.opponentSys ?? 2,
    range: opts.range ?? 1000,
    incomingDps: opts.incomingDps ?? 200,
    regenUnderFire: Math.min(1, Math.max(0, opts.regenUnderFire ?? 0)),
    reserveFuelInJumpMass: opts.reserveFuelInJumpMass ?? false,
    ventHitRate: Math.min(1, Math.max(0, opts.ventHitRate ?? 1)),
    thermalVentFactor: Math.max(0, opts.thermalVentFactor ?? 1),
    corrosiveHullMultiplier: Math.max(1, opts.corrosiveHullMultiplier ?? 1.25),
    corrosivePiercingBonus: Math.max(0, opts.corrosivePiercingBonus ?? 20),
  };
}

let opponentCache: Ship | undefined;
function defaultOpponent(): Ship {
  return (opponentCache ??= stockShip('anaconda'));
}

function shieldMults(ship: Ship, ctx: MetricContext) {
  const s = Calc.shieldMetrics(ship, ctx.sys);
  if (!s.generator) return { raw: 0, cells: 0, mult: 1, recharge: CAP, recover: CAP };
  const d = ctx.damage;
  const mult = d.kinetic * s.kinetic.total + d.thermal * s.thermal.total + d.explosive * s.explosive.total + d.absolute * s.absolute.total;
  const fin = (x: number) => (Number.isFinite(x) ? x : CAP);
  return { raw: s.summary, cells: s.cells ?? 0, mult, recharge: fin(s.recharge), recover: fin(s.recover) };
}

/**
 * Shield regeneration the distributor can keep up indefinitely at the context SYS pips: each MJ
 * regenerated costs the generator's distributor draw from the SYS capacitor (Coriolis' model).
 */
function sustainedRegen(ship: Ship, ctx: MetricContext): number {
  const slot = ship.findInternalByGroup('sg');
  const pd = ship.standard[4].m;
  if (!slot?.m || !slot.enabled || !pd || !shieldMults(ship, ctx).raw) return 0;
  const regen = slot.m.getRegenerationRate() || 0;
  const draw = slot.m.getDistDraw() || 0;
  const sysRate = Calc.sysRechargeRate(pd, ctx.sys);
  return draw > 0 ? Math.min(regen, sysRate / draw) : regen;
}

/**
 * Seconds until shields break under the context's incoming raw DPS. Regeneration counts only for
 * the context's regenUnderFire share of the time; cell banks add their MJ when `withCells`.
 */
function shieldTimeUnderFire(ship: Ship, ctx: MetricContext, withCells = false): number {
  const s = shieldMults(ship, ctx);
  if (!s.raw) return 0;
  const pool = s.raw + (withCells ? s.cells : 0);
  const net = ctx.incomingDps * s.mult - sustainedRegen(ship, ctx) * ctx.regenUnderFire;
  return net > 0 ? Math.min(pool / net, CAP) : CAP;
}

/** Effective shield HP against the single damage type the shield resists least (K/T/E). */
function shieldEhpWorstType(ship: Ship, ctx: MetricContext): number {
  const s = Calc.shieldMetrics(ship, ctx.sys);
  if (!s.generator) return 0;
  const worst = Math.max(s.kinetic.total, s.thermal.total, s.explosive.total);
  return worst > 0 ? s.summary / worst : CAP;
}

/** DPS-weighted share of hull damage that survives the opponent's hardness (1 = every weapon pierces fully). */
function pierceEfficiency(ship: Ship, ctx: MetricContext): number {
  const ws = weapons(ship);
  const total = ws.reduce((a, m) => a + m.getSDps(), 0);
  if (!total) return 0;
  const h = ctx.opponent.hardness || 1;
  return ws.reduce((a, m) => a + m.getSDps() * Math.min(1, piercing(m, ctx) / h), 0) / total;
}

function isCorrosive(m: any): boolean {
  return m?.blueprint?.special?.edname === 'special_corrosive_shell';
}

/** Piercing of a weapon including the Corrosive Shell bonus on the weapon that carries it. */
function piercing(m: any, ctx: MetricContext): number {
  return (m.getPiercing() || 0) + (isCorrosive(m) ? ctx.corrosivePiercingBonus : 0);
}

/**
 * True when a powered Corrosive Shell weapon reaches the target at the context range. Its debuff
 * lasts about 5 s and refreshes on every hit, so continuous fire keeps it up; it applies to hull
 * damage only and does not stack across weapons.
 */
function corrosiveActive(ship: Ship, ctx: MetricContext): boolean {
  return ship.hardpoints.some((slot: any) => slot.enabled && isCorrosive(slot.m) && falloffFactor(slot.m, ctx.range) > 0);
}

/** Seconds to strip the reference opponent's shields then hull at the context range (no opponent regen). */
function timeToKill(ship: Ship, ctx: MetricContext): number {
  const { shields, hull } = vsTarget(ship, ctx);
  const opp = Calc.shieldMetrics(ctx.opponent, ctx.opponentSys);
  const oppShield = opp.generator ? opp.summary : 0;
  const oppHull = Calc.armourMetrics(ctx.opponent).total;
  if ((oppShield > 0 && !(shields > 0)) || !(hull > 0)) return CAP;
  return Math.min((oppShield > 0 ? oppShield / shields : 0) + oppHull / hull, CAP);
}

/** Seconds between boosts at 4 ENG pips (distributor refill or the hull's own cooldown, whichever is longer). */
function boostInterval(ship: Ship): number {
  const pd = ship.standard[4].m;
  if (!pd || !ship.canBoost(0, ship.fuelCapacity)) return CAP;
  const rate = pd.getEnginesRechargeRate() || 0;
  return rate > 0 ? Math.max(ship.boostInt || 0, ship.boostEnergy / rate) : CAP;
}

function hullMults(ship: Ship, ctx: MetricContext) {
  const a = Calc.armourMetrics(ship);
  const d = ctx.damage;
  const mult = d.kinetic * a.kinetic.total + d.thermal * a.thermal.total + d.explosive * a.explosive.total + d.absolute * a.absolute.total;
  return { raw: a.total, mult, moduleProtection: a.moduleprotection };
}

function shieldEhp(ship: Ship, ctx: MetricContext, withCells = false) {
  const s = shieldMults(ship, ctx);
  return (s.raw + (withCells ? s.cells : 0)) / s.mult;
}

function hullEhp(ship: Ship, ctx: MetricContext) {
  const h = hullMults(ship, ctx);
  return h.raw / h.mult;
}

function vsTarget(ship: Ship, ctx: MetricContext) {
  const r = Calc.sustainedDps(ship, ctx.opponent, ctx.opponentSys, ctx.range);
  const sum = (o: any) => o.absolute + o.explosive + o.kinetic + o.thermal;
  let hull = sum(r.armoursdps);
  if (corrosiveActive(ship, ctx)) {
    // Coriolis models only the ammo penalty. Add the piercing bonus on the carrying weapon, then
    // the debuff on all hull damage.
    const h = ctx.opponent.hardness || 1;
    for (const slot of ship.hardpoints as any[]) {
      const m = slot.m;
      if (!slot.enabled || !isCorrosive(m)) continue;
      const base = Math.min(1, (m.getPiercing() || 0) / h);
      if (base <= 0) continue;
      const w = Calc._weaponSustainedDps(m, ctx.opponent, Calc.shieldMetrics(ctx.opponent, ctx.opponentSys), Calc.armourMetrics(ctx.opponent), ctx.range);
      hull += w.damage.armour.total * (Math.min(1, piercing(m, ctx) / h) / base - 1);
    }
    hull *= ctx.corrosiveHullMultiplier;
  }
  return { shields: sum(r.shieldsdps), hull };
}

function modulesOf(ship: Ship): any[] {
  return [...ship.internal, ...ship.hardpoints].map((s: any) => s.m).filter(Boolean);
}

function experimentalSdps(ship: Ship): number {
  let total = 0;
  for (const slot of ship.hardpoints) {
    const m = slot.m;
    if (!m || !slot.enabled || !m.experimental || !m.getDps()) continue;
    const dps = m.getDps();
    total += m.getClip() ? (m.getClip() * dps / m.getRoF()) / ((m.getClip() / m.getRoF()) + m.getReload()) : dps;
  }
  return total;
}

/**
 * Single-jump range with the fuel actually on board. Coriolis' own unladenRange/ladenRange burn
 * the FSD's max fuel per jump even when the tank is smaller, which an optimiser would exploit.
 */
function jump(ship: Ship, cargo: number, fuelOnBoard: number, ctx: MetricContext): number {
  const fsd = ship.standard[2].m;
  if (!fsd) return 0;
  const burn = Math.min(fsd.getMaxFuelPerJump(), fuelOnBoard);
  // ship.unladenMass already includes a full tank; start from the dry mass so fuel counts once.
  // Calc.jumpRange adds the reserve tank itself; take it back out unless the context wants it.
  const reserve = ctx.reserveFuelInJumpMass ? 0 : ship.reserveFuelCapacity || 0;
  return Calc.jumpRange(ship.dryMass + cargo + fuelOnBoard - reserve, fsd, burn, ship);
}

function totalRange(ship: Ship, cargo: number, ctx: MetricContext): number {
  const fsd = ship.standard[2].m;
  if (!fsd) return 0;
  const reserve = ctx.reserveFuelInJumpMass ? 0 : ship.reserveFuelCapacity || 0;
  return Calc.totalJumpRange(ship.dryMass + ship.fuelCapacity + cargo - reserve, fsd, ship.fuelCapacity, ship);
}

function sumAttr(ship: Ship, grp: string, attr: string): number {
  return modulesOf(ship).filter((m) => m.grp === grp).reduce((a, m) => a + (m.get(attr) ?? 0), 0);
}

/**
 * Seconds of continuous fire until the first ammunition-limited weapon has used its whole
 * magazine and reserve (CAP when no weapon is ammo-limited). Plasma Slug draws fuel instead of
 * ammunition, which Coriolis does not model, so it counts as unlimited here.
 */
function ammoEndurance(ship: Ship): number {
  let min = CAP;
  for (const slot of ship.hardpoints) {
    const m = slot.m;
    if (!m || !slot.enabled || !slot.maxClass || !m.getDps()) continue;
    if (String(m.blueprint?.special?.edname ?? '').startsWith('special_plasma_slug')) continue;
    const rounds = (m.getAmmo() || 0) + (m.getClip() || 0);
    const shotsPerSecond = m.getRoF() * m.getSustainedFactor();
    if (!rounds || !(shotsPerSecond > 0)) continue;
    min = Math.min(min, rounds / shotsPerSecond);
  }
  return min;
}

function isThermalVent(m: any): boolean {
  return m?.blueprint?.special?.edname === 'special_thermal_vent';
}

/** Damage falloff factor at the context range (1 inside falloff, 0 at maximum range). */
function falloffFactor(m: any, range: number): number {
  const falloff = m.getFalloff();
  const max = m.getRange();
  if (!falloff || !max || range <= falloff) return 1;
  return Math.max(0, 1 - (range - falloff) / Math.max(max - falloff, 1e-9));
}

/**
 * Heat per second in a fight. Thermal Vent beams generate their heat only while missing; while on
 * target they remove thermalVentFactor x their thermal load, reduced by falloff.
 */
function heatBudget(ship: Ship, ctx: MetricContext) {
  const plant = combatDeployedDraw(ship) * (ship.standard[0].m?.getThermalEfficiency() ?? 0);
  let weapons = 0, ventGeneration = 0, ventRemoval = 0;
  for (const m of weapons_(ship)) {
    const hps = m.getHps() || 0;
    if (isThermalVent(m)) {
      ventGeneration += (1 - ctx.ventHitRate) * hps;
      ventRemoval += ctx.ventHitRate * ctx.thermalVentFactor * hps * falloffFactor(m, ctx.range);
    } else {
      weapons += hps;
    }
  }
  const generated = plant + weapons + ventGeneration;
  // thermalVentFactor at which venting cancels everything else that heats the ship.
  const unitRemoval = ctx.ventHitRate * weapons_(ship).filter(isThermalVent).reduce((a, m) => a + (m.getHps() || 0) * falloffFactor(m, ctx.range), 0);
  return { plant, generated, ventRemoval, net: generated - ventRemoval, breakEvenFactor: unitRemoval > 0 ? generated / unitRemoval : CAP };
}

function weapons_(ship: Ship): any[] {
  return weapons(ship);
}

/** Fitted, enabled weapons (hardpoints that deal damage). */
function weapons(ship: Ship): any[] {
  return ship.hardpoints.filter((s: any) => s.m && s.enabled && s.maxClass > 0 && s.m.getDps()).map((s: any) => s.m);
}

const WEAPONS = weaponGroups();
const EXPERIMENTAL_WEAPONS = WEAPONS.filter((g) => Modules.hardpoints[g].some((m) => m.experimental));

export const METRICS: MetricDef[] = [
  // --- travel ---
  { name: 'unladenRange', unit: 'Ly', better: 'higher', groups: ['gfsb'], description: 'Max single jump with no cargo and just enough fuel for that jump (the headline Coriolis/EDSY jump range).', compute: (s, c) => jump(s, 0, Math.min(s.fuelCapacity, s.standard[2].m?.getMaxFuelPerJump() ?? 0), c) },
  { name: 'fullTankRange', unit: 'Ly', better: 'higher', groups: ['gfsb'], description: 'Single jump with a full fuel tank and no cargo.', compute: (s, c) => jump(s, 0, s.fuelCapacity, c) },
  { name: 'ladenRange', unit: 'Ly', better: 'higher', groups: ['gfsb'], description: 'Single jump with full fuel tank and every cargo rack full.', compute: (s, c) => jump(s, s.cargoCapacity, s.fuelCapacity, c) },
  { name: 'totalRangeUnladen', unit: 'Ly', better: 'higher', groups: ['gfsb', 'ft'], description: 'Total distance on one tank of fuel (consecutive jumps) with no cargo.', compute: (s, c) => totalRange(s, 0, c) },
  { name: 'totalRangeLaden', unit: 'Ly', better: 'higher', groups: ['gfsb', 'ft'], description: 'Total distance on one tank of fuel with full cargo.', compute: (s, c) => totalRange(s, s.cargoCapacity, c) },
  { name: 'topSpeed', unit: 'm/s', better: 'higher', groups: [], description: 'Top speed (4 pips ENG, unladen, full fuel).', compute: (s) => s.topSpeed },
  { name: 'topBoost', unit: 'm/s', better: 'higher', groups: [], description: 'Boost speed (4 pips ENG, unladen, full fuel). 0 when the distributor cannot boost.', compute: (s) => (s.canBoost(0, s.fuelCapacity) ? s.topBoost : 0) },
  { name: 'pitch', unit: 'deg/s', better: 'higher', groups: [], description: 'Pitch rate (4 pips ENG).', compute: (s) => s.topPitch },
  { name: 'yaw', unit: 'deg/s', better: 'higher', groups: [], description: 'Yaw rate (4 pips ENG).', compute: (s) => s.topYaw },
  { name: 'roll', unit: 'deg/s', better: 'higher', groups: [], description: 'Roll rate (4 pips ENG).', compute: (s) => s.topRoll },
  // --- capacity / mass / money ---
  { name: 'cargoCapacity', unit: 't', better: 'higher', groups: ['cr', 'crl'], description: 'Total cargo rack capacity.', compute: (s) => s.cargoCapacity },
  { name: 'passengerCapacity', unit: 'seats', better: 'higher', groups: ['pce', 'pci', 'pcm', 'pcq'], description: 'Total passenger berths.', compute: (s) => s.passengerCapacity },
  { name: 'fuelCapacity', unit: 't', better: 'higher', groups: ['ft'], description: 'Main + internal fuel tank capacity.', compute: (s) => s.fuelCapacity },
  { name: 'dryMass', unit: 't', better: 'lower', groups: [], description: 'Hull and modules, without fuel or cargo.', compute: (s) => s.dryMass },
  { name: 'ladenMass', unit: 't', better: 'lower', groups: [], description: 'Hull + modules + full fuel + full cargo.', compute: (s) => s.ladenMass },
  { name: 'totalCost', unit: 'CR', better: 'lower', groups: [], description: 'Purchase price of hull and modules (no discounts, engineering not priced).', compute: (s) => s.totalCost },
  { name: 'powerMargin', unit: 'MW', better: 'higher', groups: [], description: 'Power plant output minus power draw with hardpoints deployed.', compute: (s) => s.powerAvailable - s.powerDeployed },
  { name: 'fuelScoopRate', unit: 'kg/s', better: 'higher', groups: ['fs'], description: 'Fuel scoop rate at a main-sequence star.', compute: (s) => sumAttr(s, 'fs', 'rate') },
  // --- mining ---
  { name: 'refineryBins', unit: 'bins', better: 'higher', groups: ['rf'], description: 'Refinery bins.', compute: (s) => sumAttr(s, 'rf', 'bins') },
  { name: 'collectorLimpets', unit: 'limpets', better: 'higher', groups: ['cc', 'mlc'], description: 'Simultaneously active collector limpets (collector + multi-limpet controllers).', compute: (s) => sumAttr(s, 'cc', 'maximum') + sumAttr(s, 'mlc', 'maximum') },
  // --- defence ---
  { name: 'shieldRaw', unit: 'MJ', better: 'higher', groups: SHIELD_GROUPS, description: 'Shield strength (generator + boosters + guardian SRPs), no resistances.', compute: (s, c) => shieldMults(s, c).raw },
  { name: 'shieldEHP', unit: 'MJ', better: 'higher', groups: SHIELD_GROUPS, description: 'Effective shield HP against the context damage mix at the context SYS pips (resistances incl. diminishing returns).', compute: (s, c) => shieldEhp(s, c) },
  { name: 'shieldEHPWithCells', unit: 'MJ', better: 'higher', groups: [...SHIELD_GROUPS, 'scb'], description: 'shieldEHP plus total shield cell bank capacity.', compute: (s, c) => shieldEhp(s, c, true) },
  { name: 'shieldRecharge', unit: 's', better: 'lower', groups: SHIELD_GROUPS, description: 'Seconds to recharge shields from 50% to 100% at the recovery SYS pips (context.recoverySys).', compute: (s, c) => shieldMults(s, { ...c, sys: c.recoverySys }).recharge },
  { name: 'shieldRecover', unit: 's', better: 'lower', groups: SHIELD_GROUPS, description: 'Seconds from shields broken back to 50% at the recovery SYS pips (includes 16 s delay).', compute: (s, c) => shieldMults(s, { ...c, sys: c.recoverySys }).recover },
  { name: 'shieldRegen', unit: 'MJ/s', better: 'higher', groups: SHIELD_GROUPS, description: 'Shield regeneration the distributor can sustain indefinitely at the fighting SYS pips.', compute: (s, c) => sustainedRegen(s, c) },
  { name: 'shieldSustainDps', unit: 'dps', better: 'higher', groups: SHIELD_GROUPS, description: 'Raw incoming DPS (context damage mix and SYS pips) the shields absorb indefinitely: sustained regeneration divided by the damage multiplier. Rewards high resistances, which multiply the value of every MJ regenerated.', compute: (s, c) => { const m = shieldMults(s, c); return m.raw ? sustainedRegen(s, c) / m.mult : 0; } },
  { name: 'shieldTimeUnderFire', unit: 's', better: 'higher', groups: SHIELD_GROUPS, description: 'Seconds until shields break under context.incomingDps raw damage (context mix and SYS pips); regeneration counts for the context.regenUnderFire share of the time (default 0). 1e6 = never.', compute: (s, c) => shieldTimeUnderFire(s, c) },
  { name: 'shieldTimeUnderFireWithCells', unit: 's', better: 'higher', groups: [...SHIELD_GROUPS, 'scb'], description: 'shieldTimeUnderFire with all shield cell bank MJ added to the pool (ignores spin-up and heat).', compute: (s, c) => shieldTimeUnderFire(s, c, true) },
  { name: 'shieldEHPWorstType', unit: 'MJ', better: 'higher', groups: SHIELD_GROUPS, description: 'Effective shield HP against whichever of kinetic/thermal/explosive the shield resists least, at context SYS pips. Exposes the stock -20% thermal hole that a mixed average hides.', compute: (s, c) => shieldEhpWorstType(s, c) },
  { name: 'hullRaw', unit: 'HP', better: 'higher', groups: HULL_GROUPS, description: 'Armour (bulkheads + hull reinforcements), no resistances.', compute: (s, c) => hullMults(s, c).raw },
  { name: 'hullEHP', unit: 'HP', better: 'higher', groups: HULL_GROUPS, description: 'Effective armour against the context damage mix.', compute: (s, c) => hullEhp(s, c) },
  { name: 'totalEHP', unit: 'HP', better: 'higher', groups: [...SHIELD_GROUPS, ...HULL_GROUPS], description: 'shieldEHP + hullEHP: total effective damage absorbed before destruction (no cells).', compute: (s, c) => shieldEhp(s, c) + hullEhp(s, c) },
  { name: 'totalEHPWithCells', unit: 'HP', better: 'higher', groups: [...SHIELD_GROUPS, ...HULL_GROUPS, 'scb'], description: 'totalEHP plus shield cell bank capacity.', compute: (s, c) => shieldEhp(s, c, true) + hullEhp(s, c) },
  { name: 'moduleArmour', unit: 'HP', better: 'higher', groups: ['mrp', 'gmrp'], description: 'Total integrity of module reinforcement packages: the damage they absorb before they are used up (protection share is moduleProtection).', compute: (s) => Calc.armourMetrics(s).modulearmour },
  { name: 'moduleProtection', unit: 'fraction', better: 'higher', groups: ['mrp', 'gmrp'], description: 'Share of module damage absorbed by module reinforcements.', compute: (s, c) => hullMults(s, c).moduleProtection },
  // --- offence ---
  { name: 'sustainedDps', unit: 'dps', better: 'higher', groups: WEAPONS, description: 'Raw sustained DPS of all weapons including reloads (no target resistances).', compute: (s) => s.totalSDps },
  { name: 'burstDps', unit: 'dps', better: 'higher', groups: WEAPONS, description: 'Raw burst DPS of all weapons (no reloads).', compute: (s) => s.totalDps },
  { name: 'dpsVsShields', unit: 'dps', better: 'higher', groups: WEAPONS, description: 'Sustained DPS against the reference opponent\'s shields at context range (resistances, falloff).', compute: (s, c) => vsTarget(s, c).shields },
  { name: 'dpsVsHull', unit: 'dps', better: 'higher', groups: WEAPONS, description: 'Sustained DPS against the reference opponent\'s hull at context range (resistances, hardness/piercing, falloff, Corrosive Shell debuff).', compute: (s, c) => vsTarget(s, c).hull },
  { name: 'pierceEfficiency', unit: 'fraction', better: 'higher', groups: WEAPONS, description: 'DPS-weighted share of hull damage that survives the reference opponent\'s armour hardness (piercing/hardness, capped at 1). Rewards Focused engineering and piercing-100 weapons against big hulls.', compute: (s, c) => pierceEfficiency(s, c) },
  { name: 'timeToKill', unit: 's', better: 'lower', groups: WEAPONS, description: 'Seconds to strip the reference opponent\'s shields (at its SYS pips) and then its hull at the context range; opponent regeneration ignored. 1e6 = cannot.', compute: (s, c) => timeToKill(s, c) },
  { name: 'boostInterval', unit: 's', better: 'lower', groups: [], description: 'Seconds between boosts at 4 ENG pips (distributor refill or the hull\'s boost cooldown, whichever is longer). 1e6 = cannot boost.', compute: (s) => boostInterval(s) },
  { name: 'weaponSustain', unit: 's', better: 'higher', groups: [], description: 'Seconds of continuous fire before the WEP capacitor is empty at context WEP pips (capped at 1e6 = sustainable).', compute: (s, c) => { const t = Calc.timeToDrainWep(s, c.wep); return Number.isFinite(t) ? t : CAP; } },
  { name: 'emptyHardpoints', unit: 'slots', better: 'lower', groups: [], description: 'Weapon hardpoints left empty. Constrain to 0 for a fully armed ship.', compute: (s) => s.hardpoints.filter((h: any) => h.maxClass > 0 && !h.m).length },
  { name: 'emptyUtilities', unit: 'slots', better: 'lower', groups: [], description: 'Utility mounts left empty.', compute: (s) => s.hardpoints.filter((h: any) => h.maxClass === 0 && !h.m).length },
  { name: 'powerAvailable', unit: 'MW', better: 'higher', groups: [], description: 'Power plant output.', compute: (s) => s.powerAvailable },
  { name: 'combatPowerDraw', unit: 'MW', better: 'lower', groups: [], description: 'Power drawn in a fight: every combat module with hardpoints deployed (non-combat modules shed by priority).', compute: (s) => combatDeployedDraw(s) },
  { name: 'powerPlantHeat', unit: 'heat/s', better: 'lower', groups: [], description: 'Heat the power plant generates in a fight: combat power draw x the plant\'s heat efficiency. Low Emissions, Armoured and Thermal Spread lower it; Overcharged raises it.', compute: (s) => combatDeployedDraw(s) * (s.standard[0].m?.getThermalEfficiency() ?? 0) },
  { name: 'combatHeat', unit: 'heat/s', better: 'lower', groups: [], description: 'Net heat per second in a fight: power plant heat plus weapon heat, minus what Thermal Vent beams remove while on target (context.ventHitRate, context.thermalVentFactor). Negative = the ship cools while firing. Thrusters and FSD charging are not included (no data); the hull\'s own dissipation is not modelled, so compare builds of the same hull.', compute: (s, c) => heatBudget(s, c).net },
  { name: 'thermalVentRemoval', unit: 'heat/s', better: 'higher', groups: [], description: 'Heat per second Thermal Vent beams remove while on target (assumed thermalVentFactor x their thermal load, reduced by falloff).', compute: (s, c) => heatBudget(s, c).ventRemoval },
  { name: 'thermalVentBreakEven', unit: 'x', better: 'lower', groups: [], description: 'Thermal Vent strength (multiple of the vent beams\' own thermal load) needed to cancel all other heat in a fight. If your ship reaches 0 heat with vents on target, the real factor is at least this. 1e6 = no vent beams.', compute: (s, c) => heatBudget(s, c).breakEvenFactor },
  { name: 'weaponTypes', unit: 'types', better: 'lower', groups: [], description: 'Number of distinct weapon types fitted (e.g. plasma accelerators + multi-cannons = 2). Cap it to keep a loadout with matching ballistics that is easy to aim and manage.', compute: (s) => new Set(weapons(s).map((m) => m.grp)).size },
  { name: 'mountTypes', unit: 'types', better: 'lower', groups: [], description: 'Number of distinct mount types among weapons (fixed, gimballed, turreted). 1 = all weapons share one mount type.', compute: (s) => new Set(weapons(s).map((m) => m.mount)).size },
  { name: 'weaponHeat', unit: 'heat/s', better: 'lower', groups: [], description: 'Heat generated per second by all weapons firing (Coriolis thermal load). Cap it at a proven build\'s value to avoid overheating loadouts.', compute: (s) => s.totalHps },
  { name: 'ammoEndurance', unit: 's', better: 'higher', groups: [], description: 'Seconds of continuous fire until the first ammunition-limited weapon runs completely dry (magazine + reserve; 1e6 = no ammo-limited weapons). Pair with DPS goals so finite ordnance is not mistaken for sustained damage.', compute: (s) => ammoEndurance(s) },
  { name: 'axSustainedDps', unit: 'dps', better: 'higher', groups: EXPERIMENTAL_WEAPONS, description: 'Raw sustained DPS from AX/Guardian (experimental) weapons only — the damage that counts against Thargoid interceptors.', compute: (s) => experimentalSdps(s) },
];

const BY_NAME = new Map(METRICS.map((m) => [m.name, m]));

/**
 * Resolves a metric name, including parametrised ones:
 *   count:<group>  number of fitted modules of a group (e.g. count:fs, count:ml)
 *   class:<group>  sum of the classes of fitted modules of a group (e.g. class:cc)
 */
export function getMetric(name: string): MetricDef {
  const found = BY_NAME.get(name);
  if (found) return found;
  const m = name.match(/^(count|class):([a-z0-9]+)$/i);
  if (m) {
    const [, kind, grp] = m;
    if (!groupCategory(grp)) throw new Error(`Unknown module group "${grp}" in metric "${name}". Use list_module_groups.`);
    const def: MetricDef = {
      name, unit: kind === 'count' ? 'modules' : 'class', better: 'higher', groups: [grp],
      description: kind === 'count' ? `Number of fitted ${grp} modules.` : `Sum of classes of fitted ${grp} modules.`,
      compute: (s) => modulesOf(s).filter((x) => x.grp === grp).reduce((a, x) => a + (kind === 'count' ? 1 : x.class), 0),
    };
    BY_NAME.set(name, def);
    return def;
  }
  throw new Error(`Unknown metric "${name}". Use list_metrics.`);
}

export function computeAll(ship: Ship, ctx: MetricContext): Record<string, number> {
  const out: Record<string, number> = {};
  for (const m of METRICS) out[m.name] = round(m.compute(ship, ctx));
  return out;
}

export function round(x: number): number {
  if (!Number.isFinite(x)) return x;
  const a = Math.abs(x);
  const digits = a >= 1000 ? 0 : a >= 10 ? 2 : 3;
  return Number(x.toFixed(digits));
}
