//! Port of Coriolis' `ModuleSet`: the modules its outfitting screen offers for each slot of a ship.

use corai_data::{DATA, ModuleDef};
use indexmap::IndexMap;

use crate::ship::{STANDARD_GROUPS, ShipError};

type Defs = Vec<&'static ModuleDef>;

/// `filter(arr, maxClass, minClass, mass)`: class range, and a max mass that carries `mass`.
fn filter<'a>(
    defs: impl IntoIterator<Item = &'a &'static ModuleDef>,
    max_class: u8,
    min_class: u8,
    mass: f64,
) -> Defs {
    defs.into_iter()
        .copied()
        .filter(|m| {
            m.class <= max_class
                && m.class >= min_class
                && m.stat("maxmass").is_none_or(|mm| mass <= mm)
        })
        .collect()
}

/// Modules restricted to other ships (`ship` field) are not offered.
fn allowed_on(m: &ModuleDef, ship_id: &str) -> bool {
    m.ship.is_empty() || m.ship.iter().any(|s| s == ship_id)
}

/// Unrecognised-module placeholders kept in the data for imports.
fn recognised_fuel_tank(m: &ModuleDef) -> bool {
    let id = m.id.to_lowercase();
    let name = m
        .name
        .as_deref()
        .or(m.uk_name.as_deref())
        .unwrap_or_default()
        .to_lowercase();
    !(id.is_empty()
        || name.contains("unrecognised")
        || id.contains("unrecognised")
        || id == "4m"
        || id == "1z"
        || id == "0z"
        || id.ends_with('z')
        || name.contains("error")
        || name.contains("placeholder")
        || m.rating == "Z")
}

/// What Coriolis offers in each slot of one ship.
#[derive(Debug, Clone)]
pub struct ModuleSet {
    ship_id: &'static str,
    mass: f64,
    luxury_cabins: bool,
    fighter_hangars: bool,
    vessel_hangars_mk_ii: bool,
    vehicle_hangars_mk_ii: bool,
    standard: [Defs; 7],
    internal: IndexMap<&'static str, Defs>,
    hardpoints: IndexMap<&'static str, Defs>,
}

impl ModuleSet {
    /// `ModuleUtils.forShip(shipId)`.
    ///
    /// # Errors
    /// Unknown ship id.
    pub fn for_ship(ship_id: &str) -> Result<Self, ShipError> {
        let data = &*DATA;
        let (id, ship) = data
            .ships
            .get_key_value(ship_id)
            .ok_or_else(|| ShipError::UnknownShip(ship_id.into()))?;
        let p = &ship.properties;
        let mass = p.hull_mass + 6.5;
        let max_std: Vec<u8> = ship.slots.standard.iter().map(|s| s.class()).collect();
        let max_internal = ship
            .slots
            .internal
            .iter()
            .map(|s| s.class())
            .max()
            .unwrap_or(0);
        let max_hardpoint = ship
            .slots
            .hardpoints
            .iter()
            .map(|s| s.class())
            .max()
            .unwrap_or(0);
        let std = |grp: &str| {
            data.modules
                .standard
                .get(grp)
                .map(|g| g.iter().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let max = |i: usize| max_std.get(i).copied().unwrap_or(0);

        let pp = filter(&std("pp"), max(0), 0, mass);
        let fsd = filter(&std("fsd"), max(2), 0, mass)
            .into_iter()
            // SCO drives fit only a slot of exactly their class.
            .filter(|m| {
                !m.name.as_deref().is_some_and(|n| n.contains("(SCO)")) || m.class == max(2)
            })
            .filter(|m| allowed_on(m, id))
            .collect();
        let pd = filter(&std("pd"), max(4), 0, mass);
        let ft = filter(&std("ft"), max(6), 0, mass)
            .into_iter()
            .filter(|m| recognised_fuel_tank(m))
            .collect();
        // Thrusters: from the smallest class able to move the hull up to the slot class.
        let min_thruster = std("t")
            .iter()
            .filter(|t| t.stat("maxmass").is_some_and(|mm| mm >= mass))
            .map(|t| t.class)
            .fold(max(1), u8::min);
        let t = filter(&std("t"), max(1), min_thruster, 0.0)
            .into_iter()
            .filter(|m| allowed_on(m, id) && m.rating != "Z")
            .collect();
        // Life support and sensors must match the slot class.
        let ls = filter(&std("ls"), max(3), max(3), 0.0);
        let s = filter(&std("s"), max(5), max(5), mass);

        let by_group = |groups: &'static IndexMap<String, Vec<ModuleDef>>, max_class: u8| {
            groups
                .iter()
                .map(|(g, ms)| {
                    (
                        g.as_str(),
                        filter(&ms.iter().collect::<Vec<_>>(), max_class, 0, mass),
                    )
                })
                .collect()
        };
        Ok(Self {
            ship_id: id.as_str(),
            mass,
            luxury_cabins: p.luxury_cabins,
            fighter_hangars: p.fighter_hangars,
            vessel_hangars_mk_ii: p.vessel_hangars_mk_ii,
            vehicle_hangars_mk_ii: p.vehicle_hangars_mk_ii,
            standard: [pp, t, fsd, ls, pd, s, ft],
            internal: by_group(&data.modules.internal, max_internal),
            hardpoints: by_group(&data.modules.hardpoints, max_hardpoint),
        })
    }

    /// Modules offered in standard slot `index` (`STANDARD_GROUPS` order).
    pub fn standard(&self, index: usize) -> &[&'static ModuleDef] {
        self.standard.get(index).map_or(&[], Vec::as_slice)
    }

    /// The standard group of slot `index`.
    pub fn standard_group(index: usize) -> Option<&'static str> {
        STANDARD_GROUPS.get(index).copied()
    }

    /// `getInts(ship, class, eligible)`: modules offered in an internal slot.
    pub fn internal(
        &self,
        class: u8,
        eligible: Option<&IndexMap<String, u8>>,
    ) -> Vec<&'static ModuleDef> {
        self.internal
            .iter()
            .filter(|(g, _)| eligible.is_none_or(|e| e.get(**g).is_some_and(|v| *v != 0)))
            .filter(|(g, _)| match **g {
                // Large cargo racks only go into the special Cargo slots.
                "crl" => eligible.is_some(),
                "pcq" => self.luxury_cabins,
                "fh" => self.fighter_hangars,
                "fhmkii" => self.vessel_hangars_mk_ii,
                "lpvmkii" => self.vehicle_hangars_mk_ii,
                _ => true,
            })
            .flat_map(|(_, ms)| filter(ms, class, 0, self.mass))
            .collect()
    }

    /// `getHps(ship, class, eligible, slot)`: modules offered in a hardpoint (class > 0) or
    /// utility mount (class 0). `mvr` is the slot's Mining Volley Repeater opt-in.
    pub fn hardpoints(
        &self,
        class: u8,
        eligible: Option<&IndexMap<String, u8>>,
        mvr: bool,
    ) -> Vec<&'static ModuleDef> {
        self.hardpoints
            .iter()
            .filter(|(g, _)| eligible.is_none_or(|e| e.get(**g).is_some_and(|v| *v != 0)))
            // The Anaconda Mk II takes the volley repeater only on its two opted-in hardpoints.
            .filter(|(g, _)| **g != "mvr" || self.ship_id != "anaconda2" || mvr)
            .flat_map(|(_, ms)| filter(ms, class, u8::from(class > 0), self.mass))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(defs: &[&ModuleDef]) -> Vec<String> {
        defs.iter().map(|m| format!("{}/{}", m.grp, m.id)).collect()
    }

    #[test]
    fn life_support_and_sensors_match_the_slot_class() {
        let set = ModuleSet::for_ship("anaconda").expect("anaconda");
        assert!(set.standard(3).iter().all(|m| m.class == 5));
        assert!(set.standard(5).iter().all(|m| m.class == 8));
        assert!(!set.standard(3).is_empty() && !set.standard(5).is_empty());
    }

    #[test]
    fn hull_mass_limits_shield_generators() {
        let set = ModuleSet::for_ship("anaconda").expect("anaconda");
        let offered = set.internal(8, None);
        let mass = DATA.ships["anaconda"].properties.hull_mass + 6.5;
        let sg: Vec<_> = offered.iter().filter(|m| m.grp == "sg").collect();
        assert!(!sg.is_empty());
        assert!(
            sg.iter()
                .all(|m| m.stat("maxmass").is_none_or(|mm| mm >= mass))
        );
    }

    #[test]
    fn mk_ii_hangars_need_the_ship_flag() {
        for (id, ship) in &DATA.ships {
            let set = ModuleSet::for_ship(id).expect("ship");
            let has = ids(&set.internal(8, None))
                .iter()
                .any(|g| g.starts_with("fhmkii/"));
            assert_eq!(has, ship.properties.vessel_hangars_mk_ii, "{id}");
        }
    }

    #[test]
    fn volley_repeater_only_on_flagged_anaconda_mk_ii_hardpoints() {
        let set = ModuleSet::for_ship("anaconda2").expect("anaconda2");
        let offered = |mvr| {
            ids(&set.hardpoints(3, None, mvr))
                .iter()
                .any(|g| g.starts_with("mvr/"))
        };
        assert!(offered(true));
        assert!(!offered(false));
    }
}
