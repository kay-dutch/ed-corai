// Pulls the newest Coriolis code and data (the branch set in .gitmodules), rebuilds, runs the
// tests and reports which ships and modules changed. Failing tests roll the update back.
//
//   node scripts/update-upstream.mjs            update, test, report
//   node scripts/update-upstream.mjs --check    only report what is new upstream
//   node scripts/update-upstream.mjs --keep     keep the update even if tests fail
//   node scripts/update-upstream.mjs --report report.md   also write the report to a file
//
// EDSY is watched, not imported: its data licence (non-commercial) does not fit ED Corai's MIT
// licence, so the report only lists EDSY commits newer than the Coriolis data as a heads-up.
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SUBMODULES = ['upstream/coriolis', 'upstream/coriolis-data'];
const args = process.argv.slice(2);
const checkOnly = args.includes('--check');
const keep = args.includes('--keep');
const reportFile = args.includes('--report') ? args[args.indexOf('--report') + 1] : undefined;

const sh = (cmd, cmdArgs, opts = {}) =>
  execFileSync(cmd, cmdArgs, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts }).trim();
const git = (dir, ...a) => sh('git', ['-C', path.join(root, dir), ...a]);
const branchOf = (dir) => {
  try { return sh('git', ['config', '-f', '.gitmodules', `submodule.${dir}.branch`]); } catch { return 'master'; }
};

// ---------- data snapshot ----------

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = path.join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.json')) out.push(p);
  }
  return out;
}

function collectModules(node, out) {
  if (Array.isArray(node)) node.forEach((x) => collectModules(x, out));
  else if (node && typeof node === 'object') {
    if (typeof node.id === 'string' && node.grp) out.set(node.id, node);
    else Object.values(node).forEach((x) => collectModules(x, out));
  }
}

function snapshot() {
  const data = path.join(root, 'upstream/coriolis-data');
  const ships = new Map();
  for (const f of walk(path.join(data, 'ships'))) {
    // ship-slots.json and similar helpers sit beside the ships; real ships have properties.name.
    for (const [id, ship] of Object.entries(JSON.parse(readFileSync(f, 'utf8')))) if (ship?.properties?.name) ships.set(id, ship);
  }
  const modules = new Map();
  for (const f of walk(path.join(data, 'modules'))) collectModules(JSON.parse(readFileSync(f, 'utf8')), modules);
  return { ships, modules };
}

const moduleLabel = (m) => m.name ?? `${m.class ?? ''}${m.rating ?? ''} ${m.grp}${m.mount ? ` (${m.mount})` : ''}`.trim();

function diff(before, after) {
  const changes = (a, b) => ({
    added: [...b.keys()].filter((k) => !a.has(k)),
    removed: [...a.keys()].filter((k) => !b.has(k)),
    changed: [...b.keys()].filter((k) => a.has(k) && JSON.stringify(a.get(k)) !== JSON.stringify(b.get(k))),
  });
  return { ships: changes(before.ships, after.ships), modules: changes(before.modules, after.modules) };
}

// ---------- EDSY watch ----------

async function edsyCommitsSince(iso) {
  try {
    const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'ed-corai-upstream-check' };
    if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
    const res = await fetch(`https://api.github.com/repos/taleden/EDSY/commits?since=${encodeURIComponent(iso)}&per_page=20`, { headers, signal: AbortSignal.timeout(10_000) });
    if (!res.ok) return { error: `GitHub answered ${res.status}` };
    return { commits: (await res.json()).map((c) => ({ date: c.commit.committer.date.slice(0, 10), message: c.commit.message.split('\n')[0] })) };
  } catch (e) {
    return { error: e.message };
  }
}

// ---------- main ----------

const lines = [];
const say = (s = '') => { lines.push(s); console.log(s); };

const old = Object.fromEntries(SUBMODULES.map((d) => [d, git(d, 'rev-parse', 'HEAD')]));
for (const d of SUBMODULES) git(d, 'fetch', '--quiet', 'origin', branchOf(d));
const upstreamHead = Object.fromEntries(SUBMODULES.map((d) => [d, git(d, 'rev-parse', 'FETCH_HEAD')]));
const behind = Object.fromEntries(SUBMODULES.map((d) => [d, Number(git(d, 'rev-list', '--count', `HEAD..${upstreamHead[d]}`))]));
const dataDate = git('upstream/coriolis-data', 'log', '-1', '--format=%cI', upstreamHead['upstream/coriolis-data']);

say('# Upstream update');
say();
for (const d of SUBMODULES) {
  say(`- ${d} (${branchOf(d)}): ${behind[d] ? `${behind[d]} new commit(s), ${old[d].slice(0, 7)} → ${upstreamHead[d].slice(0, 7)}` : `up to date at ${old[d].slice(0, 7)}`}`);
}
const anyNew = SUBMODULES.some((d) => behind[d] > 0);

if (anyNew) {
  say();
  say('## Commits');
  for (const d of SUBMODULES.filter((x) => behind[x])) {
    say(`### ${d}`);
    for (const l of git(d, 'log', '--no-merges', '--format=- %as %s', `HEAD..${upstreamHead[d]}`).split('\n').slice(0, 30)) say(l);
  }
}

let failed = false;
if (anyNew && !checkOnly) {
  const before = snapshot();
  sh('git', ['submodule', 'update', '--remote', ...SUBMODULES]);
  const d = diff(before, snapshot());
  say();
  say('## Data changes');
  const list = (title, ids, label) => {
    if (!ids.length) return;
    say(`- ${title} (${ids.length}): ${ids.slice(0, 40).map(label).join(', ')}${ids.length > 40 ? ', …' : ''}`);
  };
  const after = snapshot();
  const shipName = (id) => after.ships.get(id)?.properties?.name ?? before.ships.get(id)?.properties?.name ?? id;
  const modName = (id) => moduleLabel(after.modules.get(id) ?? before.modules.get(id));
  list('New ships', d.ships.added, shipName);
  list('Removed ships', d.ships.removed, shipName);
  list('Changed ships', d.ships.changed, shipName);
  list('New modules', d.modules.added, modName);
  list('Removed modules', d.modules.removed, modName);
  list('Changed modules', d.modules.changed, modName);
  if (!Object.values(d).some((x) => x.added.length + x.removed.length + x.changed.length)) say('- No ship or module data changed.');

  say();
  say('## Tests');
  const tail = (e) => String(e.stdout ?? '').split('\n').filter((l) => /not ok|fail|Error|panicked|mismatches/i.test(l)).slice(0, 15);
  try {
    sh('npm', ['test'], { stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 64 * 1024 * 1024 });
    say('- npm test passed.');
  } catch (e) {
    failed = true;
    say('- npm test FAILED:');
    for (const l of tail(e)) say(`  ${l}`);
  }
  // The Rust engine is checked against the Node (Coriolis) engine on the new data: regenerate the
  // parity fixture and run the Rust tests. Skipped where cargo is not installed.
  if (!failed) {
    let cargo = true;
    try { sh('cargo', ['--version']); } catch { cargo = false; }
    if (!cargo) {
      say('- Rust: cargo not found, Rust parity not checked.');
    } else {
      try {
        sh('npm', ['run', 'oracle'], { maxBuffer: 64 * 1024 * 1024 });
        sh('cargo', ['test', '--workspace', '--locked'], { maxBuffer: 64 * 1024 * 1024 });
        say('- Rust parity fixture regenerated; cargo test passed.');
      } catch (e) {
        failed = true;
        say('- Rust parity FAILED on the new data (cargo test):');
        for (const l of tail(e)) say(`  ${l}`);
      }
    }
  }
  if (failed && !keep) {
    for (const dir of SUBMODULES) git(dir, 'checkout', '--quiet', old[dir]);
    sh('git', ['checkout', '--quiet', '--', 'crates/corai-engine/tests/fixtures/oracle.json']);
    sh('node', ['scripts/build.mjs']);
    say('- Rolled back to the previous Coriolis commits (use --keep to keep the update).');
  }
}

say();
say('## EDSY (watched, not imported)');
const edsy = await edsyCommitsSince(dataDate);
if (edsy.error) say(`- Could not read EDSY commits: ${edsy.error}`);
else if (!edsy.commits.length) say(`- No EDSY commits since the Coriolis data (${dataDate.slice(0, 10)}).`);
else {
  say(`- EDSY commits since the Coriolis data (${dataDate.slice(0, 10)}); new ships or modules here may not be in Coriolis yet:`);
  for (const c of edsy.commits) say(`  - ${c.date} ${c.message}`);
}

if (reportFile) writeFileSync(reportFile, lines.join('\n') + '\n');
process.exitCode = failed ? 1 : 0;
