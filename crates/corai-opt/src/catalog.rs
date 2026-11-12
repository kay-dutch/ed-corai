//! Which modules (and which engineering) may legally go into which slot.

use std::collections::HashSet;

use corai_data::{DATA, ModuleDef};
use corai_engine::{STANDARD_GROUPS, Ship, Slot, SlotKind};

/// Where a module sits on a ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotRef {
    Bulkheads,
    Standard(usize),
    /// Index into `Ship::hardpoints` (weapon and utility mounts).
    Hardpoint(usize),
    Internal(usize),
}

impl SlotRef {
    /// Every slot of a ship, in Coriolis order (bulkheads, standard, hardpoints, internal).
    pub fn all(ship: &Ship) -> Vec<SlotRef> {
        std::iter::once(SlotRef::Bulkheads)
            .chain((0..ship.standard.len()).map(SlotRef::Standard))
            .chain((0..ship.hardpoints.len()).map(SlotRef::Hardpoint))
            .chain((0..ship.internal.len()).map(SlotRef::Internal))
            .collect()
    }

    pub fn slot<'a>(&self, ship: &'a Ship) -> Option<&'a Slot> {
        match *self {
            SlotRef::Bulkheads => None,
            SlotRef::Standard(i) => ship.standard.get(i),
            SlotRef::Hardpoint(i) => ship.hardpoints.get(i),
            SlotRef::Internal(i) => ship.internal.get(i),
        }
    }

    pub fn slot_mut<'a>(&self, ship: &'a mut Ship) -> Option<&'a mut Slot> {
        match *self {
            SlotRef::Bulkheads => None,
            SlotRef::Standard(i) => ship.standard.get_mut(i),
            SlotRef::Hardpoint(i) => ship.hardpoints.get_mut(i),
            SlotRef::Internal(i) => ship.internal.get_mut(i),
        }
    }

    /// Stable user-facing key: `bulkheads`, `standard:fsd`, `hardpoint:2`, `utility:1`, `internal:5`.
    pub fn key(&self, ship: &Ship) -> String {
        match *self {
            SlotRef::Bulkheads => "bulkheads".into(),
            SlotRef::Standard(i) => format!(
                "standard:{}",
                STANDARD_GROUPS.get(i).copied().unwrap_or("?")
            ),
            SlotRef::Internal(i) => format!("internal:{}", i + 1),
            SlotRef::Hardpoint(i) => {
                let kind = ship.hardpoints[i].kind;
                let n = ship.hardpoints[..=i]
                    .iter()
                    .filter(|s| s.kind == kind)
                    .count();
                format!(
                    "{}:{n}",
                    if kind == SlotKind::Utility {
                        "utility"
                    } else {
                        "hardpoint"
                    }
                )
            }
        }
    }

    /// The slot kind as lock rules name it: `bulkheads`, `standard`, `hardpoint`, `utility`,
    /// `internal`.
    pub fn kind_name(&self, ship: &Ship) -> &'static str {
        match *self {
            SlotRef::Bulkheads => "bulkheads",
            SlotRef::Standard(_) => "standard",
            SlotRef::Internal(_) => "internal",
            SlotRef::Hardpoint(i) => {
                if ship.hardpoints[i].kind == SlotKind::Utility {
                    "utility"
                } else {
                    "hardpoint"
                }
            }
        }
    }

    /// Whether a lock rule covers this slot: its key (`internal:3`), its kind (`hardpoint`), or for
    /// standard slots its group (`fsd`).
    pub fn matches_lock(&self, ship: &Ship, rule: &str) -> bool {
        let key = self.key(ship);
        rule == key
            || rule == self.kind_name(ship)
            || (matches!(self, SlotRef::Standard(_)) && key.strip_prefix("standard:") == Some(rule))
    }

    /// Every slot covered by any of the lock rules.
    pub fn locked_by(ship: &Ship, rules: &[impl AsRef<str>]) -> Vec<SlotRef> {
        SlotRef::all(ship)
            .into_iter()
            .filter(|r| rules.iter().any(|rule| r.matches_lock(ship, rule.as_ref())))
            .collect()
    }

    /// Inverse of [`SlotRef::key`].
    pub fn parse(ship: &Ship, key: &str) -> Option<SlotRef> {
        SlotRef::all(ship).into_iter().find(|r| r.key(ship) == key)
    }
}

/// Which modules the search may use. Everything is allowed by default.
#[derive(Debug, Clone)]
pub struct Filters {
    /// Modules awarded by Powerplay.
    pub allow_powerplay: bool,
    /// Pre-engineered modules from tech brokers / outfitting.
    pub allow_pre_engineered: bool,
    /// Community-goal reward modules.
    pub allow_community_goal: bool,
    /// Guardian technology.
    pub allow_guardian: bool,
    /// AX / Guardian experimental weapons.
    pub allow_experimental: bool,
    pub exclude_groups: Vec<String>,
    pub exclude_modules: Vec<String>,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            allow_powerplay: true,
            allow_pre_engineered: true,
            allow_community_goal: true,
            allow_guardian: true,
            allow_experimental: true,
            exclude_groups: Vec::new(),
            exclude_modules: Vec::new(),
        }
    }
}

/// Only one module of each family may be fitted.
const UNIQUE_FAMILIES: &[&[&str]] = &[
    &["sg", "bsg", "psg"],
    &["rf"],
    &["fs"],
    &["fh"],
    &["gfsb"],
    &["dc"],
    &["ews"],
    &["mlc"],
    &["pv"],
    &["sua"],
    &["fi"],
    &["ss"],
    &["cs"],
    &["ws"],
    &["kw"],
    &["xs"],
    &["pwa"],
];
/// In-game limit on AX/Guardian experimental weapons.
pub const MAX_EXPERIMENTAL_WEAPONS: usize = 4;
const GUARDIAN_GROUPS: &[&str] = &[
    "gpc", "ggc", "gsc", "gsrp", "gfsb", "ghrp", "gmrp", "gpp", "gpd",
];
const MINING_GROUPS: &[&str] = &["ml", "scl", "sdm", "abl", "mvr", "pwa"];

pub fn unique_family(grp: &str) -> Option<&'static [&'static str]> {
    UNIQUE_FAMILIES.iter().copied().find(|f| f.contains(&grp))
}

/// Entries that are not real modules ("Unrecognised …" import placeholders, rating Z), and the
/// one module whose single-character id cannot be represented in a Coriolis share code.
fn unusable(m: &ModuleDef) -> bool {
    let named = m.name.is_some() || m.uk_name.is_some();
    let symbol = m.symbol.as_deref().unwrap_or_default();
    let name = m.name.as_deref().unwrap_or_default().to_ascii_lowercase();
    (matches!(m.id.as_str(), "0z" | "1z") && !named)
        || m.rating == "Z"
        || symbol.contains("_Missing_")
        || matches!(m.grp.as_str(), "mh" | "mm")
        || name.contains("error")
        || name.contains("placeholder")
        || m.id.len() != 2
}

fn passes(m: &ModuleDef, f: &Filters) -> bool {
    if unusable(m) || (m.pp.is_some() && !f.allow_powerplay) {
        return false;
    }
    if let Some(pe) = &m.pre_engineered
        && (!f.allow_pre_engineered
            || (pe.availability.as_deref() == Some("CG") && !f.allow_community_goal))
    {
        return false;
    }
    // Pre-engineered modules whose blueprints the data cannot resolve would be applied with no effect.
    if m.pre_engineered
        .as_ref()
        .is_some_and(|pe| pe.blueprints.iter().any(|b| !blueprint_exists(b.trim())))
    {
        return false;
    }
    !(is_guardian(m) && !f.allow_guardian)
        && !(m.experimental && !f.allow_experimental)
        && !f.exclude_groups.contains(&m.grp)
        && !f.exclude_modules.contains(&m.id)
}

/// Mirrors Coriolis' `canMount` and its restricted-slot rules.
pub fn fits(ship: &Ship, slot: &Slot, m: &ModuleDef) -> bool {
    if !m.ship.is_empty() && !m.ship.iter().any(|s| s == ship.id) {
        return false;
    }
    let name = slot.name.unwrap_or_default().to_ascii_lowercase();
    let eligible = |grp: &str| slot.eligible.is_none_or(|e| e.contains_key(grp));
    // ModuleSet lists only modules whose max mass (shield generators, ...) carries hull + 6.5 t.
    if slot.kind != SlotKind::Standard
        && m.stat("maxmass")
            .is_some_and(|mm| ship.def.properties.hull_mass + 6.5 > mm)
    {
        return false;
    }
    let props = &ship.def.properties;
    match slot.kind {
        SlotKind::Standard => m.class <= slot.max_class, // refined per slot by `fits_standard`
        SlotKind::Internal => {
            m.class <= slot.max_class
                && eligible(&m.grp)
                && ((m.id != "5L" && m.id != "5M" && m.grp != "crl") || name == "cargo")
                && ((m.id != "4C" && m.id != "4F" && m.grp != "pas")
                    || name == "planetaryapproachsuite")
                && (m.id != "7W" || name == "limpets")
                && (m.grp != "pcq" || props.luxury_cabins)
                && (m.grp != "fh" || props.fighter_hangars)
                && (m.grp != "fhmkii" || props.vessel_hangars_mk_ii)
                && (m.grp != "lpvmkii" || props.vehicle_hangars_mk_ii)
        }
        SlotKind::Hardpoint => {
            m.class > 0
                && m.class <= slot.max_class
                && eligible(&m.grp)
                && (name != "mining" || MINING_GROUPS.contains(&m.grp.as_str()))
                // The Anaconda Mk II takes the volley repeater only on its two flagged hardpoints.
                && (m.grp != "mvr" || ship.id != "anaconda2" || slot.mvr)
        }
        SlotKind::Utility => m.class == 0 && eligible(&m.grp),
    }
}

/// Core-module rules from Coriolis' outfitting list (ModuleSet): life support and sensors must
/// match the slot class, SCO frame shift drives must match it too, thrusters must be at least the
/// smallest class able to move the hull, and modules with a max mass must carry hull + 6.5 t.
fn fits_standard(ship: &Ship, slot: &Slot, index: usize, m: &ModuleDef) -> bool {
    let data = &*DATA;
    let mass = ship.def.properties.hull_mass + 6.5;
    let max_mass_ok = m.stat("maxmass").is_none_or(|mm| mass <= mm);
    match STANDARD_GROUPS.get(index).copied() {
        Some("ls") => m.class == slot.max_class,
        Some("s") => m.class == slot.max_class && max_mass_ok,
        Some("fsd") if m.name.as_deref().is_some_and(|n| n.contains("(SCO)")) => {
            m.class == slot.max_class
        }
        Some("t") => {
            let min_class = data.modules.standard["t"]
                .iter()
                .filter(|t| t.stat("maxmass").is_some_and(|mm| mm >= mass))
                .map(|t| t.class)
                .fold(slot.max_class, u8::min);
            m.class >= min_class
        }
        _ => max_mass_ok,
    }
}

/// Modules that may go into `slot`, limited to `groups` (`None` = all) for hardpoint, utility
/// and internal slots. Standard slots always list their own group.
pub fn candidates(
    ship: &Ship,
    slot: &Slot,
    index: usize,
    filters: &Filters,
    groups: Option<&HashSet<&str>>,
) -> Vec<&'static ModuleDef> {
    let data = &*DATA;
    let pool: Vec<&'static ModuleDef> = match slot.kind {
        SlotKind::Standard => STANDARD_GROUPS
            .get(index)
            .and_then(|g| data.modules.standard.get(*g))
            .map(|g| g.iter().collect())
            .unwrap_or_default(),
        SlotKind::Internal => data
            .modules
            .internal
            .iter()
            .filter(|(g, _)| groups.is_none_or(|s| s.contains(g.as_str())))
            .flat_map(|(_, ms)| ms)
            .collect(),
        SlotKind::Hardpoint | SlotKind::Utility => data
            .modules
            .hardpoints
            .iter()
            .filter(|(g, _)| groups.is_none_or(|s| s.contains(g.as_str())))
            .flat_map(|(_, ms)| ms)
            .collect(),
    };
    pool.into_iter()
        .filter(|m| fits(ship, slot, m) && passes(m, filters))
        .filter(|m| slot.kind != SlotKind::Standard || fits_standard(ship, slot, index, m))
        .collect()
}

/// An engineering choice: blueprint at a grade, optionally with an experimental effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EngSpec {
    pub blueprint: &'static str,
    pub grade: u8,
    pub special: Option<&'static str>,
}

fn max_grade(grp: &str, blueprint: &str) -> Option<u8> {
    let data = &*DATA;
    let grades = data
        .modifications
        .modules
        .get(grp)?
        .blueprints
        .get(blueprint)?;
    grades
        .grades
        .iter()
        .filter(|(_, g)| !g.engineers.is_empty()) // e.g. CargoRack_IncreasedCapacity: no engineer offers it
        .filter_map(|(k, _)| k.parse::<u8>().ok())
        .filter(|g| {
            data.modifications
                .blueprints
                .get(blueprint)
                .is_some_and(|b| b.grades.contains_key(g.to_string().as_str()))
        })
        .max()
}

/// Engineering worth trying on a module: each obtainable blueprint at its highest grade, alone and
/// with each compatible experimental. Pre-engineered modules can only take an experimental.
/// Whether Coriolis' blueprint lookup resolves a name (case-insensitive substring).
fn blueprint_exists(name: &str) -> bool {
    let needle = name
        .to_ascii_lowercase()
        .replace("outfittingfieldtype_", "")
        .replace("persecond", "");
    DATA.modifications
        .blueprints
        .keys()
        .any(|k| k.to_ascii_lowercase().contains(&needle))
}

/// Guardian technology, by group or by name/symbol (the hybrid power plant and distributor sit in
/// the normal `pp`/`pd` groups).
pub fn is_guardian(m: &ModuleDef) -> bool {
    GUARDIAN_GROUPS.contains(&m.grp.as_str())
        || m.symbol
            .as_deref()
            .is_some_and(|s| s.to_ascii_lowercase().contains("guardian"))
        || m.name
            .as_deref()
            .is_some_and(|n| n.starts_with("Guardian "))
}

/// Coriolis' rule for whether a module can take engineering: `engineering` flag not false, group
/// has modifiable attributes, and not Guardian technology.
pub fn can_engineer(m: &ModuleDef) -> bool {
    let disabled = matches!(
        m.stats.get("engineering"),
        Some(serde_json::Value::Bool(false))
    ) || m.stats.get("engineering").and_then(|v| v.as_str()) == Some("False");
    if disabled || is_guardian(m) {
        return false;
    }
    DATA.modifications
        .modules
        .get(&m.grp)
        .is_some_and(|g| !g.modifications.is_empty() || !g.blueprints.is_empty())
}

pub fn engineering_options(m: &ModuleDef) -> Vec<EngSpec> {
    if !can_engineer(m) {
        return Vec::new();
    }
    let data = &*DATA;
    let Some(group) = data.modifications.modules.get(&m.grp) else {
        return Vec::new();
    };
    let specials: Vec<&'static str> = group
        .specials
        .iter()
        .filter_map(|s| data.modifications.specials.get_key_value(s))
        .map(|(k, _)| k.as_str())
        .collect();
    if let Some(pe) = &m.pre_engineered {
        if !pe.can_apply_experimental {
            return Vec::new();
        }
        let Some(first) = pe
            .blueprints
            .first()
            .and_then(|b| data.modifications.blueprints.get_key_value(b.as_str()))
        else {
            return Vec::new();
        };
        return specials
            .iter()
            .map(|s| EngSpec {
                blueprint: first.0.as_str(),
                grade: pe.grade,
                special: Some(s),
            })
            .collect();
    }
    let mut out = Vec::new();
    for (key, _) in &group.blueprints {
        let Some((key, _)) = data.modifications.blueprints.get_key_value(key.as_str()) else {
            continue;
        };
        let Some(grade) = max_grade(&m.grp, key) else {
            continue;
        };
        out.push(EngSpec {
            blueprint: key,
            grade,
            special: None,
        });
        out.extend(specials.iter().map(|s| EngSpec {
            blueprint: key,
            grade,
            special: Some(s),
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(id: &str) -> Ship {
        Ship::stock(id).expect("ship")
    }

    #[test]
    fn slot_keys_round_trip() {
        let s = ship("anaconda");
        for r in SlotRef::all(&s) {
            assert_eq!(SlotRef::parse(&s, &r.key(&s)), Some(r));
        }
        assert_eq!(
            SlotRef::parse(&s, "standard:fsd"),
            Some(SlotRef::Standard(2))
        );
        assert_eq!(SlotRef::parse(&s, "nope:1"), None);
    }

    #[test]
    fn excludes_placeholders_and_url_unsafe_ids() {
        let s = ship("anaconda");
        let all: Vec<_> = SlotRef::all(&s)
            .into_iter()
            .filter_map(|r| r.slot(&s).map(|sl| (r, sl)))
            .flat_map(|(r, sl)| {
                let i = match r {
                    SlotRef::Standard(i) | SlotRef::Hardpoint(i) | SlotRef::Internal(i) => i,
                    SlotRef::Bulkheads => 0,
                };
                candidates(&s, sl, i, &Filters::default(), None)
            })
            .collect();
        assert!(!all.is_empty());
        assert!(all.iter().all(|m| m.rating != "Z" && m.id.len() == 2));
    }

    #[test]
    fn military_slot_only_takes_eligible_groups() {
        let s = ship("anaconda");
        let (i, slot) = s
            .internal
            .iter()
            .enumerate()
            .find(|(_, sl)| sl.name == Some("Military"))
            .expect("military slot");
        let cands = candidates(&s, slot, i, &Filters::default(), None);
        assert!(!cands.is_empty());
        assert!(
            cands
                .iter()
                .all(|m| slot.eligible.is_some_and(|e| e.contains_key(&m.grp)))
        );
        assert!(!cands.iter().any(|m| m.grp == "sg"));
    }

    #[test]
    fn core_modules_follow_coriolis_class_rules() {
        let s = ship("anaconda");
        let classes = |i: usize| {
            candidates(&s, &s.standard[i], i, &Filters::default(), None)
                .iter()
                .map(|m| m.class)
                .collect::<HashSet<_>>()
        };
        assert_eq!(
            classes(3),
            HashSet::from([s.standard[3].max_class]),
            "life support"
        );
        assert_eq!(
            classes(5),
            HashSet::from([s.standard[5].max_class]),
            "sensors"
        );
        // Smallest thruster class whose max mass carries hull + 6.5 t (4E at 420 t for the 400 t Anaconda).
        let hull = s.def.properties.hull_mass + 6.5;
        let min_class = DATA.modules.standard["t"]
            .iter()
            .filter(|t| t.stat("maxmass").is_some_and(|m| m >= hull))
            .map(|t| t.class)
            .min()
            .expect("some thruster");
        assert_eq!(min_class, 4);
        assert_eq!(
            classes(1).iter().min().copied(),
            Some(min_class),
            "thrusters must move the hull"
        );
        let fsd = candidates(&s, &s.standard[2], 2, &Filters::default(), None);
        assert!(
            fsd.iter()
                .filter(|m| m.name.as_deref().is_some_and(|n| n.contains("(SCO)")))
                .all(|m| m.class == s.standard[2].max_class)
        );
    }

    #[test]
    fn group_restriction_limits_candidates() {
        let s = ship("python");
        let groups: HashSet<&str> = ["cr"].into_iter().collect();
        let cands = candidates(&s, &s.internal[0], 0, &Filters::default(), Some(&groups));
        assert!(!cands.is_empty() && cands.iter().all(|m| m.grp == "cr"));
    }

    #[test]
    fn engineering_skips_blueprints_no_engineer_offers() {
        let rack = DATA.modules.internal["cr"]
            .iter()
            .find(|m| m.pre_engineered.is_none())
            .expect("cargo rack");
        assert!(engineering_options(rack).is_empty());
        let fsd = DATA.modules.standard["fsd"]
            .iter()
            .find(|m| m.id == "Ff")
            .expect("5A FSD");
        let opts = engineering_options(fsd);
        assert!(opts.contains(&EngSpec {
            blueprint: "FSD_LongRange",
            grade: 5,
            special: Some("special_fsd_heavy")
        }));
    }

    #[test]
    fn guardian_modules_cannot_be_engineered() {
        let gpp = DATA.modules.standard["pp"]
            .iter()
            .find(|m| m.name.as_deref() == Some("Guardian Hybrid Power Plant"))
            .expect("guardian pp");
        assert!(is_guardian(gpp) && engineering_options(gpp).is_empty());
        let gsrp = &DATA.modules.internal["gsrp"][0];
        assert!(engineering_options(gsrp).is_empty());
        let pp = DATA.modules.standard["pp"]
            .iter()
            .find(|m| m.class == 4 && m.rating == "A" && !is_guardian(m))
            .expect("plain 4A pp");
        assert!(!engineering_options(pp).is_empty());
    }

    #[test]
    fn pre_engineered_modules_only_take_experimentals() {
        let v1 = DATA.modules.standard["fsd"]
            .iter()
            .find(|m| m.id == "5O")
            .expect("V1 FSD");
        let opts = engineering_options(v1);
        assert!(!opts.is_empty() && opts.iter().all(|o| o.special.is_some()));
    }

    #[test]
    fn everything_is_allowed_by_default_and_can_be_excluded() {
        let cg = |f: &Filters| {
            let s = ship("type_9_heavy");
            candidates(&s, &s.internal[0], 0, f, None)
                .into_iter()
                .any(|m| {
                    m.pre_engineered
                        .as_ref()
                        .is_some_and(|p| p.availability.as_deref() == Some("CG"))
                })
        };
        assert!(cg(&Filters::default()));
        assert!(!cg(&Filters {
            allow_community_goal: false,
            ..Filters::default()
        }));
    }
}
