// MCP server: exposes the Coriolis engine and the build optimiser as tools for Claude.
import './stdio-guard.js';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';
import {
  Modules, Ships, findSlot, listSlots, place, shipData, shipIds, stockShip,
  type EngSpec, type Ship,
} from './engine.js';
import { describeEngineering, listGroups, maxBlueprintGrade, slotCandidates } from './catalog.js';
import { METRICS, getMetric, makeContext, round, type MetricContext } from './metrics.js';
import { compareScores, optimize, type OptimizeRequest } from './optimizer.js';
import { summarize } from './describe.js';
import { parseBuild, toLoadout, toSlef } from './interchange.js';
import { applyChange } from './changes.js';
import { BUILD } from './version.js';
import { checkForUpdates, installUpdate, type UpdateStatus } from './update.js';

const INSTRUCTIONS = `
Elite Dangerous ship-build optimiser built on the Coriolis (coriolis.io) calculation engine and data.
Every number comes from the engine and documented game mechanics, never from community tier lists. Workflow:
1. Translate the user's goal into ONE objective metric plus hard constraints (list_metrics shows them).
   E.g. "best exploration Krait" -> objective unladenRange, constraints count:fs>=1, count:am>=1, count:pv>=1.
   "Tanky PvE FdL" -> objective totalEHP with constraint dpsVsHull>=X; "max cargo T-9 that can still jump 20 Ly laden" -> cargoCapacity, ladenRange>=20.
2. Required modules are constraints on count:<group> (list_module_groups gives group codes: fs fuel scoop, am AFMU, pv SRV hangar,
   fh fighter hangar, dc docking computer, hs heat sink, rf refinery, cc collector limpets, ml mining laser, ...).
3. Use compare_ships to rank hulls (effort quick, sizes to restrict), then optimize_build on the best few with effort normal/thorough.
4. Improving a player's build (pass it as "build"):
   - Lock every slot the request does not touch (lock: ["hardpoint", "standard:fsd", "internal:3", ...]).
   - Turn what the build already achieves into constraints (current shieldEHP, hullEHP, sustainedDps, unladenRange, ...
     as minimums) so the result is never worse where the player did not ask for a trade.
   - Ties keep the player's modules (keepBase), but a metric that genuinely improves will still swap modules:
     check the changed slots and explain each one.
5. Keep builds playable: one weapon type and mount type where possible (weaponTypes, mountTypes), a real fuel tank,
   no large fuel scoop on combat ships, required utility modules as count: constraints.
6. Ask how the player flies when it matters and set context: sysPips/wepPips while fighting (default 2/4),
   recoverySysPips (default 4), damagePreset pve for NPC combat, incomingDps, rangeMeters, opponent.
7. Present the Coriolis and EDSY links and explain trade-offs with the numbers. Users may paste builds from Coriolis,
   EDSY (SLEF export or import link), Inara or their journal; export_build gives SLEF / journal JSON.
8. Explore trade-offs by re-running with tightened constraints (epsilon-constraint method) rather than inventing weights.
Assumptions (say so when a result depends on them, and offer a rerun with another value): the pve damage mix,
regenUnderFire=0, thermalVentFactor=1 (Thermal Vent heat removal is unmeasured), Corrosive Shell 1.25x hull damage and
+20 piercing (community figures), engineering at best roll.
Engineering defaults to grade 5 best-roll blueprints plus the best experimental effect per module ("engineering":"none" for stock).
Guardian modules cannot be engineered; life support, sensors and SCO FSDs keep their slot size; all modules count as obtainable.
Searched module groups are limited to those that can affect the requested metrics; use extraGroups or count:<group> constraints to bring others in.
Optimisation is a heuristic local search: very good but not a proof of global optimality; higher effort = more restarts.
When the user asks about updates, use check_for_updates; install_update only after the user agrees.
`.trim();

const BUILD_HELP = 'A build in any common form: Coriolis outfit or import link, EDSY import link, SLEF JSON (EDSY: Export → SLEF), or a journal Loadout event.';

const server = new McpServer({ name: 'ed-corai', version: BUILD.version }, { instructions: INSTRUCTIONS });

// One background release check per server start (CORAI_UPDATE_CHECK=0 turns it off). When a newer
// release exists, the next tool result carries a one-time notice so Claude can offer the update.
let pendingNotice: string | undefined;
if (process.env.CORAI_UPDATE_CHECK !== '0') {
  checkForUpdates().then((s: UpdateStatus) => {
    if (s.updateAvailable) pendingNotice = `${s.message} Offer the update to the user; install_update installs it only after they agree.`;
  }).catch(() => undefined);
}
/** Appends the pending update notice (once) as a separate text block after the tool's JSON. */
const withNotice = <R extends { content: { type: 'text'; text: string }[] }>(r: R): R => {
  if (!pendingNotice) return r;
  const text = `Update notice: ${pendingNotice}`;
  pendingNotice = undefined;
  return { ...r, content: [...r.content, { type: 'text' as const, text }] };
};

const json = (x: unknown) => ({ content: [{ type: 'text' as const, text: JSON.stringify(x, null, 1) }] });
const fail = (e: unknown) => ({ isError: true, content: [{ type: 'text' as const, text: e instanceof Error ? e.message : String(e) }] });
const wrap = <A,>(fn: (a: A) => unknown) => async (a: A) => { try { return withNotice(json(await fn(a))); } catch (e) { return fail(e); } };

// ---------- shared schemas ----------

const contextSchema = z.object({
  sysPips: z.number().min(0).max(4).optional().describe('SYS pips while fighting: shield resistance, regen under fire (default 2)'),
  wepPips: z.number().min(0).max(4).optional().describe('WEP pips while fighting: weapon capacitor sustain (default 4)'),
  recoverySysPips: z.number().min(0).max(4).optional().describe('SYS pips while recovering: shield recharge and rebuild times (default 4)'),
  damage: z.object({
    kinetic: z.number().min(0).optional(), thermal: z.number().min(0).optional(),
    explosive: z.number().min(0).optional(), absolute: z.number().min(0).optional(),
  }).optional().describe('Incoming damage mix for effective-HP metrics (relative weights; default equal kinetic/thermal/explosive)'),
  opponent: z.string().optional().describe('Reference target for dpsVsShields/dpsVsHull: ship id (stock) or any build (link, SLEF). Default stock Anaconda'),
  opponentSysPips: z.number().min(0).max(4).optional(),
  rangeMeters: z.number().min(0).optional().describe('Engagement range for falloff (default 1000)'),
  incomingDps: z.number().min(0).optional().describe('Raw incoming DPS for shieldTimeUnderFire (default 200)'),
  damagePreset: z.enum(['even', 'pve']).optional().describe('Named damage mix: even (default) or pve (thermal-heavy NPC fire: K 0.30 / T 0.60 / E 0.05 / Abs 0.05, an assumption). Explicit damage weights override it.'),
  regenUnderFire: z.number().min(0).max(1).optional().describe('Share of time shields regenerate while taking fire (default 0; the in-game pause after a hit is unmeasured)'),
  ventHitRate: z.number().min(0).max(1).optional().describe('Share of time Thermal Vent beams are on target (default 1)'),
  thermalVentFactor: z.number().min(0).optional().describe('Heat a hitting Thermal Vent beam removes, as a multiple of its own thermal load (default 1; unmeasured)'),
  corrosiveHullMultiplier: z.number().min(1).optional().describe('Hull damage multiplier while a Corrosive Shell weapon keeps hitting (default 1.25; community figure, does not stack)'),
  corrosivePiercingBonus: z.number().min(0).optional().describe('Armour piercing added to the Corrosive Shell weapon itself (default 20; community figure)'),
  reserveFuelInJumpMass: z.boolean().optional().describe('Count the reserve tank in jump mass like Coriolis (default false, matching the journal MaxJumpRange)'),
}).optional();

const goalSchema = z.object({
  metric: z.string().describe('Metric name from list_metrics, or count:<group> / class:<group>'),
  direction: z.enum(['max', 'min']).optional().describe('Defaults to the metric\'s natural direction'),
});

const constraintSchema = z.object({ metric: z.string(), min: z.number().optional(), max: z.number().optional() });

const searchOptions = {
  objective: goalSchema,
  tiebreakers: z.array(goalSchema).optional().describe('Secondary goals used only to break exact ties'),
  constraints: z.array(constraintSchema).optional().describe('Hard bounds, e.g. {metric:"ladenRange",min:20}, {metric:"count:fs",min:1}'),
  engineering: z.enum(['full', 'none']).optional().describe('full = G5 max roll + best experimental (default); none = stock modules'),
  power: z.enum(['priority', 'deployed', 'retracted', 'ignore']).optional().describe('Power budget check. priority (default): combat modules powered with hardpoints out, non-combat ones (scoop, docking computer, SRV bay, ...) shed by priority as in game, everything powered with hardpoints retracted. deployed: everything on with hardpoints out.'),
  requireBoost: z.boolean().optional().describe('Distributor must be able to boost (default true)'),
  extraGroups: z.array(z.string()).optional().describe('Extra module groups to consider even if no metric needs them'),
  filters: z.object({
    allowPowerplay: z.boolean().optional(), allowPreEngineered: z.boolean().optional(),
    allowCommunityGoal: z.boolean().optional(), allowGuardian: z.boolean().optional(),
    allowExperimental: z.boolean().optional().describe('AX/Guardian experimental weapons (max 4 per ship)'),
    excludeGroups: z.array(z.string()).optional(), excludeModules: z.array(z.string()).optional(),
  }).optional(),
  context: contextSchema,
  effort: z.enum(['quick', 'normal', 'thorough']).optional(),
  seed: z.number().int().optional(),
};

function resolveOpponent(spec?: string): Ship | undefined {
  if (!spec) return undefined;
  return /[/{[]/.test(spec) || spec.includes('code=') ? parseBuild(spec) : stockShip(spec);
}

function buildContext(c?: z.infer<typeof contextSchema>): MetricContext {
  return makeContext({
    sys: c?.sysPips, wep: c?.wepPips, recoverySys: c?.recoverySysPips, damage: c?.damage, opponent: resolveOpponent(c?.opponent),
    opponentSys: c?.opponentSysPips, range: c?.rangeMeters, incomingDps: c?.incomingDps,
    damagePreset: c?.damagePreset, regenUnderFire: c?.regenUnderFire, reserveFuelInJumpMass: c?.reserveFuelInJumpMass,
    ventHitRate: c?.ventHitRate, thermalVentFactor: c?.thermalVentFactor,
    corrosiveHullMultiplier: c?.corrosiveHullMultiplier, corrosivePiercingBonus: c?.corrosivePiercingBonus,
  });
}

function resolveShipId(name: string): string {
  if (Ships[name]) return name;
  const lower = name.toLowerCase().replace(/[^a-z0-9]/g, '');
  const hit = shipIds().find((id) => id.replace(/_/g, '') === lower || String(Ships[id].properties.name).toLowerCase().replace(/[^a-z0-9]/g, '') === lower);
  if (!hit) throw new Error(`Unknown ship "${name}". Use list_ships.`);
  return hit;
}

function loadBase(a: { ship?: string; build?: string; startFrom?: 'stock' | 'build' }): Ship {
  if (a.build) return parseBuild(a.build, a.ship ? resolveShipId(a.ship) : undefined);
  if (!a.ship) throw new Error('Pass either ship (id/name) or build (Coriolis URL).');
  return stockShip(resolveShipId(a.ship));
}

function slotLayout(id: string) {
  const s = stockShip(id);
  const refs = listSlots(s);
  const sizes = (kind: string) => refs.filter((r) => r.kind === kind).map((r) => `${r.maxClass}${r.name ? `(${r.name})` : ''}`);
  return { hardpoints: sizes('hardpoint'), utility: sizes('utility').length, internal: sizes('internal'), standard: refs.filter((r) => r.kind === 'standard').map((r) => `${r.key.split(':')[1]}${r.maxClass}`) };
}

const links = (b: { coriolisUrl: string; edsyUrl: string }) => ({ coriolisUrl: b.coriolisUrl, edsyUrl: b.edsyUrl });

function scoreSummary(req: OptimizeRequest, r: ReturnType<typeof optimize>, ctx: MetricContext) {
  const wanted = new Set([req.objective.metric, ...(req.tiebreakers ?? []).map((t) => t.metric), ...(req.constraints ?? []).map((c) => c.metric)]);
  const key: Record<string, number> = {};
  for (const n of wanted) key[n] = round(getMetric(n).compute(r.ship, ctx));
  return key;
}

// ---------- tools ----------

server.registerTool('list_ships', {
  title: 'List ships',
  description: 'All ships with id, size class (1 small - 3 large), price, base speed/boost, mass, masslock and slot layout.',
  inputSchema: {},
  annotations: { readOnlyHint: true },
}, wrap(() => shipIds().map((id) => {
  const p = Ships[id].properties;
  return { id, name: p.name, manufacturer: p.manufacturer, size: p.class, hullCost: Ships[id].retailCost, speed: p.speed, boost: p.boost, hullMass: p.hullMass, masslock: p.masslock, baseShield: p.baseShieldStrength, baseArmour: p.baseArmour, ...slotLayout(id) };
})));

server.registerTool('ship_details', {
  title: 'Ship details',
  description: 'Slot keys/sizes/restrictions of a ship plus its full stock (or given) build summary. Slot keys are what lock/changes refer to.',
  inputSchema: { ship: z.string().optional(), build: z.string().optional().describe(BUILD_HELP), context: contextSchema },
  annotations: { readOnlyHint: true },
}, wrap((a: { ship?: string; build?: string; context?: any }) => {
  const s = loadBase(a);
  return {
    properties: shipData(s.id).properties,
    bulkheads: shipData(s.id).bulkheads.map((b: any, i: number) => ({ index: i, name: b.name, mass: b.mass, hullboost: b.hullboost })),
    slots: listSlots(s).map((r) => ({ slot: r.key, size: r.maxClass, restriction: r.name, eligibleGroups: r.eligible ? Object.keys(r.eligible) : undefined })),
    build: summarize(s, buildContext(a.context), { includeEmpty: true }),
  };
}));

server.registerTool('list_module_groups', {
  title: 'List module groups',
  description: 'Module group codes (used in count:<group> metrics, extraGroups, filters) with names and categories.',
  inputSchema: { category: z.enum(['standard', 'hardpoint', 'utility', 'internal']).optional() },
  annotations: { readOnlyHint: true },
}, wrap((a: { category?: string }) => listGroups().filter((g) => !a.category || g.category === a.category)));

server.registerTool('module_info', {
  title: 'Module info',
  description: 'Stock stats of every module in a group (optionally one class) and the blueprints / experimental effects that apply to it, with grade-5 feature ranges.',
  inputSchema: { group: z.string(), class: z.number().int().optional() },
  annotations: { readOnlyHint: true },
}, wrap((a: { group: string; class?: number }) => {
  const all = [...(Modules.standard[a.group] ?? []), ...(Modules.hardpoints[a.group] ?? []), ...(Modules.internal[a.group] ?? [])];
  if (!all.length) throw new Error(`Unknown group "${a.group}". Use list_module_groups.`);
  const pick = ['id', 'class', 'rating', 'mount', 'missile', 'name', 'mass', 'power', 'cost', 'integrity', 'pgen', 'optmass', 'maxmass', 'maxfuel', 'fuelmul', 'engcap', 'engrate', 'syscap', 'sysrate', 'wepcap', 'weprate', 'shieldboost', 'explres', 'kinres', 'thermres', 'hullreinforcement', 'damage', 'fireint', 'clip', 'reload', 'distdraw', 'thermload', 'range', 'falloff', 'piercing', 'cargo', 'passengers', 'fuel', 'bins', 'jumpboost', 'shieldreinforcement', 'shieldaddition', 'experimental', 'pp', 'preEngineered'];
  return {
    modules: all.filter((m) => a.class === undefined || m.class === a.class).map((m) => Object.fromEntries(pick.filter((k) => m[k] !== undefined).map((k) => [k, k === 'preEngineered' ? m[k].description : m[k]]))),
    engineering: describeEngineering(a.group),
  };
}));

server.registerTool('list_metrics', {
  title: 'List metrics',
  description: 'Every metric usable as objective/constraint, with unit and meaning. Also: count:<group> and class:<group>.',
  inputSchema: {},
  annotations: { readOnlyHint: true },
}, wrap(() => ({
  metrics: METRICS.map((m) => ({ name: m.name, unit: m.unit, better: m.better, description: m.description })),
  parametrised: ['count:<group> = number of fitted modules of a group', 'class:<group> = sum of classes of fitted modules of a group'],
  context: 'Effective-HP metrics use context.damage/damagePreset and sysPips; shieldTimeUnderFire uses incomingDps and regenUnderFire; dpsVs*, pierceEfficiency and timeToKill use context.opponent and rangeMeters; weaponSustain uses wepPips; jump ranges use reserveFuelInJumpMass.',
  assumptions: 'Defaults that no published measurement settles: the pve damage mix, regenUnderFire=0, thermalVentFactor=1, the Corrosive Shell 1.25 hull multiplier and +20 piercing (community figures), reserve fuel excluded from jump mass, Coriolis hull-resistance stacking. Rerun with the alternative when a ranking is close.',
})));

const changeSchema = z.object({
  slot: z.string().describe('Slot key from ship_details, e.g. internal:3, hardpoint:1, utility:2, standard:fsd, bulkheads'),
  empty: z.boolean().optional().describe('Remove the module'),
  moduleId: z.string().optional().describe('Exact module id (from module_info)'),
  group: z.string().optional(), class: z.number().int().optional(), rating: z.string().optional(),
  mount: z.enum(['F', 'G', 'T']).optional(), bulkheadIndex: z.number().int().optional(),
  blueprint: z.string().optional().describe('Blueprint key, e.g. FSD_LongRange (module_info lists them)'),
  grade: z.number().int().optional().describe('Default: highest grade'),
  special: z.string().optional().describe('Experimental key, e.g. special_fsd_heavy'),
});

server.registerTool('evaluate_build', {
  title: 'Evaluate build',
  description: 'Full stats of a build (Coriolis URL or stock ship), optionally after applying slot changes. Use for what-if comparisons; returns a new Coriolis URL.',
  inputSchema: {
    ship: z.string().optional(), build: z.string().optional().describe(BUILD_HELP),
    changes: z.array(changeSchema).optional(), context: contextSchema,
  },
  annotations: { readOnlyHint: true },
}, wrap((a: { ship?: string; build?: string; changes?: any[]; context?: any }) => {
  const s = loadBase(a);
  for (const c of a.changes ?? []) applyChange(s, c);
  return summarize(s, buildContext(a.context));
}));

server.registerTool('export_build', {
  title: 'Export build',
  description: 'A build as SLEF (EDCD Ship Loadout Exchange Format, accepted by EDSY, Coriolis, Inara, EDDiscovery) or as a Frontier journal Loadout event, plus Coriolis and EDSY links.',
  inputSchema: {
    ship: z.string().optional(), build: z.string().optional().describe(BUILD_HELP),
    format: z.enum(['slef', 'loadout']).optional().describe('Default slef'),
    name: z.string().optional().describe('Build name to embed'),
  },
  annotations: { readOnlyHint: true },
}, wrap((a: { ship?: string; build?: string; format?: 'slef' | 'loadout'; name?: string }) => {
  const s = loadBase(a);
  const data = a.format === 'loadout' ? toLoadout(s, a.name) : toSlef(s, a.name);
  const links = summarize(s, makeContext(), { buildName: a.name });
  return { format: a.format ?? 'slef', json: JSON.stringify(data), coriolisUrl: links.coriolisUrl, edsyUrl: links.edsyUrl };
}));

server.registerTool('optimize_build', {
  title: 'Optimise build',
  description: 'Search for the outfitting of one ship that maximises/minimises an objective metric subject to constraints, using the Coriolis engine for every evaluation. Starts from stock or a given build; lock keeps slots unchanged.',
  inputSchema: {
    ship: z.string().optional().describe('Ship id or name'),
    build: z.string().optional().describe(`${BUILD_HELP} Optimisation starts from it; locked slots keep its modules.`),
    lock: z.array(z.string()).optional().describe('Slot keys or kinds to keep: e.g. ["hardpoint","utility:1","standard:fsd"]'),
    keepBase: z.boolean().optional().describe('On equal scores keep the starting build\'s modules instead of the cheaper fit (default true when build is given)'),
    timeLimitSeconds: z.number().positive().optional().describe('Default 60'),
    ...searchOptions,
  },
  annotations: { readOnlyHint: true },
}, wrap((a: any) => {
  const ctx = buildContext(a.context);
  const req: OptimizeRequest = { ...a, base: loadBase(a), keepBase: a.keepBase ?? Boolean(a.build), ctx, timeLimitMs: (a.timeLimitSeconds ?? 60) * 1000 };
  const r = optimize(req);
  return {
    feasible: r.feasible,
    violations: r.violations,
    key: scoreSummary(req, r, ctx),
    best: summarize(r.ship, ctx),
    alternatives: r.alternatives.map((alt) => ({ key: scoreSummary(req, { ...r, ship: alt.ship }, ctx), ...links(summarize(alt.ship, ctx)) })),
    search: r.stats,
  };
}));

server.registerTool('compare_ships', {
  title: 'Compare ships',
  description: 'Run the optimiser on many hulls with the same objective/constraints and rank them. Default: all ships, effort quick. Use to answer "which ship is best for X".',
  inputSchema: {
    ships: z.array(z.string()).optional().describe('Ship ids/names; default all'),
    sizes: z.array(z.number().int().min(1).max(3)).optional().describe('Restrict to ship size classes (1 small, 2 medium, 3 large)'),
    maxHullCost: z.number().optional(),
    top: z.number().int().positive().optional().describe('Rows to return (default 10)'),
    timeLimitSeconds: z.number().positive().optional().describe('Total budget (default 240)'),
    ...searchOptions,
  },
  annotations: { readOnlyHint: true },
}, wrap((a: any) => {
  const ctx = buildContext(a.context);
  let ids = a.ships ? a.ships.map(resolveShipId) : shipIds();
  if (a.sizes) ids = ids.filter((id: string) => a.sizes.includes(Ships[id].properties.class));
  if (a.maxHullCost) ids = ids.filter((id: string) => Ships[id].retailCost <= a.maxHullCost);
  const budget = (a.timeLimitSeconds ?? 240) * 1000;
  const t0 = Date.now();
  const rows: any[] = [];
  for (const id of ids) {
    const left = budget - (Date.now() - t0);
    const perShip = Math.max(2000, left / Math.max(1, ids.length - rows.length));
    const req: OptimizeRequest = { ...a, effort: a.effort ?? 'quick', base: stockShip(id), ctx, timeLimitMs: perShip };
    try {
      const r = optimize(req);
      rows.push({ ship: id, name: Ships[id].properties.name, feasible: r.feasible, score: r.score, key: scoreSummary(req, r, ctx), totalCost: Math.round(r.ship.totalCost), ...links(summarize(r.ship, ctx)), violations: r.feasible ? undefined : r.violations, timedOut: r.stats.timedOut || undefined });
    } catch (e) {
      rows.push({ ship: id, error: e instanceof Error ? e.message : String(e), score: { violation: Infinity, values: [] } });
    }
  }
  rows.sort((x, y) => compareScores(y.score, x.score));
  return {
    ranked: rows.slice(0, a.top ?? 10).map(({ score, ...r }, i) => ({ rank: i + 1, ...r })),
    infeasible: rows.filter((r) => !r.feasible).map((r) => r.ship),
    seconds: Math.round((Date.now() - t0) / 1000),
  };
}));

server.registerTool('check_for_updates', {
  title: 'Check for updates',
  description: 'Installed corai version, the Coriolis code and data it was built from, and the latest corai release on GitHub with its notes.',
  inputSchema: {},
  annotations: { readOnlyHint: true, openWorldHint: true },
}, async () => {
  try { return json(await checkForUpdates()); } catch (e) { return fail(e); }
});

server.registerTool('install_update', {
  title: 'Install update',
  description: 'Updates this ED Corai install (a clean git clone) to a release: fetches the tag, refreshes the Coriolis submodules, installs dependencies and rebuilds. Only call after the user explicitly agreed. The MCP server must be restarted afterwards (Claude Code: /mcp, Claude Desktop: restart the app).',
  inputSchema: {
    version: z.string().optional().describe('Release tag, e.g. "v0.2.0". Default: the latest release.'),
    confirm: z.literal(true).describe('Must be true: the user agreed to install the update.'),
  },
  annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: true },
}, async (a: { version?: string }) => {
  try { return json(await installUpdate(a.version)); } catch (e) { return fail(e); }
});

server.connect(new StdioServerTransport()).catch((e) => {
  console.error(e);
  process.exit(1);
});
