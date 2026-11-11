// Version of this build and of the Coriolis code and data compiled into it (set by scripts/build.mjs).

export interface UpstreamInfo { commit?: string; date?: string }
export interface BuildInfo {
  version: string;
  commit?: string;
  coriolis: UpstreamInfo;
  coriolisData: UpstreamInfo;
  builtAt?: string;
}

declare const __CORAI_BUILD__: BuildInfo | undefined;

export const BUILD: BuildInfo = typeof __CORAI_BUILD__ !== 'undefined'
  ? __CORAI_BUILD__
  : { version: '0.0.0-dev', coriolis: {}, coriolisData: {} };

/** Compares two semantic versions ("v" prefix allowed). Pre-releases sort before their release. */
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => {
    const [core, pre = ''] = v.trim().replace(/^v/i, '').split('+')[0].split(/-(.*)/s);
    return { nums: core.split('.').map((x) => Number.parseInt(x, 10) || 0), pre };
  };
  const x = parse(a), y = parse(b);
  for (let i = 0; i < Math.max(x.nums.length, y.nums.length, 3); i++) {
    const d = (x.nums[i] ?? 0) - (y.nums[i] ?? 0);
    if (d) return Math.sign(d);
  }
  if (x.pre === y.pre) return 0;
  if (!x.pre) return 1;
  if (!y.pre) return -1;
  return x.pre.localeCompare(y.pre, 'en', { numeric: true }) > 0 ? 1 : -1;
}
