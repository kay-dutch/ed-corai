//! A fitted module: base definition plus engineering modifications, resolved exactly like
//! Coriolis' `Module.js` (including its null/NaN/truthiness semantics and rounding).

use corai_data::{
    BlueprintDef, DATA, GroupEngineering, ModificationKind, ModificationMethod, ModifierAction,
    ModuleDef, SpecialDef,
};
use std::sync::{LazyLock, OnceLock};

use indexmap::IndexMap;

use crate::jsmath::{js_round, num, or, truthy};

/// Attributes whose modifications scale with `1 - base` when set.
const RESISTANCES: [&str; 4] = ["explres", "kinres", "thermres", "causres"];

/// Pre-engineered modules Coriolis applies at grade 1 instead of 5 when first placed.
/// Share of a weapon's damage by type.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DamageDist {
    pub absolute: f64,
    pub explosive: f64,
    pub kinetic: f64,
    pub thermal: f64,
}

impl DamageDist {
    fn from_map(m: &IndexMap<String, f64>) -> Self {
        let g = |k: &str| m.get(k).copied().unwrap_or(0.0);
        Self {
            absolute: g("A"),
            explosive: g("E"),
            kinetic: g("K"),
            thermal: g("T"),
        }
    }
}

/// A blueprint applied to a module.
#[derive(Debug, Clone)]
pub struct AppliedBlueprint {
    pub key: &'static str,
    pub def: &'static BlueprintDef,
    pub grade: u8,
    /// False when Coriolis set it up for a pre-engineered module: that record carries no blueprint
    /// id, so share codes leave it out and decoding rebuilds it from the module definition.
    pub coded: bool,
}

/// An experimental effect applied to a module.
#[derive(Debug, Clone, Copy)]
pub struct AppliedSpecial {
    pub key: &'static str,
    pub def: &'static SpecialDef,
}

/// Errors from engineering a module.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum EngineeringError {
    #[error("unknown blueprint {0}")]
    UnknownBlueprint(String),
    #[error("blueprint {blueprint} has no grade {grade}")]
    UnknownGrade { blueprint: String, grade: u8 },
    #[error("unknown experimental effect {0}")]
    UnknownSpecial(String),
    #[error("{0} is pre-engineered and cannot be re-engineered")]
    PreEngineered(String),
}

#[derive(Debug, Clone)]
pub struct Module {
    pub def: &'static ModuleDef,
    /// Modification values keyed by attribute, scaled like Coriolis (percentages ×10 000, numerics ×100).
    pub(crate) mods: IndexMap<String, f64>,
    pub(crate) blueprint: Option<AppliedBlueprint>,
    pub(crate) special: Option<AppliedSpecial>,
    /// Damage-type split stored by a blueprint (object-type `damagedist` modification).
    pub(crate) damagedist_mod: Option<&'static IndexMap<String, f64>>,
    /// Stats resolved from the current engineering; cleared by every mutation.
    resolved: OnceLock<Resolved>,
}

macro_rules! resolved_stats {
    ($($field:ident),* $(,)?) => {
        /// Every stat the ship calculations read, resolved once per engineering state.
        /// Fields named after Coriolis attributes hold `get(name)` (modified; `None` = JS null).
        #[derive(Debug, Clone, Copy, Default, PartialEq)]
        pub struct Resolved {
            $(pub $field: Option<f64>,)*
            pub min_mass: Option<f64>,
            pub max_mass: Option<f64>,
            pub falloff: Option<f64>,
            pub clip: Option<f64>,
            /// Unmodified FSD curve constants (Coriolis reads these raw).
            pub fuelmul: Option<f64>,
            pub fuelpower: Option<f64>,
            /// Hull boost modification on reinforcement packages (`getModValue('hullboost')`).
            pub hullboost_mod: Option<f64>,
            /// Mass-curve multipliers `[min, opt, max]` for the generic, `speed` and `rotation` curves.
            pub muls: [[f64; 3]; 3],
            pub dps: f64,
            pub sdps: f64,
            pub eps: f64,
            pub hps: f64,
            pub rof: f64,
            pub damage_dist: DamageDist,
        }

        impl Resolved {
            fn compute(m: &Module) -> Self {
                let muls = |kind| [m.mass_mul("minmul", kind), m.mass_mul("optmul", kind), m.mass_mul("maxmul", kind)];
                Self {
                    $($field: m.get(stringify!($field)),)*
                    min_mass: m.min_mass(),
                    max_mass: m.max_mass(),
                    falloff: m.falloff(),
                    clip: m.clip(),
                    fuelmul: m.base("fuelmul"),
                    fuelpower: m.base("fuelpower"),
                    hullboost_mod: m.mod_value("hullboost", false),
                    muls: [muls(None), muls(Some("speed")), muls(Some("rotation"))],
                    dps: m.dps(),
                    sdps: m.sdps(),
                    eps: m.eps(),
                    hps: m.hps(),
                    rof: m.rof(),
                    damage_dist: m.damage_dist(),
                }
            }
        }
    };
}

resolved_stats!(
    mass,
    power,
    pgen,
    fuel,
    cargo,
    passengers,
    maxfuel,
    optmass,
    engcap,
    engrate,
    syscap,
    sysrate,
    wepcap,
    weprate,
    explres,
    kinres,
    thermres,
    causres,
    shieldboost,
    hullboost,
    hullreinforcement,
    integrity,
    protection,
    regen,
    brokenregen,
    distdraw,
    shieldreinforcement,
    duration,
    ammo,
    shieldaddition,
    jumpboost,
    piercing,
    reload,
    range,
    rate,
    bins,
    maximum,
);

/// Index into [`Resolved::muls`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Generic = 0,
    Speed = 1,
    Rotation = 2,
}

/// Finds a blueprint the way Coriolis' `getBlueprint` does: the first key containing the name,
/// case-insensitively, with the multi-cannon "Overcharged" special case.
pub fn find_blueprint(
    name: &str,
    module: &ModuleDef,
) -> Option<(&'static str, &'static BlueprintDef)> {
    let is_mc = module
        .symbol
        .as_deref()
        .is_some_and(|s| s.to_ascii_lowercase().contains("multicannon"));
    let name = match name {
        "weapon_overcharged" if is_mc => "mc_overcharged",
        "Weapon_Overcharged" if is_mc => "MC_Overcharged",
        other => other,
    };
    let needle = name
        .to_ascii_lowercase()
        .replace("outfittingfieldtype_", "")
        .replace("persecond", "");
    BLUEPRINT_KEYS
        .iter()
        .find(|(lower, _, _)| lower.contains(&needle))
        .map(|(_, key, def)| (*key, *def))
}

/// Blueprint keys in data order, with their lowercase form for `find_blueprint`.
static BLUEPRINT_KEYS: LazyLock<Vec<(String, &'static str, &'static BlueprintDef)>> =
    LazyLock::new(|| {
        let data = &*DATA;
        data.modifications
            .blueprints
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), k.as_str(), v))
            .collect()
    });

pub fn find_special(key: &str) -> Option<AppliedSpecial> {
    let data = &*DATA;
    data.modifications
        .specials
        .get_key_value(key)
        .map(|(k, def)| AppliedSpecial {
            key: k.as_str(),
            def,
        })
}

/// Attribute values a blueprint grade sets at a given roll quality (0..=1), already scaled.
pub fn quality_values(
    bp: &'static BlueprintDef,
    grade: u8,
    quality: f64,
) -> Vec<(&'static str, f64)> {
    let data = &*DATA;
    let Some(g) = bp.grades.get(grade.to_string().as_str()) else {
        return Vec::new();
    };
    g.features
        .iter()
        .filter_map(|(name, feature)| {
            // Damage-split features are not numeric rolls; Coriolis cannot apply them either.
            let corai_data::Feature::Range([a, b]) = *feature else {
                return None;
            };
            let m = data.modifications.modifications.get(name)?;
            let worse_first = a < 0.0 || (a == 0.0 && b < 0.0);
            let value = match (m.higherbetter, worse_first) {
                (true, true) => b + (a - b) * quality,
                (true, false) => a + (b - a) * quality,
                (false, true) => a + (b - a) * quality,
                (false, false) => b + (a - b) * quality,
            };
            let scaled = match m.kind {
                ModificationKind::Percentage => value * 10000.0,
                ModificationKind::Numeric => value * 100.0,
                ModificationKind::Object => value,
            };
            Some((name.as_str(), scaled))
        })
        .collect()
}

impl Module {
    pub fn new(def: &'static ModuleDef) -> Self {
        Self {
            def,
            mods: IndexMap::new(),
            blueprint: None,
            special: None,
            damagedist_mod: None,
            resolved: OnceLock::new(),
        }
    }

    /// Resolved stats for the current engineering (computed on first use).
    pub fn r(&self) -> &Resolved {
        self.resolved.get_or_init(|| Resolved::compute(self))
    }

    /// Must be called after any change to `mods`, `blueprint` or `special`.
    pub(crate) fn invalidate(&mut self) {
        self.resolved = OnceLock::new();
    }

    pub fn mods(&self) -> &IndexMap<String, f64> {
        &self.mods
    }

    pub fn blueprint(&self) -> Option<&AppliedBlueprint> {
        self.blueprint.as_ref()
    }

    pub fn special(&self) -> Option<AppliedSpecial> {
        self.special
    }

    pub fn grp(&self) -> &'static str {
        self.def.grp.as_str()
    }

    fn group_engineering(&self) -> Option<&'static GroupEngineering> {
        let data = &*DATA;
        data.modifications.modules.get(&self.def.grp)
    }

    pub fn is_shield_generator(&self) -> bool {
        matches!(self.grp(), "sg" | "bsg" | "psg")
    }

    // ------------------------------------------------------------ resolution

    /// Unmodified attribute (`this[name]`); `None` for absent (JS `undefined`).
    pub fn base(&self, name: &str) -> Option<f64> {
        match name {
            "class" => Some(f64::from(self.def.class)),
            "cost" => Some(self.def.cost),
            "rof" => self
                .def
                .stat("rof")
                .or_else(|| self.def.stat("fireint").map(|f| 1.0 / f)),
            _ => self.def.stat(name),
        }
    }

    /// `getModValue`: stored modification plus experimental adjustments, rounded.
    /// `None` is JS `null`; `Some(NaN)` is possible and meaningful (it disables the modifier).
    pub fn mod_value(&self, name: &str, raw: bool) -> Option<f64> {
        let mut result = self
            .mods
            .get(name)
            .copied()
            .filter(|v| *v != 0.0 && !v.is_nan());
        if !raw && let Some(special) = self.special {
            let data = &*DATA;
            let action = data
                .modifications
                .modifier_actions
                .get(special.key)
                .and_then(|a| a.get(name));
            // An action for an attribute without a modification definition would throw in JS; ignore it.
            if let (Some(ModifierAction::Value(a)), Some(m)) =
                (action, data.modifications.modifications.get(name))
                && *a != 0.0
            {
                let mult = if m.kind == ModificationKind::Percentage {
                    10000.0
                } else {
                    100.0
                };
                let r = num(result);
                result = Some(if RESISTANCES.contains(&name) {
                    r + a * (1.0 - (self.base(name).unwrap_or(f64::NAN) + r / mult)) * 100.0
                } else {
                    match m.method {
                        ModificationMethod::Additive => r + a * 100.0,
                        ModificationMethod::Overwrite => *a,
                        ModificationMethod::Multiplicative => {
                            ((1.0 + r / mult) * (1.0 + a) - 1.0) * mult
                        }
                    }
                });
            }
        }
        result.map(|v| if v.is_nan() { v } else { js_round(v) })
    }

    /// `get(name)`: the modified attribute value, `None` if absent or NaN.
    pub fn get(&self, name: &str) -> Option<f64> {
        let data = &*DATA;
        let mut result = self.base(name);
        if let Some(m) = data.modifications.modifications.get(name) {
            let mv = self.mod_value(name, false);
            let mut mod_value = match m.kind {
                ModificationKind::Percentage => num(mv) / 10000.0,
                ModificationKind::Numeric => num(mv) / 100.0,
                ModificationKind::Object => num(mv),
            };
            if mod_value != 0.0 && !mod_value.is_nan() {
                if !truthy(result) && m.method == ModificationMethod::Additive {
                    result = Some(0.0);
                }
                if let Some(r) = result {
                    result = Some(match m.method {
                        ModificationMethod::Additive => r + mod_value,
                        ModificationMethod::Overwrite => mod_value,
                        ModificationMethod::Multiplicative
                            if name == "shieldboost" || name == "hullboost" =>
                        {
                            (1.0 + r) * (1.0 + mod_value) - 1.0
                        }
                        ModificationMethod::Multiplicative => {
                            if name == "rof" {
                                mod_value = 1.0 / (1.0 + mod_value) - 1.0;
                            }
                            r * (1.0 + mod_value)
                        }
                    });
                } else if name == "burstrof" || name == "burst" {
                    result = Some(mod_value);
                }
            }
        }
        result.filter(|v| !v.is_nan())
    }

    /// Unmodified attribute as returned by `get(name, false)`.
    pub fn get_base(&self, name: &str) -> Option<f64> {
        self.base(name).filter(|v| !v.is_nan())
    }

    fn mass_with_related_modifier(&self, name: &str) -> Option<f64> {
        let base = self.base(name).filter(|v| !v.is_nan())?;
        let mut m = self.mod_value(name, false);
        if m.is_none_or(f64::is_nan) {
            if self.is_shield_generator() {
                if let Some(opt) = self.mod_value("optmass", false).filter(|v| !v.is_nan()) {
                    if name == "minmass" || (name == "maxmass" && opt > 0.0) {
                        m = Some(opt);
                    }
                }
            } else {
                m = self.mod_value("optmass", false);
            }
        }
        Some(match m {
            Some(v) if !v.is_nan() => base * (1.0 + v / 10000.0),
            _ => base,
        })
    }

    pub fn min_mass(&self) -> Option<f64> {
        self.mass_with_related_modifier("minmass")
    }

    pub fn max_mass(&self) -> Option<f64> {
        self.mass_with_related_modifier("maxmass")
    }

    pub fn opt_mass(&self) -> Option<f64> {
        self.get("optmass")
    }

    /// `getMinMul`/`getOptMul`/`getMaxMul`: `prefix` is `minmul`/`optmul`/`maxmul`,
    /// `kind` optionally `speed`, `rotation`, ...
    pub fn mass_mul(&self, prefix: &str, kind: Option<&str>) -> f64 {
        let specific = kind
            .and_then(|k| self.base(&format!("{prefix}{k}")))
            .filter(|v| truthy(Some(*v)));
        let mut result = match specific {
            Some(v) => v,
            None => or(self.base(prefix), 0.0),
        };
        if truthy(Some(result)) {
            let mult = num(self.mod_value("optmul", false)) / 10000.0;
            if truthy(Some(mult)) {
                result *= 1.0 + mult;
            }
        }
        result
    }

    pub fn damage_dist(&self) -> DamageDist {
        if let Some(special) = self.special {
            let data = &*DATA;
            if let Some(ModifierAction::DamageDist(d)) = data
                .modifications
                .modifier_actions
                .get(special.key)
                .and_then(|a| a.get("damagedist"))
            {
                return DamageDist::from_map(d);
            }
        }
        if let Some(d) = self.damagedist_mod {
            return DamageDist::from_map(d);
        }
        self.def
            .damagedist
            .as_ref()
            .map(DamageDist::from_map)
            .unwrap_or_default()
    }

    pub fn range(&self) -> Option<f64> {
        self.get("range")
    }

    pub fn falloff(&self) -> Option<f64> {
        if truthy(self.mods.get("fallofffromrange").copied()) {
            return self.range();
        }
        let range = self.range();
        let falloff = self.get("falloff");
        match (falloff, range) {
            (Some(f), Some(r)) => Some(if f > r { r } else { f }),
            // JS compares null as 0: `f > null` picks `range` (null).
            (Some(f), None) => {
                if f > 0.0 {
                    None
                } else {
                    Some(f)
                }
            }
            (None, _) => None,
        }
    }

    pub fn rof(&self) -> f64 {
        let burst = or(self.get("burst"), 1.0);
        let burst_rof = or(self.get("burstrof"), 1.0);
        let int_rof = num(self.get("rof"));
        let charge = or(self.get("charge"), 0.0);
        burst / ((burst - 1.0) / burst_rof + 1.0 / int_rof + charge)
    }

    fn rof_or_one(&self) -> f64 {
        or(Some(self.rof()), 1.0)
    }

    pub fn dps(&self) -> f64 {
        let rounds = or(self.base("roundspershot"), 1.0);
        num(self.get("damage")) * rounds * self.rof_or_one()
    }

    pub fn eps(&self) -> f64 {
        num(self.get("distdraw")) * self.rof_or_one()
    }

    pub fn hps(&self) -> f64 {
        num(self.get("thermload")) * self.rof_or_one()
    }

    pub fn clip(&self) -> Option<f64> {
        self.get("clip")
            .map(|c| if c != 0.0 { c.ceil() } else { c })
    }

    pub fn reload(&self) -> Option<f64> {
        self.get("reload")
    }

    pub fn sustained_factor(&self) -> f64 {
        let Some(mut clip) = self.clip().filter(|c| *c != 0.0) else {
            return 1.0;
        };
        if self
            .special
            .is_some_and(|s| s.def.edname == "special_auto_loader")
        {
            clip += clip - 1.0;
        }
        let burst = or(self.get("burst"), 1.0);
        let rof = self.rof();
        let overhead = (burst - 1.0) / or(self.get("burstrof"), 1.0);
        let srof = clip / ((clip - burst) / rof + overhead + num(self.reload()));
        srof / rof
    }

    pub fn sdps(&self) -> f64 {
        self.dps() * self.sustained_factor()
    }

    // ------------------------------------------------------------ engineering

    /// `setModValue` (value not including special effects).
    pub fn set_mod_value(&mut self, name: &str, value: f64) {
        let value = if RESISTANCES.contains(&name) {
            (1.0 - num(self.get_base(name))) * value
        } else {
            value
        };
        if value == 0.0 {
            self.mods.shift_remove(name);
        } else {
            self.mods.insert(name.to_string(), value);
        }
        self.invalidate();
    }

    /// `Ship.setModification`: ignored unless the group supports `name` (or `force`).
    pub fn set_modification(&mut self, name: &str, value: f64, force: bool) {
        if !force
            && let Some(g) = self.group_engineering()
            && !g.modifications.iter().any(|m| m == name)
        {
            return;
        }
        self.set_mod_value(name, if value.is_nan() { 0.0 } else { value });
    }

    /// Applies a blueprint at a roll quality (1.0 = best roll), replacing previous modifications.
    ///
    /// # Errors
    /// Unknown blueprint/grade, or a pre-engineered module.
    pub fn apply_blueprint(
        &mut self,
        name: &str,
        grade: u8,
        quality: f64,
    ) -> Result<(), EngineeringError> {
        if self.def.pre_engineered.is_some() {
            return Err(EngineeringError::PreEngineered(self.def.id.clone()));
        }
        let (key, def) = find_blueprint(name, self.def)
            .ok_or_else(|| EngineeringError::UnknownBlueprint(name.into()))?;
        if !def.grades.contains_key(grade.to_string().as_str()) {
            return Err(EngineeringError::UnknownGrade {
                blueprint: key.into(),
                grade,
            });
        }
        self.blueprint = Some(AppliedBlueprint {
            key,
            def,
            grade,
            coded: true,
        });
        self.mods.clear();
        self.damagedist_mod = None;
        for (feature, value) in quality_values(def, grade, quality) {
            self.set_modification(feature, value, false);
        }
        // Object-type features (a replacement damage split) are stored as given, if the group
        // accepts the modification at all (Ship.setModification).
        if let Some(g) = def.grades.get(grade.to_string().as_str()) {
            for (name, feature) in &g.features {
                if let corai_data::Feature::DamageDist(d) = feature
                    && self
                        .group_engineering()
                        .is_none_or(|ge| ge.modifications.iter().any(|m| m == name))
                {
                    self.damagedist_mod = Some(d);
                }
            }
        }
        self.invalidate();
        Ok(())
    }

    /// Sets (or clears) the experimental effect.
    ///
    /// # Errors
    /// Unknown special key.
    pub fn set_special(&mut self, key: Option<&str>) -> Result<(), EngineeringError> {
        self.special = match key {
            None => None,
            Some(k) => {
                Some(find_special(k).ok_or_else(|| EngineeringError::UnknownSpecial(k.into()))?)
            }
        };
        self.invalidate();
        Ok(())
    }

    /// What `Ship.use()` does when a pre-engineered module is placed. Modules with explicit
    /// tech-broker modifiers get a blueprint record without a code id; the others get the full first
    /// blueprint (with its id, so share codes store it).
    pub fn init_pre_engineered(&mut self) {
        if self.blueprint.is_some() || self.special.is_some() {
            return;
        }
        self.apply_pre_engineered(true, false);
        let explicit = self
            .def
            .pre_engineered
            .as_ref()
            .is_some_and(|pe| pe.modifiers.is_some());
        if let Some(b) = self.blueprint.as_mut() {
            b.coded = !explicit;
        }
    }

    /// `Ship._applyPreEngineeredModifiers`. With `setup_blueprint` the first fixed blueprint (without
    /// a code id) and the first experimental are recorded; `clear_first` drops existing
    /// modifications. The stats come either from the module's explicit tech-broker modifiers or,
    /// when it has none, from every fixed blueprint added onto the current values at best roll.
    pub(crate) fn apply_pre_engineered(&mut self, setup_blueprint: bool, clear_first: bool) {
        let Some(pe) = &self.def.pre_engineered else {
            return;
        };
        if pe.blueprints.is_empty() {
            return;
        }
        let data = &*DATA;
        let grade = if pe.grade == 0 { 5 } else { pe.grade };
        if setup_blueprint {
            self.blueprint = find_blueprint(pe.blueprints[0].trim(), self.def).map(|(key, def)| {
                AppliedBlueprint {
                    key,
                    def,
                    grade,
                    coded: false,
                }
            });
            if let Some(effect) = pe.experimental_effects.first() {
                self.special = data
                    .modifications
                    .specials
                    .iter()
                    .find(|(_, s)| &s.edname == effect)
                    .map(|(k, def)| AppliedSpecial {
                        key: k.as_str(),
                        def,
                    });
            }
        }
        if clear_first {
            self.mods.clear();
            self.damagedist_mod = None;
        }
        if let Some(modifiers) = &pe.modifiers {
            for (feature, value) in modifiers {
                let Some(m) = data.modifications.modifications.get(feature) else {
                    continue;
                };
                let scaled = match m.kind {
                    ModificationKind::Percentage => value * 10000.0,
                    ModificationKind::Numeric => value * 100.0,
                    ModificationKind::Object => continue,
                };
                self.set_modification(feature, scaled, true);
            }
        } else {
            for name in &pe.blueprints {
                let Some((_, def)) = find_blueprint(name.trim(), self.def) else {
                    continue;
                };
                for (feature, value) in quality_values(def, grade, 1.0) {
                    let current = or(self.mod_value(feature, true), 0.0);
                    self.set_modification(feature, current + value, true);
                }
            }
        }
        self.invalidate();
    }

    /// Re-applies a blueprint's object-type features (a replacement damage split), which share
    /// codes cannot store; `buildWith` does this for engineered hardpoints.
    pub(crate) fn restore_object_features(&mut self) {
        let Some(b) = &self.blueprint else { return };
        if let Some(g) = b.def.grades.get(b.grade.to_string().as_str()) {
            for feature in g.features.values() {
                if let corai_data::Feature::DamageDist(d) = feature {
                    self.damagedist_mod = Some(d);
                }
            }
        }
        self.invalidate();
    }

    /// Engineering in compact form: (blueprint key, grade, special key).
    pub fn engineering(&self) -> (Option<(&'static str, u8)>, Option<&'static str>) {
        (
            self.blueprint.as_ref().map(|b| (b.key, b.grade)),
            self.special.map(|s| s.key),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(category: &str, grp: &str, id: &str) -> Module {
        let data = &*DATA;
        let groups = match category {
            "standard" => &data.modules.standard,
            "hardpoints" => &data.modules.hardpoints,
            _ => &data.modules.internal,
        };
        Module::new(
            groups[grp]
                .iter()
                .find(|m| m.id == id)
                .expect("module exists"),
        )
    }

    #[test]
    fn increased_range_fsd_matches_coriolis() {
        // 5A FSD (id Ff): optmass 1050, G5 Increased Range rolls optmass +55%.
        let mut m = module("standard", "fsd", "Ff");
        m.apply_blueprint("FSD_LongRange", 5, 1.0)
            .expect("valid blueprint");
        assert_eq!(m.opt_mass(), Some(1050.0 * 1.55));
        m.set_special(Some("special_fsd_heavy"))
            .expect("valid special");
        assert!((m.opt_mass().expect("optmass") - 1050.0 * 1.55 * 1.04).abs() < 1e-9);
    }

    #[test]
    fn multicannon_overcharged_uses_mc_blueprint() {
        let m = module("hardpoints", "mc", "23");
        assert_eq!(
            find_blueprint("Weapon_Overcharged", m.def).map(|b| b.0),
            Some("MC_Overcharged")
        );
    }

    #[test]
    fn stat_cache_follows_every_change() {
        // Read through the cache before each change: a missing invalidation would return stale stats.
        let mut m = module("standard", "fsd", "Ff");
        assert_eq!(m.r().optmass, Some(1050.0));
        m.apply_blueprint("FSD_LongRange", 5, 1.0)
            .expect("valid blueprint");
        assert_eq!(m.r().optmass, Some(1050.0 * 1.55));
        m.set_special(Some("special_fsd_heavy"))
            .expect("valid special");
        assert!((m.r().optmass.expect("optmass") - 1050.0 * 1.55 * 1.04).abs() < 1e-9);
        m.set_modification("optmass", 0.0, false);
        assert!((m.r().optmass.expect("optmass") - 1050.0 * 1.04).abs() < 1e-9);
        m.set_special(None).expect("clearing is valid");
        assert_eq!(m.r().optmass, Some(1050.0));
    }

    #[test]
    fn engineering_errors() {
        let mut m = module("standard", "fsd", "Ff");
        assert!(matches!(
            m.apply_blueprint("NoSuchBlueprint", 5, 1.0),
            Err(EngineeringError::UnknownBlueprint(_))
        ));
        assert!(matches!(
            m.apply_blueprint("FSD_LongRange", 9, 1.0),
            Err(EngineeringError::UnknownGrade { grade: 9, .. })
        ));
        assert!(matches!(
            m.set_special(Some("special_nope")),
            Err(EngineeringError::UnknownSpecial(_))
        ));
        let mut pre = module("standard", "fsd", "5O");
        assert!(matches!(
            pre.apply_blueprint("FSD_LongRange", 5, 1.0),
            Err(EngineeringError::PreEngineered(_))
        ));
    }

    #[test]
    fn pre_engineered_modules_get_their_fixed_engineering() {
        let mut m = module("standard", "fsd", "5O"); // V1 FSD: G5 Increased Range + Faster Boot
        m.init_pre_engineered();
        assert_eq!(
            m.blueprint().map(|b| (b.key, b.grade)),
            Some(("FSD_LongRange", 5))
        );
        assert!(m.r().optmass.expect("optmass") > 1050.0);
    }

    #[test]
    fn rof_derives_from_fire_interval() {
        let m = module("hardpoints", "mc", "23"); // fireint 0.13
        assert!((m.rof() - 1.0 / 0.13).abs() < 1e-12);
    }
}
