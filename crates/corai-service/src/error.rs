//! Errors the tools report. Messages match the Node server word for word.

use corai_engine::interchange::InterchangeError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    #[error("Unknown ship \"{0}\". Use list_ships.")]
    UnknownShip(String),
    #[error("Unknown ship \"{0}\". Use list_ships to see valid ids.")]
    UnknownShipId(String),
    #[error("Pass either ship (id/name) or build (Coriolis URL).")]
    NoShip,
    #[error("Unknown group \"{0}\". Use list_module_groups.")]
    UnknownGroup(String),
    #[error("Unknown module group \"{0}\".")]
    UnknownSearchGroup(String),
    #[error("Unknown metric \"{0}\". Use list_metrics.")]
    UnknownMetric(String),
    #[error("Unknown module group \"{group}\" in metric \"{metric}\". Use list_module_groups.")]
    UnknownMetricGroup { metric: String, group: String },
    #[error("Unknown slot \"{slot}\" for {ship}. Use ship_details to list slots.")]
    UnknownSlot { slot: String, ship: String },
    #[error("No module matching {change} fits {slot}.")]
    NoModuleFits { change: String, slot: String },
    #[error("{0} is empty; give moduleId or group.")]
    EmptySlot(String),
    #[error("Unknown experimental effect \"{0}\".")]
    UnknownSpecial(String),
    #[error("Unknown blueprint \"{0}\".")]
    UnknownBlueprint(String),
    #[error("Blueprint {blueprint} has no grade {grade}.")]
    UnknownGrade { blueprint: String, grade: u8 },
    #[error("Unknown damage preset \"{0}\" (even, pve).")]
    UnknownPreset(String),
    #[error(transparent)]
    Import(#[from] InterchangeError),
}
