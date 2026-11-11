// Release checks against GitHub and in-place updates of a git-cloned install.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { BUILD, compareVersions, type BuildInfo } from './version.js';

export interface ReleaseInfo {
  tag: string;
  version: string;
  name: string;
  url: string;
  publishedAt: string;
  notes: string;
}

export interface UpdateStatus {
  current: BuildInfo;
  installDir: string;
  repo?: string;
  latest?: ReleaseInfo;
  updateAvailable: boolean;
  canAutoInstall: boolean;
  message: string;
}

/** The repository root of this install (dist/*.cjs sits one level below it). */
export function installDir(): string {
  return path.resolve(__dirname, '..');
}

function run(cmd: string, args: string[], cwd: string, timeoutMs = 120_000): string {
  // Windows refuses to spawn .cmd files without a shell.
  const shell = process.platform === 'win32' && /\.cmd$/i.test(cmd);
  return execFileSync(shell ? `"${cmd}"` : cmd, args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: timeoutMs, windowsHide: true, shell }).trim();
}

function gitOut(dir: string, ...args: string[]): string | undefined {
  try { return run('git', args, dir, 15_000); } catch { return undefined; }
}

/** "owner/name" from a GitHub URL in any of the usual forms. */
export function parseGithubSlug(url: string | undefined): string | undefined {
  const m = url?.trim().match(/github\.com[/:]([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/i);
  return m ? `${m[1]}/${m[2]}` : undefined;
}

/** GitHub repository to check: CORAI_REPO, else the clone's origin, else package.json repository. */
export function repoSlug(dir = installDir()): string | undefined {
  const env = process.env.CORAI_REPO?.trim();
  if (env) return parseGithubSlug(env) ?? (/^[\w.-]+\/[\w.-]+$/.test(env) ? env : undefined);
  const origin = parseGithubSlug(gitOut(dir, 'remote', 'get-url', 'origin'));
  if (origin) return origin;
  try {
    const pkg = JSON.parse(readFileSync(path.join(dir, 'package.json'), 'utf8'));
    return parseGithubSlug(typeof pkg.repository === 'string' ? pkg.repository : pkg.repository?.url);
  } catch {
    return undefined;
  }
}

export async function latestRelease(repo: string, timeoutMs = 8_000): Promise<ReleaseInfo> {
  const headers: Record<string, string> = { Accept: 'application/vnd.github+json', 'User-Agent': 'ed-corai-update-check' };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, { headers, signal: AbortSignal.timeout(timeoutMs) });
  if (res.status === 404) throw new Error(`No published releases found for ${repo}.`);
  if (!res.ok) throw new Error(`GitHub answered ${res.status} for ${repo}.`);
  const r: any = await res.json();
  const notes = String(r.body ?? '');
  return {
    tag: r.tag_name,
    version: String(r.tag_name).replace(/^v/i, ''),
    name: r.name || r.tag_name,
    url: r.html_url,
    publishedAt: r.published_at,
    notes: notes.length > 3000 ? notes.slice(0, 3000) + '\n…' : notes,
  };
}

function autoInstallBlocker(dir: string): string | undefined {
  if (!existsSync(path.join(dir, '.git'))) return 'This install is not a git clone; download the release and rebuild instead.';
  const dirty = gitOut(dir, 'status', '--porcelain', '--untracked-files=no');
  if (dirty === undefined) return 'git is not available.';
  if (dirty) return 'The install has local changes to tracked files; commit or stash them first.';
  return undefined;
}

export async function checkForUpdates(): Promise<UpdateStatus> {
  const dir = installDir();
  const repo = repoSlug(dir);
  const base = { current: BUILD, installDir: dir, repo, updateAvailable: false, canAutoInstall: false };
  if (!repo) {
    return { ...base, message: 'No GitHub repository known for this install. Set CORAI_REPO=owner/name or add an origin remote.' };
  }
  let latest: ReleaseInfo;
  try {
    latest = await latestRelease(repo);
  } catch (e) {
    return { ...base, message: `Could not check for updates: ${e instanceof Error ? e.message : String(e)}` };
  }
  const updateAvailable = compareVersions(latest.version, BUILD.version) > 0;
  const blocker = updateAvailable ? autoInstallBlocker(dir) : undefined;
  const message = !updateAvailable
    ? `ED Corai ${BUILD.version} is up to date (latest release ${latest.tag}).`
    : blocker
      ? `ED Corai ${latest.version} is available (installed ${BUILD.version}). Automatic install is not possible: ${blocker}`
      : `ED Corai ${latest.version} is available (installed ${BUILD.version}). It can be installed automatically; restart the MCP server afterwards.`;
  return { ...base, latest, updateAvailable, canAutoInstall: updateAvailable && !blocker, message };
}

function npmCommand(): string {
  const exe = process.platform === 'win32' ? 'npm.cmd' : 'npm';
  // MCP clients often start the server with a minimal PATH; prefer the npm next to this node.
  const sibling = path.join(path.dirname(process.execPath), exe);
  return existsSync(sibling) ? sibling : exe;
}

/**
 * Moves a clean git clone to the release tag, refreshes the Coriolis submodules, installs
 * dependencies and rebuilds. The running server keeps the old code until it is restarted.
 */
export async function installUpdate(tag?: string): Promise<{ installed: string; from: string; log: string[]; restartRequired: true }> {
  const dir = installDir();
  const blocker = autoInstallBlocker(dir);
  if (blocker) throw new Error(blocker);
  const repo = repoSlug(dir);
  if (!tag) {
    if (!repo) throw new Error('No GitHub repository known for this install; pass a tag or set CORAI_REPO.');
    tag = (await latestRelease(repo)).tag;
  }
  if (!/^[\w.+-]+$/.test(tag)) throw new Error(`Invalid tag "${tag}".`);
  const log: string[] = [];
  const step = (cmd: string, args: string[], timeoutMs?: number) => {
    log.push(`$ ${path.basename(cmd)} ${args.join(' ')}`);
    const out = run(cmd, args, dir, timeoutMs);
    if (out) log.push(out.split('\n').slice(-5).join('\n'));
  };
  step('git', ['fetch', '--tags', 'origin']);
  if (gitOut(dir, 'rev-parse', '--verify', '--quiet', `refs/tags/${tag}`) === undefined) throw new Error(`Release tag ${tag} not found in origin.`);
  const branch = gitOut(dir, 'symbolic-ref', '--quiet', '--short', 'HEAD');
  const fastForward = gitOut(dir, 'merge-base', '--is-ancestor', 'HEAD', tag) !== undefined;
  if (branch && fastForward) step('git', ['merge', '--ff-only', tag]);
  else if (!branch) step('git', ['checkout', '--quiet', tag]);
  else throw new Error(`Branch ${branch} has commits that are not in ${tag}; update manually.`);
  step('git', ['submodule', 'update', '--init', '--recursive'], 600_000);
  const npm = npmCommand();
  step(npm, [existsSync(path.join(dir, 'package-lock.json')) ? 'ci' : 'install', '--no-audit', '--no-fund'], 600_000);
  step(npm, ['run', 'build'], 600_000);
  return { installed: tag, from: BUILD.version, log, restartRequired: true };
}
