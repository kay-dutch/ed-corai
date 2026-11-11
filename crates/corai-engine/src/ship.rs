//! A ship and its outfitting: slots, module placement and aggregate stats (port of `Ship.js`).

use corai_data::{DATA, ModuleDef, ShipDef, SlotSpec};
use indexmap::IndexMap;

use crate::calc;
use crate::jsmath::{num, truthy};
use crate::module::{Module, Resolved};

/// Standard slot order used throughout Coriolis.
pub const STANDARD_GROUPS: [&str; 7] = ["pp", "t", "fsd", "ls", "pd", "s", "ft"];
/// The cargo hatch draws 0.6 MW (retracted, priority 0) unless disabled.
const CARGO_HATCH_POWER: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotKind {
    Standard,
    Hardpoint,
    Utility,
    Internal,
}

#[derive(Debug, Clone)]
pub struct Slot {
    pub kind: SlotKind,
    pub max_class: u8,
    /// Restriction label, e.g. `Military`, `Cargo`, `PlanetaryApproachSuite`.
    pub name: Option<&'static str>,
    pub eligible: Option<&'static IndexMap<String, u8>>,
    pub module: Option<Module>,
    pub enabled: bool,
    /// Power priority band 0..=4.
    pub priority: usize,
}

impl Slot {
    fn new(kind: SlotKind, spec: &'static SlotSpec) -> Self {
        Self {
            kind,
            max_class: spec.class(),
            name: spec.name(),
            eligible: spec.eligible(),
            module: None,
            enabled: true,
            priority: 0,
        }
    }

    fn is_hardpoint_mount(&self) -> bool {
        matches!(self.kind, SlotKind::Hardpoint | SlotKind::Utility)
    }

    /// `powerUsageType`: hardpoint and utility mounts draw power only when deployed,
    /// unless the module is passive; everything else always draws.
    fn draws_when_retracted(&self, m: &Module) -> bool {
        m.def.is_passive() || !self.is_hardpoint_mount()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PowerBand {
    pub retracted: f64,
    pub deployed: f64,
    pub retracted_sum: f64,
    pub deployed_sum: f64,
}

/// Aggregates recomputed by [`Ship::recalc`]. Names follow Coriolis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub power_available: f64,
    pub power_retracted: f64,
    pub power_deployed: f64,
    pub bands: [PowerBand; 5],
    /// Hull + modules, no fuel or cargo.
    pub dry_mass: f64,
    /// Coriolis' `unladenMass`: dry mass **plus full fuel**.
    pub unladen_mass: f64,
    pub laden_mass: f64,
    pub fuel_capacity: f64,
    pub cargo_capacity: f64,
    pub passenger_capacity: f64,
    pub total_cost: f64,
    pub speeds: [f64; 5],
    pub top_speed: f64,
    pub top_boost: f64,
    pub top_pitch: f64,
    pub top_roll: f64,
    pub top_yaw: f64,
    /// generator + boosters + guardian reinforcement (0 without a shield).
    pub shield: f64,
    pub shield_cells: f64,
    pub armour: f64,
    pub total_dps: f64,
    pub total_sdps: f64,
    pub total_eps: f64,
    pub total_hps: f64,
}

#[derive(Debug, Clone)]
pub struct Ship {
    pub id: &'static str,
    pub def: &'static ShipDef,
    pub standard: Vec<Slot>,
    pub hardpoints: Vec<Slot>,
    pub internal: Vec<Slot>,
    pub bulkhead: Module,
    pub bulkhead_index: usize,
    pub hatch_enabled: bool,
    pub hatch_priority: usize,
    pub stats: Stats,
}

/// Errors building or changing a ship.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ShipError {
    #[error("unknown ship {0}")]
    UnknownShip(String),
    #[error("unknown module {0}")]
    UnknownModule(String),
    #[error("ship has no bulkhead #{0}")]
    UnknownBulkhead(usize),
}

// ---------------------------------------------------------------- module lookup (ModuleUtils)

/// `ModuleUtils.standard(index, id)`: by id, else by class+rating code such as `"5A"`.
pub fn standard_module(index: usize, code: &str) -> Option<&'static ModuleDef> {
    let data = &*DATA;
    let group = data.modules.standard.get(*STANDARD_GROUPS.get(index)?)?;
    group.iter().find(|m| m.id == code).or_else(|| {
        let mut chars = code.chars();
        let (class, rating) = (chars.next()?, chars.next()?);
        group
            .iter()
            .find(|m| m.class.to_string() == class.to_string() && m.rating == rating.to_string())
    })
}

/// `ModuleUtils.findModule(grp, id)`: looks in standard, then internal, then hardpoint groups.
pub fn find_module(grp: &str, id: &str) -> Option<&'static ModuleDef> {
    let data = &*DATA;
    [
        &data.modules.standard,
        &data.modules.internal,
        &data.modules.hardpoints,
    ]
    .into_iter()
    .find_map(|groups| groups.get(grp).and_then(|g| g.iter().find(|m| m.id == id)))
}

/// `ModuleUtils.hardpoints(id)`: first module with that id across hardpoint groups.
pub fn hardpoint_module(id: &str) -> Option<&'static ModuleDef> {
    let data = &*DATA;
    data.modules
        .hardpoints
        .values()
        .flatten()
        .find(|m| m.id == id)
}

/// `ModuleUtils.internal(id)`: first module with that id across internal groups.
pub fn internal_module(id: &str) -> Option<&'static ModuleDef> {
    let data = &*DATA;
    data.modules
        .internal
        .values()
        .flatten()
        .find(|m| m.id == id)
}

impl Ship {
    /// An empty hull (lightweight bulkheads, no modules).
    ///
    /// # Errors
    /// Unknown ship id.
    pub fn empty(id: &str) -> Result<Self, ShipError> {
        let data = &*DATA;
        let (id, def) = data
            .ships
            .get_key_value(id)
            .ok_or_else(|| ShipError::UnknownShip(id.into()))?;
        let bulkhead = Module::new(def.bulkheads.first().ok_or(ShipError::UnknownBulkhead(0))?);
        let mut ship = Self {
            id: id.as_str(),
            def,
            standard: def
                .slots
                .standard
                .iter()
                .map(|s| Slot::new(SlotKind::Standard, s))
                .collect(),
            hardpoints: def
                .slots
                .hardpoints
                .iter()
                .map(|s| {
                    Slot::new(
                        if s.class() == 0 {
                            SlotKind::Utility
                        } else {
                            SlotKind::Hardpoint
                        },
                        s,
                    )
                })
                .collect(),
            internal: def
                .slots
                .internal
                .iter()
                .map(|s| Slot::new(SlotKind::Internal, s))
                .collect(),
            bulkhead,
            bulkhead_index: 0,
            hatch_enabled: true,
            hatch_priority: 0,
            stats: Stats::default(),
        };
        ship.recalc();
        Ok(ship)
    }

    /// The factory build (`buildWith(defaults)`).
    ///
    /// # Errors
    /// Unknown ship id or inconsistent default data.
    pub fn stock(id: &str) -> Result<Self, ShipError> {
        let mut ship = Self::empty(id)?;
        let defaults = &ship.def.defaults;
        for (i, code) in defaults.standard.iter().enumerate() {
            let def =
                standard_module(i, code).ok_or_else(|| ShipError::UnknownModule(code.clone()))?;
            ship.standard[i].module = Some(Module::new(def));
        }
        for (i, id) in defaults.hardpoints.iter().enumerate() {
            if let Some(id) = id {
                let def =
                    hardpoint_module(id).ok_or_else(|| ShipError::UnknownModule(id.clone()))?;
                ship.hardpoints[i].module = Some(Module::new(def));
            }
        }
        for (i, id) in defaults.internal.iter().enumerate() {
            if let Some(id) = id {
                let def =
                    internal_module(id).ok_or_else(|| ShipError::UnknownModule(id.clone()))?;
                ship.internal[i].module = Some(Module::new(def));
            }
        }
        ship.recalc();
        Ok(ship)
    }

    pub fn slots(&self, kind: SlotKind) -> impl Iterator<Item = &Slot> {
        let group = match kind {
            SlotKind::Standard => &self.standard,
            SlotKind::Internal => &self.internal,
            SlotKind::Hardpoint | SlotKind::Utility => &self.hardpoints,
        };
        group.iter().filter(move |s| s.kind == kind)
    }

    /// All slots in Coriolis serialisation order: standard, hardpoints (incl. utility), internal.
    pub fn all_slots(&self) -> impl Iterator<Item = &Slot> {
        self.standard
            .iter()
            .chain(&self.hardpoints)
            .chain(&self.internal)
    }

    /// `use(slot, module)` for a fresh module: pre-engineered modules get their fixed engineering.
    pub fn place(slot: &mut Slot, def: Option<&'static ModuleDef>) {
        slot.module = def.map(|d| {
            let mut m = Module::new(d);
            m.init_pre_engineered();
            m
        });
        slot.enabled = true;
    }

    /// `useBulkhead(index)`.
    ///
    /// # Errors
    /// Index out of range.
    pub fn use_bulkhead(&mut self, index: usize) -> Result<(), ShipError> {
        let def = self
            .def
            .bulkheads
            .get(index)
            .ok_or(ShipError::UnknownBulkhead(index))?;
        self.bulkhead = Module::new(def);
        self.bulkhead_index = index;
        Ok(())
    }

    /// `findInternalByGroup`: any shield type matches `sg`/`bsg`/`psg`.
    pub fn find_internal_by_group(&self, group: &str) -> Option<&Slot> {
        let shield = matches!(group, "sg" | "bsg" | "psg");
        self.internal.iter().find(|s| {
            s.module.as_ref().is_some_and(|m| {
                if shield {
                    m.is_shield_generator()
                } else {
                    m.grp() == group
                }
            })
        })
    }

    /// `getSlotStatus`: 0 none, 1 disabled, 2 offline (insufficient power), 3 online.
    pub fn slot_status(&self, slot: &Slot, deployed: bool) -> u8 {
        let Some(m) = &slot.module else { return 0 };
        if !slot.enabled {
            return 1;
        }
        let band = &self.stats.bands[slot.priority];
        if deployed {
            return if band.deployed_sum >= self.stats.power_available {
                2
            } else {
                3
            };
        }
        if slot.is_hardpoint_mount() && !m.def.is_passive() {
            return 0;
        }
        if band.retracted_sum >= self.stats.power_available {
            2
        } else {
            3
        }
    }

    pub fn thrusters(&self) -> Option<&Module> {
        self.standard[1].module.as_ref()
    }

    pub fn fsd(&self) -> Option<&Module> {
        self.standard[2].module.as_ref()
    }

    pub fn power_distributor(&self) -> Option<&Module> {
        self.standard[4].module.as_ref()
    }

    /// `canThrust(cargo, fuel)`: thrusters powered and under their max mass.
    pub fn can_thrust(&self, cargo: f64, fuel: f64) -> bool {
        let Some(t) = self.thrusters() else {
            return false;
        };
        self.slot_status(&self.standard[1], false) == 3
            && self.stats.dry_mass + cargo + fuel < num(t.r().max_mass)
    }

    /// `canBoost(cargo, fuel)`: can thrust and the distributor's ENG capacitor covers a boost.
    pub fn can_boost(&self, cargo: f64, fuel: f64) -> bool {
        self.power_distributor().is_some_and(|pd| {
            self.can_thrust(cargo, fuel) && num(pd.r().engcap) >= self.def.properties.boost_energy
        })
    }

    /// Recomputes every aggregate, in the order `buildWith` does.
    pub fn recalc(&mut self) {
        self.update_power_generated();
        self.update_power_used();
        self.recalculate_mass();
        self.stats.shield_cells = self.shield_cells();
        self.stats.shield =
            calc::shield_metrics(self, 0.0).map_or(0.0, |m| m.generator + m.boosters + m.addition);
        let armour = calc::armour_metrics(self);
        self.stats.armour = if truthy(Some(armour.total)) {
            armour.total
        } else {
            0.0
        };
        self.recalculate_weapons();
        self.update_movement();
        self.stats.total_cost = self.total_cost();
    }

    fn update_power_generated(&mut self) {
        self.stats.power_available = self.standard[0]
            .module
            .as_ref()
            .map_or(0.0, |m| num(m.r().pgen));
    }

    fn update_power_used(&mut self) {
        let mut bands = [PowerBand::default(); 5];
        if self.hatch_enabled {
            bands[self.hatch_priority].retracted += CARGO_HATCH_POWER;
        }
        for slot in self
            .standard
            .iter()
            .chain(&self.internal)
            .chain(&self.hardpoints)
        {
            let Some(m) = slot.module.as_ref().filter(|_| slot.enabled) else {
                continue;
            };
            let power = num(m.r().power);
            let band = &mut bands[slot.priority];
            if slot.draws_when_retracted(m) {
                band.retracted += power
            } else {
                band.deployed += power
            }
        }
        let (mut retracted, mut deployed) = (0.0, 0.0);
        for band in &mut bands {
            retracted += band.retracted;
            deployed += band.deployed + band.retracted;
            band.retracted_sum = retracted;
            band.deployed_sum = deployed;
        }
        self.stats.power_retracted = retracted;
        self.stats.power_deployed = deployed;
        self.stats.bands = bands;
    }

    fn recalculate_mass(&mut self) {
        // Same order as Coriolis (standard, internal, hardpoints): float sums decide edge cases
        // such as a ship sitting exactly at its thrusters' max mass.
        let modules: Vec<&Module> = self
            .standard
            .iter()
            .chain(&self.internal)
            .chain(&self.hardpoints)
            .filter_map(|s| s.module.as_ref())
            .collect();
        let sum = |stat: fn(&Resolved) -> Option<f64>| {
            modules.iter().map(|m| num(stat(m.r()))).sum::<f64>()
        };
        let dry = self.def.properties.hull_mass + num(self.bulkhead.r().mass) + sum(|r| r.mass);
        let fuel = sum(|r| r.fuel);
        let cargo: f64 = modules
            .iter()
            .map(|m| m.r().cargo.filter(|c| *c != 0.0).map_or(0.0, f64::floor))
            .sum();
        let passengers = sum(|r| r.passengers);
        let s = &mut self.stats;
        s.dry_mass = dry;
        s.fuel_capacity = fuel;
        s.cargo_capacity = cargo;
        s.passenger_capacity = passengers;
        s.unladen_mass = dry + fuel;
        s.laden_mass = dry + fuel + cargo;
    }

    fn shield_cells(&self) -> f64 {
        self.internal
            .iter()
            .filter_map(|s| s.module.as_ref())
            .filter(|m| m.grp() == "scb")
            .map(|m| num(m.r().shieldreinforcement) * num(m.r().duration) * (num(m.r().ammo) + 1.0))
            .sum()
    }

    fn recalculate_weapons(&mut self) {
        let (mut dps, mut sdps, mut eps, mut hps) = (0.0, 0.0, 0.0, 0.0);
        for m in self
            .hardpoints
            .iter()
            .filter(|s| s.enabled && s.kind == SlotKind::Hardpoint)
            .filter_map(|s| s.module.as_ref())
        {
            let d = m.r().dps;
            if truthy(Some(d)) {
                dps += d;
                sdps += match m.r().clip.filter(|c| *c != 0.0) {
                    Some(c) => (c * d / m.r().rof) / (c / m.r().rof + num(m.r().reload)),
                    None => d,
                };
            }
            let e = m.r().eps;
            if truthy(Some(e)) {
                eps += e;
            }
            let h = m.r().hps;
            if truthy(Some(h)) {
                hps += h;
            }
        }
        self.stats.total_dps = dps;
        self.stats.total_sdps = sdps;
        self.stats.total_eps = eps;
        self.stats.total_hps = hps;
    }

    fn update_movement(&mut self) {
        let p = &self.def.properties;
        let mass = self.stats.dry_mass + self.stats.fuel_capacity + p.reserve_fuel_capacity;
        let t = self.thrusters();
        let speeds = calc::speed(mass, p.speed, t, p.minthrust);
        let pitch = calc::rotation(mass, p.pitch, t, p.pip_speed);
        let roll = calc::rotation(mass, p.roll, t, p.pip_speed);
        let yaw = calc::rotation(mass, p.yaw, t, p.pip_speed);
        let can_boost = self.can_boost(0.0, self.stats.fuel_capacity);
        let s = &mut self.stats;
        s.speeds = speeds;
        s.top_speed = speeds[4];
        s.top_boost = if can_boost {
            speeds[4] * p.boost / p.speed
        } else {
            0.0
        };
        s.top_pitch = pitch[4];
        s.top_roll = roll[4];
        s.top_yaw = yaw[4];
    }

    fn total_cost(&self) -> f64 {
        self.def.properties.hull_cost
            + self.bulkhead.def.cost
            + self
                .all_slots()
                .filter_map(|s| s.module.as_ref())
                .map(|m| m.def.cost)
                .sum::<f64>()
    }

    /// Coriolis' own jump figures (which count fuel twice; see `corai` metrics for corrected ones).
    pub fn coriolis_jump_stats(&self) -> (f64, f64, f64) {
        let Some(fsd) = self.fsd() else {
            return (0.0, 0.0, 0.0);
        };
        let s = &self.stats;
        let max_fuel = num(fsd.r().maxfuel);
        let unladen = calc::jump_range(
            self,
            s.unladen_mass + max_fuel.min(s.fuel_capacity),
            fsd,
            None,
        );
        let full_tank = calc::jump_range(
            self,
            s.unladen_mass + s.fuel_capacity,
            fsd,
            Some(s.fuel_capacity),
        );
        let laden = calc::jump_range(self, s.laden_mass, fsd, None);
        (unladen, full_tank, laden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_errors() {
        assert!(matches!(
            Ship::empty("no_such_ship"),
            Err(ShipError::UnknownShip(_))
        ));
        let mut ship = Ship::empty("sidewinder").expect("sidewinder");
        assert!(matches!(
            ship.use_bulkhead(99),
            Err(ShipError::UnknownBulkhead(99))
        ));
    }

    #[test]
    fn standard_module_lookup_accepts_ids_and_class_rating_codes() {
        assert_eq!(standard_module(2, "5A").map(|m| m.id.as_str()), Some("Ff"));
        assert_eq!(standard_module(2, "Ff").map(|m| m.id.as_str()), Some("Ff"));
        assert!(standard_module(2, "9Q").is_none());
        assert!(standard_module(99, "5A").is_none());
    }

    #[test]
    fn disabled_modules_draw_no_power() {
        let mut ship = Ship::stock("python").expect("python");
        let before = ship.stats.power_retracted;
        let fsd_power = ship.fsd().and_then(|m| m.r().power).expect("fsd power");
        ship.standard[2].enabled = false;
        ship.recalc();
        assert!((before - ship.stats.power_retracted - fsd_power).abs() < 1e-9);
    }
}
