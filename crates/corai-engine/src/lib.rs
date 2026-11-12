//! Headless port of the Coriolis (coriolis.io) outfitting engine.
//!
//! Numbers are meant to match the upstream JavaScript exactly; the differential tests in
//! `tests/` compare against the reference TypeScript build of the original engine.

pub mod calc;
pub mod codec;
pub mod jsmath;
pub mod module;
pub mod module_set;
pub mod ship;

pub use module::{DamageDist, EngineeringError, Module};
pub use module_set::ModuleSet;
pub use ship::{STANDARD_GROUPS, Ship, ShipError, Slot, SlotKind, Stats};
