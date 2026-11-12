//! Resolving ships and builds from tool arguments.

use corai_data::DATA;
use corai_engine::{Ship, interchange};

use crate::error::ServiceError;

/// Ship ids in alphabetical order.
pub fn ship_ids() -> Vec<&'static str> {
    let mut ids: Vec<&str> = DATA.ships.keys().map(String::as_str).collect();
    ids.sort_unstable();
    ids
}

/// A stock (factory default) ship by exact id.
///
/// # Errors
/// Unknown id.
pub fn stock_ship(id: &str) -> Result<Ship, ServiceError> {
    Ship::stock(id).map_err(|_| ServiceError::UnknownShipId(id.into()))
}

/// A ship id from an id or a display name, ignoring case and punctuation (`resolveShipId`).
///
/// # Errors
/// No ship matches.
pub fn resolve_ship_id(name: &str) -> Result<&'static str, ServiceError> {
    if let Some((id, _)) = DATA.ships.get_key_value(name) {
        return Ok(id.as_str());
    }
    let norm = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
    };
    let want = norm(name);
    ship_ids()
        .into_iter()
        .find(|id| id.replace('_', "") == want || norm(&DATA.ships[*id].properties.name) == want)
        .ok_or_else(|| ServiceError::UnknownShip(name.into()))
}

/// The starting ship of a tool call: a pasted build (with its import issues) or a stock hull.
///
/// # Errors
/// Neither given, unknown ship, or unreadable build.
pub fn load_base(
    ship: Option<&str>,
    build: Option<&str>,
) -> Result<(Ship, Vec<String>), ServiceError> {
    if let Some(build) = build.filter(|b| !b.is_empty()) {
        let hint = ship.map(resolve_ship_id).transpose()?;
        let imported = interchange::parse_build(build, hint)?;
        return Ok((imported.ship, imported.issues));
    }
    let ship = ship.ok_or(ServiceError::NoShip)?;
    Ok((stock_ship(resolve_ship_id(ship)?)?, Vec::new()))
}
