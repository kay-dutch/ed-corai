// Bundles the MCP server and dev CLI, with the upstream Coriolis engine compiled in headless.
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const stub = (name) => path.join(root, 'engine/stubs', name);

// Version of ED Corai and of the Coriolis code and data compiled into this build.
const git = (dir, ...args) => {
  try { return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(); } catch { return undefined; }
};
const upstream = (name) => {
  const dir = path.join(root, 'upstream', name);
  return { commit: git(dir, 'rev-parse', 'HEAD'), date: git(dir, 'log', '-1', '--format=%cI') };
};
const pkg = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
const buildInfo = {
  version: pkg.version,
  commit: git(root, 'rev-parse', '--short', 'HEAD'),
  coriolis: upstream('coriolis'),
  coriolisData: upstream('coriolis-data'),
  builtAt: new Date().toISOString(),
};

const entries = process.argv.slice(2).length ? process.argv.slice(2) : ['server', 'cli', 'oracle', 'testkit'];

await build({
  entryPoints: Object.fromEntries(entries.map((e) => [e, path.join(root, `src/${e}.ts`)])),
  outdir: path.join(root, 'dist'),
  outExtension: { '.js': '.cjs' },
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node20',
  loader: { '.js': 'jsx' },
  sourcemap: true,
  jsxFactory: 'React.createElement',
  jsxFragment: 'React.Fragment',
  alias: {
    react: stub('react.js'),
    'coriolis-data/dist': stub('coriolis-data.js'),
  },
  plugins: [{
    name: 'stub-browser-stores',
    setup(b) {
      b.onResolve({ filter: /stores\/Persist$/ }, () => ({ path: stub('persist.js') }));
    },
  }, {
    // Upstream calls sibling exports as `this.fn()`, relying on Babel binding `this`
    // to the module namespace. Rewrite to an explicit self-namespace import.
    name: 'namespace-this',
    setup(b) {
      b.onLoad({ filter: /shipyard[\\/](Calculations|ModuleUtils)\.js$/ }, async (args) => {
        const src = await readFile(args.path, 'utf8');
        const base = path.basename(args.path, '.js');
        const contents = `import * as __self from './${base}';\n` + src.replace(/\bthis\.(\w+)\(/g, '__self.$1(');
        return { contents, loader: 'jsx' };
      });
    },
  }],
  nodePaths: [path.join(root, 'node_modules')],
  define: { __CORAI_BUILD__: JSON.stringify(buildInfo) },
  logLevel: 'error', // upstream data has harmless duplicate-key warnings
});
console.log(`built ${entries.map((e) => `dist/${e}.cjs`).join(', ')}`);
