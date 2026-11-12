// The upstream Coriolis engine is untyped JavaScript; it is consumed through
// the thin typed layer in src/engine.ts.
export const Ship: any;
export const Module: any;
export const ModuleUtils: any;
export const Calc: any;
export const Constants: any;
export const Serializer: any;
export const ShipRoles: any;
export const BlueprintFunctions: any;
export const Ships: Record<string, any>;
export const Modules: { standard: Record<string, any[]>; hardpoints: Record<string, any[]>; internal: Record<string, any[]> };
export const Modifications: {
  blueprints: Record<string, any>;
  modifications: Record<string, any>;
  modifierActions: Record<string, any>;
  modules: Record<string, { blueprints: Record<string, any>; modifications: string[]; specials?: string[] }>;
  specials: Record<string, any>;
};
export const JournalUtils: { shipFromLoadoutJSON(json: unknown): any };
export const CompanionApiUtils: {
  SHIP_FD_NAME_TO_CORIOLIS_NAME: Record<string, string>;
  shipSlots?(shipId: string, group: 'internal' | 'hardpoints' | 'standard'): { class: number; name: string }[] | null;
};
export const LinkMigration: { OLD_INTERNAL_LAYOUTS: Record<string, { kind: string; class: number }[]>; OLD_HARDPOINT_LAYOUTS: Record<string, number[]> };
