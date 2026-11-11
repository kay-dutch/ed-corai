// End-to-end: spawn the MCP server over stdio and call its tools like Claude would.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const server = fileURLToPath(new URL('../dist/server.cjs', import.meta.url));
let client;

const call = async (name, args = {}) => {
  const r = await client.callTool({ name, arguments: args }, undefined, { timeout: 600_000 });
  assert.ok(!r.isError, `${name} failed: ${r.content?.[0]?.text}`);
  return JSON.parse(r.content[0].text);
};

before(async () => {
  client = new Client({ name: 'corai-test', version: '0' });
  // No background release check: tests must not depend on the network.
  await client.connect(new StdioClientTransport({ command: process.execPath, args: ['--no-deprecation', server], env: { ...process.env, CORAI_UPDATE_CHECK: '0' } }));
});

after(async () => client?.close());

test('exposes the expected tools', async () => {
  const { tools } = await client.listTools();
  assert.deepEqual(tools.map((t) => t.name).sort(), [
    'check_for_updates', 'compare_ships', 'evaluate_build', 'export_build', 'install_update', 'list_metrics', 'list_module_groups', 'list_ships', 'module_info', 'optimize_build', 'ship_details',
  ]);
});

test('stock Python matches Coriolis', async () => {
  const b = await call('evaluate_build', { ship: 'python' });
  // Coriolis' "unladen mass" (696 t) includes the full 32 t tank; the metric is now dry mass.
  assert.equal(b.metrics.dryMass, 664);
  assert.equal(b.metrics.fuelCapacity, 32);
  assert.equal(b.metrics.cargoCapacity, 82);
  assert.match(b.coriolisUrl, /^https:\/\/coriolis\.io\/outfit\/python\?code=/);
});

test('Coriolis URL round-trips through evaluate_build', async () => {
  const a = await call('evaluate_build', { ship: 'krait_mkii', changes: [
    { slot: 'standard:fsd', group: 'fsd', class: 5, rating: 'A', blueprint: 'FSD_LongRange', special: 'special_fsd_heavy' },
  ] });
  const b = await call('evaluate_build', { build: a.coriolisUrl });
  assert.equal(b.metrics.unladenRange, a.metrics.unladenRange);
  assert.ok(a.modules.find((m) => m.slot === 'standard:fsd').engineering.includes('G5'));
});

test('exploration optimisation satisfies required modules and beats stock range', async () => {
  const stock = await call('evaluate_build', { ship: 'diamondback_explorer' });
  const r = await call('optimize_build', {
    ship: 'Diamondback Explorer', objective: { metric: 'unladenRange' }, effort: 'quick',
    constraints: [{ metric: 'count:fs', min: 1 }, { metric: 'count:am', min: 1 }],
  });
  assert.equal(r.feasible, true, JSON.stringify(r.violations));
  assert.ok(r.key['count:fs'] >= 1 && r.key['count:am'] >= 1);
  assert.ok(r.best.metrics.unladenRange > stock.metrics.unladenRange * 1.5);
  assert.ok(r.best.checks.powerOk && r.best.checks.canBoost);
});

test('compare_ships ranks hulls', async () => {
  const r = await call('compare_ships', { ships: ['sidewinder', 'eagle', 'hauler'], objective: { metric: 'cargoCapacity' }, engineering: 'none' });
  assert.equal(r.ranked.length, 3);
  assert.equal(r.ranked[0].ship, 'hauler');
});
