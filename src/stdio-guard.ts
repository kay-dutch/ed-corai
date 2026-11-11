// MCP speaks JSON-RPC over stdout; the upstream engine logs debug output with console.log
// (thousands of lines per optimisation). Silence chatter unless CORAI_DEBUG is set, and send
// whatever remains to stderr so it can never corrupt the protocol stream.
const toStderr = (...args: unknown[]) => process.stderr.write(args.map(String).join(' ') + '\n');
const quiet = () => {};
const debug = !!process.env.CORAI_DEBUG;
console.log = debug ? toStderr : quiet;
console.info = debug ? toStderr : quiet;
console.debug = debug ? toStderr : quiet;
console.warn = toStderr;
console.error = toStderr;
