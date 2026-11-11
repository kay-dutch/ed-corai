// Compact, model-friendly summaries of a build.
import { Calc, coriolisUrl, engineeringLabel, listSlots, moduleLabel, shipData, type Ship } from './engine.js';
import { computeAll, type MetricContext } from './metrics.js';
import { edsyUrl } from './interchange.js';
import { validateBuild } from './validate.js';

export interface BuildSummary {
  ship: string;
  shipId: string;
  coriolisUrl: string;
  edsyUrl: string;
  modules: { slot: string; size: number | string; module: string; engineering?: string; power?: number; mass?: number }[];
  metrics: Record<string, number>;
  checks: { powerOk: boolean; powerOkRetracted: boolean; canThrustLaden: boolean; canBoost: boolean };
  /** Modules or engineering Coriolis itself would not allow (empty for a legal build). */
  issues: string[];
  /** Resistances in % as the outfitting panel shows them (0 SYS pips, diminishing returns applied). */
  resistances: { shield?: { kinetic: number; thermal: number; explosive: number }; hull: { kinetic: number; thermal: number; explosive: number } };
}

export function summarize(ship: Ship, ctx: MetricContext, opts: { includeEmpty?: boolean; buildName?: string } = {}): BuildSummary {
  const modules: BuildSummary['modules'] = [];
  for (const ref of listSlots(ship)) {
    const m = ref.slot.m;
    if (!m && !opts.includeEmpty) continue;
    const size = ref.kind === 'bulkheads' ? '-' : ref.kind === 'utility' ? 'U' : `${ref.maxClass}${ref.name ? ` (${ref.name})` : ''}`;
    const row: BuildSummary['modules'][number] = {
      slot: ref.key,
      size: size as any,
      module: ref.kind === 'bulkheads' ? (m?.name ?? shipData(ship.id).bulkheads[0].name) : moduleLabel(m),
    };
    const eng = engineeringLabel(m);
    if (eng) row.engineering = eng;
    const power = m?.getPowerUsage?.();
    const mass = m?.getMass?.();
    if (typeof power === 'number' && power) row.power = Number(power.toFixed(2));
    if (typeof mass === 'number') row.mass = Number(mass.toFixed(2));
    modules.push(row);
  }
  return {
    ship: ship.name,
    shipId: ship.id,
    coriolisUrl: coriolisUrl(ship, opts.buildName),
    edsyUrl: edsyUrl(ship, opts.buildName),
    modules,
    metrics: computeAll(ship, ctx),
    checks: {
      powerOk: ship.powerDeployed <= ship.powerAvailable,
      powerOkRetracted: ship.powerRetracted <= ship.powerAvailable,
      canThrustLaden: ship.ladenMass < (ship.standard[1].m?.getMaxMass() ?? 0),
      canBoost: ship.canBoost(ship.cargoCapacity, ship.fuelCapacity),
    },
    issues: [...validateBuild(ship), ...(ship.importIssues ?? []).map((i: string) => `import: ${i}`)],
    resistances: resistances(ship),
  };
}

function resistances(ship: Ship): BuildSummary['resistances'] {
  const pct = (mult: number) => Math.round((1 - mult) * 1000) / 10;
  const s = Calc.shieldMetrics(ship, 0);
  const a = Calc.armourMetrics(ship);
  return {
    shield: s.generator ? { kinetic: pct(s.kinetic.total), thermal: pct(s.thermal.total), explosive: pct(s.explosive.total) } : undefined,
    hull: { kinetic: pct(a.kinetic.total), thermal: pct(a.thermal.total), explosive: pct(a.explosive.total) },
  };
}
