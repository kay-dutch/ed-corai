// Sends a few real tool calls through the Rust MCP server so the ED Corai window has something to
// show: an evaluation, a long optimisation (the station turns), a comparison and a failing call.
//   node scripts/window-demo.mjs [path/to/ed-corai-cli]   (default: target/debug/ed-corai-cli)
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const exe = process.argv[2] ?? `${root}target/debug/ed-corai-cli${process.platform === 'win32' ? '.exe' : ''}`;
const client = new Client({ name: 'window-demo', title: 'Window demo', version: '1' });
await client.connect(new StdioClientTransport({ command: exe, args: ['mcp', '--no-update-check'] }));
const call = async (name, args) => {
  process.stderr.write(`${name}…\n`);
  await client.callTool({ name, arguments: args }, undefined, { timeout: 600_000 });
};
await call('evaluate_build', { ship: 'python' });
await call('optimize_build', { ship: 'krait_mkii', objective: { metric: 'jumpLaden' }, effort: 'thorough' });
await call('compare_ships', { objective: { metric: 'cargo' }, size: 'medium', top: 3 });
await call('module_info', { group: 'nope' });
await new Promise((r) => setTimeout(r, 3000));
await client.close();
