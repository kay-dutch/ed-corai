// Developer CLI: run the optimizer without an MCP client.
//   [LOCK=slot,kind] [FILTERS=json] node dist/cli.cjs optimize <shipId | build file | link> <metric> [effort] [constraintsJSON]
//   node dist/cli.cjs evaluate <link | SLEF file | journal file>
//   node dist/cli.cjs version | check-update | update [tag] --yes
import './stdio-guard.js';
import { existsSync, readFileSync } from 'node:fs';
import { stockShip } from './engine.js';
import { parseBuild } from './interchange.js';
import { applyChange } from './changes.js';
import { makeContext } from './metrics.js';
import { optimize, type Effort } from './optimizer.js';
import { summarize } from './describe.js';
import { BUILD } from './version.js';
import { checkForUpdates, installUpdate } from './update.js';

const [cmd, ...args] = process.argv.slice(2);
// SYS=4 INCOMING_DPS=300 DAMAGE='{"kinetic":1,"thermal":1}' override the metric context.
const ctx = makeContext({
  sys: process.env.SYS ? Number(process.env.SYS) : undefined,
  wep: process.env.WEP ? Number(process.env.WEP) : undefined,
  range: process.env.RANGE ? Number(process.env.RANGE) : undefined,
  ventHitRate: process.env.VENT_HIT ? Number(process.env.VENT_HIT) : undefined,
  thermalVentFactor: process.env.VENT_K ? Number(process.env.VENT_K) : undefined,
  corrosiveHullMultiplier: process.env.CORROSIVE_MULT ? Number(process.env.CORROSIVE_MULT) : undefined,
  corrosivePiercingBonus: process.env.CORROSIVE_AP ? Number(process.env.CORROSIVE_AP) : undefined,
  recoverySys: process.env.RECOVERY_SYS ? Number(process.env.RECOVERY_SYS) : undefined,
  incomingDps: process.env.INCOMING_DPS ? Number(process.env.INCOMING_DPS) : undefined,
  damage: process.env.DAMAGE ? JSON.parse(process.env.DAMAGE) : undefined,
  damagePreset: process.env.DAMAGE_PRESET as 'even' | 'pve' | undefined,
  regenUnderFire: process.env.REGEN_UNDER_FIRE ? Number(process.env.REGEN_UNDER_FIRE) : undefined,
});
const out = (x: unknown) => process.stdout.write(JSON.stringify(x, null, 2) + '\n');

if (cmd === 'optimize') {
  // Start from a stock hull (ship id) or a build file / link; LOCK=hardpoint,utility keeps slots.
  const [start, metric, effort = 'normal', constraints = '[]'] = args;
  const base = existsSync(start) ? parseBuild(readFileSync(start, 'utf8')) : /[/{]/.test(start) ? parseBuild(start) : stockShip(start);
  for (const c of process.env.CHANGES ? JSON.parse(process.env.CHANGES) : []) applyChange(base, c);
  const lock = process.env.LOCK ? process.env.LOCK.split(',') : undefined;
  const filters = process.env.FILTERS ? JSON.parse(process.env.FILTERS) : undefined; // e.g. {"allowPreEngineered":false}
  const keepBase = process.env.KEEP_BASE ? process.env.KEEP_BASE !== '0' : base.id !== start;
  const r = optimize({ base, keepBase, objective: { metric }, constraints: JSON.parse(constraints), lock, filters, ctx, effort: effort as Effort, timeLimitMs: 300_000 });
  out({ feasible: r.feasible, violations: r.violations, stats: r.stats, best: summarize(r.ship, ctx) });
} else if (cmd === 'evaluate') {
  // A file path (SLEF / journal) or a link / pasted build.
  const input = existsSync(args[0]) ? readFileSync(args[0], 'utf8') : args[0];
  const ship = parseBuild(input);
  // CHANGES='[{"slot":"internal:1","blueprint":"ShieldGenerator_Thermic","special":"special_shield_regenerative"}]'
  for (const c of process.env.CHANGES ? JSON.parse(process.env.CHANGES) : []) applyChange(ship, c);
  out(summarize(ship, ctx));
} else if (cmd === 'version') {
  out(BUILD);
} else if (cmd === 'check-update') {
  checkForUpdates().then((s) => { out(s); process.exitCode = s.updateAvailable ? 10 : 0; });
} else if (cmd === 'update') {
  void (async () => {
    const s = await checkForUpdates();
    const tag = args.find((a) => !a.startsWith('-'));
    if (!tag && !s.canAutoInstall) process.stderr.write(s.message + '\n');
    else if (!args.includes('--yes')) process.stderr.write(`${s.message}\nRun again with --yes to install ${tag ?? s.latest?.tag}.\n`);
    else out(await installUpdate(tag ?? s.latest?.tag));
  })().catch((e) => { process.stderr.write(`${e instanceof Error ? e.message : e}\n`); process.exitCode = 1; });
} else {
  process.stderr.write('usage: cli optimize <shipId> <metric> [effort] [constraintsJSON] | evaluate <url> | version | check-update | update [tag] --yes\n');
  process.exit(1);
}
