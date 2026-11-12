//! Typed access to the Coriolis data set (EDCD/coriolis-data): ships, modules and engineering.
//!
//! The JSON is merged at build time (see `build.rs`) and parsed once on first use. Types mirror
//! the upstream shapes faithfully; interpretation (stat resolution, formulas) lives in
//! `corai-engine`.

use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// The whole data set, parsed on first access.
pub static DATA: LazyLock<CoriolisData> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/coriolis.json")))
        .expect("embedded coriolis-data matches the corai-data schema")
});

#[derive(Debug, Clone, Deserialize)]
pub struct CoriolisData {
    pub ships: IndexMap<String, ShipDef>,
    pub modules: ModuleCatalog,
    pub modifications: ModificationsData,
    /// Journal slot names per ship, keyed by Coriolis id or lowercased Frontier ship name.
    #[serde(rename = "shipSlots")]
    pub ship_slots: IndexMap<String, ShipSlotTable>,
}

/// One entry of `ship-slots.json` (the `_ShipSlots` header entry has no slots).
#[derive(Debug, Clone, Deserialize)]
pub struct ShipSlotTable {
    pub slots: Option<NamedSlotGroups>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedSlotGroups {
    #[serde(default)]
    pub standard: Vec<NamedSlot>,
    #[serde(default)]
    pub hardpoints: Vec<NamedSlot>,
    #[serde(default)]
    pub internal: Vec<NamedSlot>,
}

/// A slot with its exact Frontier journal name, e.g. `Slot01_Size7` or `HugeHardpoint1`.
#[derive(Debug, Clone, Deserialize)]
pub struct NamedSlot {
    pub class: u8,
    pub name: String,
}

/// Module definitions by category, then by group code (e.g. `fsd`, `mc`, `sg`).
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleCatalog {
    pub standard: IndexMap<String, Vec<ModuleDef>>,
    pub hardpoints: IndexMap<String, Vec<ModuleDef>>,
    pub internal: IndexMap<String, Vec<ModuleDef>>,
}

// ---------------------------------------------------------------- ships

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipDef {
    #[serde(rename = "edID")]
    pub ed_id: Option<u64>,
    pub properties: ShipProperties,
    pub retail_cost: f64,
    pub bulkheads: Vec<ModuleDef>,
    pub slots: ShipSlots,
    pub defaults: ShipDefaults,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipProperties {
    pub name: String,
    pub manufacturer: String,
    /// Landing pad size: 1 small, 2 medium, 3 large.
    pub class: u8,
    pub hull_cost: f64,
    pub speed: f64,
    pub boost: f64,
    pub minthrust: f64,
    pub boost_energy: f64,
    pub boost_int: f64,
    pub base_shield_strength: f64,
    pub base_armour: f64,
    pub heat_capacity: f64,
    pub hardness: f64,
    pub hull_mass: f64,
    pub masslock: f64,
    #[serde(default)]
    pub pip_speed: f64,
    pub pitch: f64,
    pub roll: f64,
    pub yaw: f64,
    #[serde(default)]
    pub crew: f64,
    #[serde(default)]
    pub reserve_fuel_capacity: f64,
    #[serde(default)]
    pub fighter_hangars: bool,
    #[serde(default)]
    pub luxury_cabins: bool,
    #[serde(default, rename = "vesselHangarsMkII")]
    pub vessel_hangars_mk_ii: bool,
    #[serde(default, rename = "vehicleHangarsMkII")]
    pub vehicle_hangars_mk_ii: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShipSlots {
    pub standard: Vec<SlotSpec>,
    pub hardpoints: Vec<SlotSpec>,
    pub internal: Vec<SlotSpec>,
}

/// A slot is either a plain size or a named/restricted slot.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SlotSpec {
    Size(u8),
    Restricted {
        class: u8,
        name: Option<String>,
        eligible: Option<IndexMap<String, u8>>,
        /// Opt-in for the Mining Volley Repeater (Anaconda Mk II lower large hardpoints).
        #[serde(default)]
        mvr: bool,
    },
}

impl SlotSpec {
    pub fn class(&self) -> u8 {
        match self {
            Self::Size(c) | Self::Restricted { class: c, .. } => *c,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Size(_) => None,
            Self::Restricted { name, .. } => name.as_deref(),
        }
    }

    /// Whether this hardpoint opts in to the Mining Volley Repeater on ships that gate it.
    pub fn mvr(&self) -> bool {
        matches!(self, Self::Restricted { mvr: true, .. })
    }

    /// Groups allowed in a restricted slot; `None` means unrestricted.
    pub fn eligible(&self) -> Option<&IndexMap<String, u8>> {
        match self {
            Self::Size(_) => None,
            Self::Restricted { eligible, .. } => eligible.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShipDefaults {
    /// Standard modules as class+rating codes, e.g. `"5A"`.
    pub standard: Vec<String>,
    #[serde(deserialize_with = "module_refs")]
    pub hardpoints: Vec<Option<String>>,
    #[serde(deserialize_with = "module_refs")]
    pub internal: Vec<Option<String>>,
}

/// Default module references are ids (`"4j"`, `17`) or `0` / `""` for an empty slot.
fn module_refs<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Option<String>>, D::Error> {
    let raw = Vec::<Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s),
            Value::Number(n) if n.as_u64() != Some(0) => Some(n.to_string()),
            _ => None,
        })
        .collect())
}

// ---------------------------------------------------------------- modules

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

/// One purchasable module variant. Numeric stats not modelled as fields live in `stats`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDef {
    pub id: String,
    pub grp: String,
    #[serde(default)]
    pub class: u8,
    #[serde(default)]
    pub rating: String,
    pub name: Option<String>,
    pub uk_name: Option<String>,
    pub symbol: Option<String>,
    /// `F` fixed, `G` gimballed, `T` turret.
    pub mount: Option<String>,
    /// `D` dumbfire, `S` seeker.
    pub missile: Option<String>,
    #[serde(default)]
    pub cost: f64,
    /// Powerplay power that awards this module.
    pub pp: Option<String>,
    pub pre_engineered: Option<PreEngineered>,
    /// Ship ids this module is restricted to (empty = any ship). The data gives one id or a list.
    #[serde(default, deserialize_with = "one_or_many")]
    pub ship: Vec<String>,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub passive: Option<Value>,
    /// Damage type split, e.g. `{"K": 0.5, "T": 0.5}`.
    pub damagedist: Option<IndexMap<String, f64>>,
    /// Every other attribute (mass, power, optmass, damage, ...). Non-numeric extras are kept as JSON.
    #[serde(flatten)]
    pub stats: IndexMap<String, Value>,
}

impl ModuleDef {
    /// Numeric attribute by Coriolis name, if present.
    pub fn stat(&self, name: &str) -> Option<f64> {
        self.stats.get(name).and_then(Value::as_f64)
    }

    /// JavaScript truthiness of the `passive` flag (passive hardpoints draw power when retracted).
    pub fn is_passive(&self) -> bool {
        match &self.passive {
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(_) | Value::Object(_)) => true,
            Some(Value::Null) | None => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreEngineered {
    #[serde(default)]
    pub reengineerable: bool,
    #[serde(default)]
    pub grade_changeable: bool,
    pub blueprints: Vec<String>,
    /// Grade of the fixed blueprints; Coriolis treats a missing grade as 5.
    #[serde(default)]
    pub grade: u8,
    /// Exact tech-broker values (in display units, e.g. 0.5 = +50 %). When present they replace the
    /// blueprint roll computation entirely.
    #[serde(default)]
    pub modifiers: Option<IndexMap<String, f64>>,
    #[serde(default)]
    pub can_apply_experimental: bool,
    #[serde(default)]
    pub experimental_effects: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// `"CG"` marks community-goal rewards that can no longer be obtained.
    pub availability: Option<String>,
}

// ---------------------------------------------------------------- engineering

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModificationsData {
    /// Blueprints by key (usually the Frontier name, e.g. `FSD_LongRange`).
    pub blueprints: IndexMap<String, BlueprintDef>,
    /// Modifiable attributes by name (`optmass`, `kinres`, ...).
    pub modifications: IndexMap<String, ModificationDef>,
    /// Experimental effect adjustments: special key -> attribute -> value.
    pub modifier_actions: IndexMap<String, IndexMap<String, ModifierAction>>,
    /// Per module group: allowed blueprints (with engineers per grade), modifiable attributes and specials.
    pub modules: IndexMap<String, GroupEngineering>,
    /// Experimental effects by key (`special_fsd_heavy`, ...).
    pub specials: IndexMap<String, SpecialDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlueprintDef {
    pub fdname: String,
    pub id: i32,
    pub name: Option<String>,
    pub grades: IndexMap<String, GradeDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GradeDef {
    #[serde(default)]
    pub components: IndexMap<String, u32>,
    /// Attribute -> roll bounds (fractions, e.g. 0.55 = +55%) or, for `damagedist`, a damage split.
    #[serde(default)]
    pub features: IndexMap<String, Feature>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Feature {
    /// [first, second] bounds of the roll.
    Range([f64; 2]),
    /// A replacement damage-type split, e.g. `{"T": 0.961, "A": 0.039}`.
    DamageDist(IndexMap<String, f64>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModificationDef {
    pub id: i32,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ModificationKind,
    pub method: ModificationMethod,
    #[serde(default)]
    pub higherbetter: bool,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModificationKind {
    /// Stored scaled by 10 000 (e.g. +12.34% -> 1234).
    Percentage,
    /// Stored scaled by 100.
    Numeric,
    /// Non-numeric (damage distribution).
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModificationMethod {
    Multiplicative,
    Additive,
    Overwrite,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ModifierAction {
    Value(f64),
    DamageDist(IndexMap<String, f64>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupEngineering {
    #[serde(default)]
    pub blueprints: IndexMap<String, GroupBlueprint>,
    #[serde(default)]
    pub modifications: Vec<String>,
    #[serde(default)]
    pub specials: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupBlueprint {
    #[serde(default)]
    pub grades: IndexMap<String, GroupGrade>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupGrade {
    #[serde(default)]
    pub engineers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpecialDef {
    pub id: i32,
    pub edname: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_flags_and_slot_opt_ins_parse() {
        let p = |f: fn(&ShipProperties) -> bool| {
            DATA.ships.values().filter(|s| f(&s.properties)).count()
        };
        assert_eq!(p(|p| p.vessel_hangars_mk_ii), 4);
        assert_eq!(p(|p| p.vehicle_hangars_mk_ii), 4);
        let anaconda2 = &DATA.ships["anaconda2"];
        assert_eq!(
            anaconda2
                .slots
                .hardpoints
                .iter()
                .filter(|s| s.mvr())
                .count(),
            2
        );
    }

    #[test]
    fn loads_all_ships_and_modules() {
        let d = &*DATA;
        assert!(d.ships.len() >= 47, "{} ships", d.ships.len());
        assert_eq!(d.ships["python"].properties.hull_mass, 350.0);
        assert!(d.modules.standard["fsd"].iter().any(|m| m.id == "5O"));
        assert!(d.modules.internal.contains_key("mahr"));
        assert_eq!(
            d.modifications.blueprints["FSD_LongRange"].fdname,
            "FSD_LongRange"
        );
        assert!(d.modifications.specials.contains_key("special_fsd_heavy"));
    }

    #[test]
    fn default_refs_normalise_empty_slots() {
        let ana = &DATA.ships["anaconda"].defaults;
        assert_eq!(ana.standard[0], "8E");
        assert_eq!(ana.hardpoints[0], None);
        assert_eq!(ana.hardpoints[6].as_deref(), Some("17"));
        assert_eq!(ana.internal[10], None); // "" in the source
    }

    #[test]
    fn duplicate_keys_keep_last_value_like_javascript() {
        // modifications/modules.json defines "amr" twice; JS object semantics keep the last.
        assert!(DATA.modifications.modules.contains_key("amr"));
    }
}
