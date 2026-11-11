// Every optimised build must pass Coriolis' own outfitting and engineering rules.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const cli = fileURLToPath(new URL('../dist/cli.cjs', import.meta.url));
const run = (args, env = {}) => JSON.parse(execFileSync(process.execPath, ['--no-deprecation', cli, ...args], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 64 << 20 }));

const cases = [
  ['anaconda2', 'shieldTimeUnderFire', '[{"metric":"sustainedDps","min":100},{"metric":"emptyHardpoints","max":0}]'],
  ['federal_corvette', 'totalEHP', '[{"metric":"sustainedDps","min":100}]'],
  ['krait_mkii', 'unladenRange', '[{"metric":"count:fs","min":1}]'],
  ['type_9_heavy', 'cargoCapacity', '[{"metric":"ladenRange","min":15}]'],
  ['fer_de_lance', 'dpsVsHull', '[{"metric":"shieldEHP","min":2000}]'],
];

for (const [ship, metric, constraints] of cases) {
  test(`${ship} optimised for ${metric} is a legal build`, () => {
    const r = run(['optimize', ship, metric, 'quick', constraints]);
    const illegal = r.violations.filter((v) => v.startsWith('illegal:'));
    assert.deepEqual(illegal, [], illegal.join('\n'));
    for (const m of r.best.modules) {
      assert.ok(!/Guardian/.test(m.module) || !m.engineering || /pre-engineered/.test(m.engineering), `${m.slot}: engineered Guardian module`);
    }
  });
}

test('core modules keep their slot class', () => {
  const r = run(['optimize', 'anaconda', 'unladenRange', 'quick']);
  const by = Object.fromEntries(r.best.modules.map((m) => [m.slot, m.module]));
  assert.match(by['standard:ls'], /^5/, 'life support');
  assert.match(by['standard:s'], /^8/, 'sensors');
});
