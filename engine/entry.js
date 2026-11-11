// Headless entry point into the upstream Coriolis shipyard engine.
// Bundled by scripts/build-engine.mjs with browser-only deps replaced by stubs.
export { default as Ship } from '../upstream/coriolis/src/app/shipyard/Ship';
export { default as Module } from '../upstream/coriolis/src/app/shipyard/Module';
export * as ModuleUtils from '../upstream/coriolis/src/app/shipyard/ModuleUtils';
export * as Calc from '../upstream/coriolis/src/app/shipyard/Calculations';
export * as Constants from '../upstream/coriolis/src/app/shipyard/Constants';
export * as Serializer from '../upstream/coriolis/src/app/shipyard/Serializer';
export * as ShipRoles from '../upstream/coriolis/src/app/shipyard/ShipRoles';
export * as BlueprintFunctions from '../upstream/coriolis/src/app/utils/BlueprintFunctions';
export { Ships, Modules, Modifications } from 'coriolis-data/dist';
export * as JournalUtils from '../upstream/coriolis/src/app/utils/JournalUtils';
export * as CompanionApiUtils from '../upstream/coriolis/src/app/utils/CompanionApiUtils';
export * as LinkMigration from '../upstream/coriolis/src/app/shipyard/LinkMigration';
