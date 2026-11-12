// Records the Node MCP server's answers to a fixed set of tool calls, for the Rust service parity
// test. Optimisation is covered by the optimiser benchmark instead (searches differ by design).
//   node scripts/oracle-tools.mjs   (after npm run build)
import { fileURLToPath } from 'node:url';
import { writeFileSync, mkdirSync } from 'node:fs';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const client = new Client({ name: 'corai-oracle', version: '0' });
await client.connect(new StdioClientTransport({
  command: process.execPath,
  args: ['--no-deprecation', `${root}dist/server.cjs`],
  env: { ...process.env, CORAI_UPDATE_CHECK: '0' },
}));

const call = async (name, args = {}) => {
  const r = await client.callTool({ name, arguments: args }, undefined, { timeout: 600_000 });
  const text = r.content?.[0]?.text ?? '';
  return r.isError ? { error: text } : { result: JSON.parse(text) };
};

// A real engineered build to pass around as a link.
const krait = await call('evaluate_build', { ship: 'krait_mkii', changes: [
  { slot: 'standard:fsd', group: 'fsd', class: 5, rating: 'A', blueprint: 'FSD_LongRange', special: 'special_fsd_heavy' },
  { slot: 'hardpoint:1', group: 'pa', class: 3, mount: 'F', blueprint: 'Weapon_Focused', special: 'special_plasma_slug_pa' },
  { slot: 'utility:1', group: 'sb', class: 0, rating: 'A', blueprint: 'ShieldBooster_HeavyDuty', special: 'special_shieldbooster_chunky' },
] });
const link = krait.result.coriolisUrl;
const slef = JSON.parse((await call('export_build', { build: link, name: 'Oracle' })).result.json);

const CALLS = [
  ['list_ships', {}],
  ['list_module_groups', {}],
  ['list_module_groups', { category: 'utility' }],
  ['list_metrics', {}],
  ['module_info', { group: 'fsd' }],
  ['module_info', { group: 'sb', class: 0 }],
  ['module_info', { group: 'pa' }],
  ['module_info', { group: 'gsc' }],
  ['module_info', { group: 'nope' }],
  ['ship_details', { ship: 'python' }],
  ['ship_details', { ship: 'Anaconda Mk II' }],
  ['ship_details', { build: link, context: { damagePreset: 'pve', sysPips: 4 } }],
  ['ship_details', { ship: 'not a ship' }],
  ['evaluate_build', { ship: 'krait_mkii' }],
  ['evaluate_build', { build: link, context: { damagePreset: 'pve', incomingDps: 300, opponent: 'federal_corvette', rangeMeters: 2000 } }],
  ['evaluate_build', { build: JSON.stringify(slef) }],
  ['evaluate_build', { ship: 'python', changes: [{ slot: 'internal:1', group: 'fs', class: 6, rating: 'A' }, { slot: 'bulkheads', bulkheadIndex: 2, blueprint: 'Armour_HeavyDuty', special: 'special_armour_chunky' }] }],
  ['evaluate_build', { ship: 'python', changes: [{ slot: 'standard:fsd', blueprint: 'FSD_LongRange', grade: 3 }] }],
  ['evaluate_build', { ship: 'python', changes: [{ slot: 'hardpoint:1', empty: true }, { slot: 'utility:2', moduleId: '04' }] }],
  ['evaluate_build', { ship: 'python', changes: [{ slot: 'internal:1', group: 'sg', class: 8 }] }],
  ['evaluate_build', { ship: 'python', changes: [{ slot: 'internal:99', empty: true }] }],
  ['evaluate_build', { ship: 'python', context: { damage: { kinetic: 1 }, wepPips: 2, ventHitRate: 0.5, thermalVentFactor: 3, corrosiveHullMultiplier: 1.5, reserveFuelInJumpMass: true } }],
  ['evaluate_build', {}],
  ['export_build', { build: link, name: 'Oracle' }],
  ['export_build', { ship: 'python', format: 'loadout', name: 'Hauler' }],
  ['export_build', { build: 'https://edsy.org/#/L=abc' }],
];

const cases = [];
for (const [tool, args] of CALLS) cases.push({ tool, args, ...(await call(tool, args)) });
const { tools } = await client.listTools();
const instructions = client.getInstructions();
await client.close();
// The Rust server's tool definitions and instructions are kept identical to these.
writeFileSync(`${root}crates/corai-service/tests/fixtures/tool-definitions.json`, JSON.stringify({ instructions, tools }, null, 1) + '\n');

const out = `${root}crates/corai-service/tests/fixtures/tools.json`;
mkdirSync(new URL('../crates/corai-service/tests/fixtures/', import.meta.url), { recursive: true });
writeFileSync(out, JSON.stringify({ generator: 'corai scripts/oracle-tools.mjs', cases }) + '\n');
process.stderr.write(`oracle-tools: ${cases.length} calls, ${cases.filter((c) => c.error).length} errors\n`);
