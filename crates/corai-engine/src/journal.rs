//! Frontier journal `Loadout` events.
//!
//! Import is a port of Coriolis' `JournalUtils.shipFromLoadoutJSON` (including its quirks, so the
//! result matches what Coriolis builds), followed by corai's correction pass that sets every
//! engineered attribute to the exact journal value. Export writes the same event, with the exact
//! engineered values as `Modifiers`.

use corai_data::{DATA, ModifierAction, ModuleDef};
use serde_json::{Map, Value, json};

use crate::jsmath::js_round;
use crate::module::{AppliedBlueprint, AppliedSpecial, Module, find_blueprint, find_special};
use crate::ship::{STANDARD_GROUPS, Ship, Slot, SlotKind};

/// Frontier ship names (Coriolis' `SHIP_FD_NAME_TO_CORIOLIS_NAME`).
const SHIP_FD_NAMES: &[(&str, &str)] = &[
    ("Adder", "adder"),
    ("Anaconda", "anaconda"),
    ("AnacondaMkII", "anaconda2"),
    ("Asp", "asp"),
    ("Asp_Scout", "asp_scout"),
    ("BelugaLiner", "beluga"),
    ("Explorer_NX", "explorer_nx"),
    ("CobraMkIII", "cobra_mk_iii"),
    ("CobraMkIV", "cobra_mk_iv"),
    ("CobraMkV", "cobramkv"),
    ("Corsair", "imperial_corsair"),
    ("Cutter", "imperial_cutter"),
    ("DiamondBackXL", "diamondback_explorer"),
    ("DiamondBack", "diamondback"),
    ("Dolphin", "dolphin"),
    ("Eagle", "eagle"),
    ("Empire_Courier", "imperial_courier"),
    ("Empire_Eagle", "imperial_eagle"),
    ("Empire_Trader", "imperial_clipper"),
    ("Federation_Corvette", "federal_corvette"),
    ("Federation_Dropship", "federal_dropship"),
    ("Federation_Dropship_MkII", "federal_assault_ship"),
    ("Federation_Gunship", "federal_gunship"),
    ("FerDeLance", "fer_de_lance"),
    ("Hauler", "hauler"),
    ("Independant_Trader", "keelback"),
    ("smallcombat01_nx", "kestrel"),
    ("Krait_MkII", "krait_mkii"),
    ("Mamba", "mamba"),
    ("Mandalay", "mandalay"),
    ("mediumtransport01", "lynx_highliner"),
    ("Krait_Light", "krait_phantom"),
    ("Orca", "orca"),
    ("PantherMKII", "panthermkii"),
    ("Python", "python"),
    ("Python_nx", "python_nx"),
    ("SideWinder", "sidewinder"),
    ("Type6", "type_6_transporter"),
    ("Type7", "type_7_transport"),
    ("Type8", "type_8_transport"),
    ("Type9", "type_9_heavy"),
    ("Type9_Military", "type_10_defender"),
    ("LakonMiner", "type_11_prospector"),
    ("TypeX", "alliance_chieftain"),
    ("TypeX_2", "alliance_crusader"),
    ("TypeX_3", "alliance_challenger"),
    ("Viper", "viper"),
    ("Viper_MkIV", "viper_mk_iv"),
    ("Vulture", "vulture"),
];

const STANDARD_SLOTS: [&str; 7] = [
    "PowerPlant",
    "MainEngines",
    "FrameShiftDrive",
    "LifeSupport",
    "PowerDistributor",
    "Radar",
    "FuelTank",
];
const HARDPOINT_SIZES: [&str; 5] = ["Tiny", "Small", "Medium", "Large", "Huge"];

/// Errors reading a Loadout. Messages match the Node version.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum JournalError {
    #[error("Not a Loadout: missing \"Modules\".")]
    NotALoadout,
    #[error("No such ship found: \"{0}\"")]
    UnknownShip(String),
    #[error("Unknown bulkheads \"{0}\"")]
    UnknownBulkheads(String),
}

/// Coriolis ship id for a Frontier ship name (case-insensitive), or a Coriolis id given directly.
pub fn ship_id_from_fd(name: &str) -> Option<&'static str> {
    SHIP_FD_NAMES
        .iter()
        .find(|(fd, _)| fd.eq_ignore_ascii_case(name))
        .map(|(_, id)| *id)
        .or_else(|| {
            let lower = name.to_lowercase();
            DATA.ships.get_key_value(&lower).map(|(k, _)| k.as_str())
        })
}

/// Frontier ship name for a Coriolis id (the id itself when unknown).
pub fn fd_ship_name(ship_id: &str) -> &str {
    SHIP_FD_NAMES
        .iter()
        .find(|(_, id)| *id == ship_id)
        .map_or(ship_id, |(fd, _)| fd)
}

/// `_moduleFromFdName`: the first module (standard, then hardpoints, then internal, in data order)
/// whose symbol matches, case-insensitively.
pub fn module_from_fd_name(fd: &str) -> Option<&'static ModuleDef> {
    let data = &*DATA;
    [
        &data.modules.standard,
        &data.modules.hardpoints,
        &data.modules.internal,
    ]
    .into_iter()
    .flat_map(|groups| groups.values().flatten())
    .find(|m| {
        m.symbol
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case(fd))
    })
}

/// The authoritative slot table of a ship (keyed by Coriolis id or lowercased Frontier name).
fn ship_slot_table(ship_id: &str) -> Option<&'static corai_data::NamedSlotGroups> {
    let data = &*DATA;
    data.ship_slots
        .get(ship_id)
        .or_else(|| data.ship_slots.get(&fd_ship_name(ship_id).to_lowercase()))
        .and_then(|t| t.slots.as_ref())
}

/// Frontier slot names for a hull's hardpoints and internals, in Coriolis slot order (the names
/// corai exports; port of `journalSlotNames`).
pub fn journal_slot_names(ship_id: &str) -> (Vec<String>, Vec<Option<String>>) {
    let template = &DATA.ships[ship_id];
    if let Some(t) = ship_slot_table(ship_id)
        && t.hardpoints.len() == template.slots.hardpoints.len()
        && t.internal.len() == template.slots.internal.len()
    {
        return (
            t.hardpoints.iter().map(|s| s.name.clone()).collect(),
            t.internal.iter().map(|s| Some(s.name.clone())).collect(),
        );
    }
    let mut counters = [0u32; 5];
    let hardpoints = template
        .slots
        .hardpoints
        .iter()
        .map(|def| {
            let cls = usize::from(def.class());
            let prefix = def.name().unwrap_or_default();
            counters[cls] += 1;
            if ship_id == "type_8_transport" && cls == 1 && counters[cls] == 3 {
                counters[cls] += 1;
            }
            format!("{}{prefix}Hardpoint{}", HARDPOINT_SIZES[cls], counters[cls])
        })
        .collect();
    let mut n = if ship_id == "type_9_heavy" { 0 } else { 1 };
    let mut named: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let internal = template
        .slots
        .internal
        .iter()
        .map(|def| {
            let name = def.name();
            if name == Some("PlanetaryApproachSuite") {
                return Some("PlanetaryApproachSuite".to_string());
            }
            let prefix = match name {
                Some("Military") => Some("Military"),
                Some("Cargo") => Some("Cargo"),
                Some("Limpets") => Some("LimpetController"),
                Some("Fighter") => Some("FighterBay"),
                _ => None,
            };
            if let (Some(name), Some(prefix)) = (name, prefix) {
                let k = named.entry(name).or_insert(0);
                *k += 1;
                return Some(format!("{prefix}0{k}"));
            }
            // The game skips slot numbers on some hulls.
            if (n == 11 && ship_id == "anaconda")
                || (n == 9 && (ship_id == "type_9_heavy" || ship_id == "type_10_defender"))
            {
                n += 2;
            } else if n == 4 && ship_id == "vulture" {
                n += 1;
            }
            let out = format!("Slot{n:02}_Size{}", def.class());
            n += 1;
            Some(out)
        })
        .collect();
    (hardpoints, internal)
}

// ---------------------------------------------------------------- import (Coriolis)

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// JS truthiness of a JSON value.
fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn priority_of(v: &Value) -> usize {
    v.get("Priority")
        .and_then(Value::as_f64)
        .map_or(0, |p| p.max(0.0) as usize)
}

fn place(slot: &mut Slot, def: Option<&'static ModuleDef>) {
    Ship::place(slot, def);
}

/// Modifier action table for a journal label (Coriolis' `findMod`: compares names without the
/// `OutfittingFieldType_` prefix and `PerSecond`, case-insensitively).
fn modifier_actions_for(
    label: &str,
) -> Option<&'static indexmap::IndexMap<String, ModifierAction>> {
    let norm = |s: &str| {
        s.to_lowercase()
            .replace("outfittingfieldtype_", "")
            .replace("persecond", "")
    };
    let want = norm(label);
    DATA.modifications
        .modifier_actions
        .iter()
        .find(|(k, _)| norm(k) == want)
        .map(|(_, v)| v)
}

/// `_addModifications(module, Modifiers, Quality, BlueprintName, Level, ExperimentalEffect)`.
fn add_modifications(m: &mut Module, eng: &Value) {
    let modifiers = eng.get("Modifiers");
    let quality = eng.get("Quality");
    if !js_truthy(modifiers) && !js_truthy(quality) {
        return;
    }
    let mut special_key = str_of(eng, "ExperimentalEffect").to_string();
    if special_key == "special_plasma_slug" {
        let symbol = m.def.symbol.as_deref().unwrap_or_default().to_lowercase();
        special_key = if symbol.contains("plasmaaccelerator") {
            "special_plasma_slug_pa".into()
        } else {
            "special_plasma_slug_cooled".into()
        };
    }
    let special: Option<AppliedSpecial> = DATA
        .modifications
        .specials
        .get_key_value(&special_key)
        .map(|(k, def)| AppliedSpecial {
            key: k.as_str(),
            def,
        });
    let blueprint_name = str_of(eng, "BlueprintName");
    if !blueprint_name.is_empty() {
        let grade = eng
            .get("Level")
            .and_then(Value::as_f64)
            .filter(|g| *g != 0.0)
            .map_or(0, |g| g as u8);
        let blueprint = find_blueprint(blueprint_name, m.def).map(|(key, def)| AppliedBlueprint {
            key,
            def,
            grade,
            coded: true,
        });
        // A fresh blueprint record: the experimental is the journal's, or none.
        m.set_engineering_records(blueprint, special);
    }
    if let Some(list) = modifiers.and_then(Value::as_array) {
        for mf in list {
            let label = str_of(mf, "Label");
            let v = mf.get("Value").and_then(Value::as_f64).unwrap_or(f64::NAN);
            let o = mf
                .get("OriginalValue")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NAN);
            let mut value = (v / o * 100.0 - 100.0) * 100.0;
            if value == f64::INFINITY {
                value = v * 100.0;
            }
            if label.contains("DamageFalloffRange") {
                value = (v / m.base("range").unwrap_or(f64::NAN) - 1.0) * 100.0;
            }
            if label.contains("Resistance") {
                value = v * 100.0 - o * 100.0;
            }
            if label.contains("ShieldMultiplier")
                || label.contains("DefenceModifierHealthMultiplier")
            {
                value = ((100.0 + v) / (100.0 + o) * 100.0 - 100.0) * 100.0;
            }
            let Some(actions) = modifier_actions_for(label) else {
                continue;
            };
            for (attr, action) in actions {
                match action {
                    ModifierAction::DamageDist(d) => m.set_damagedist_mod(d),
                    ModifierAction::Value(_) => m.set_mod_value_js(attr, value, true),
                }
            }
        }
    } else if let Some(q) = quality.and_then(Value::as_f64)
        && let Some(b) = m.blueprint().cloned()
    {
        m.apply_quality_unchecked(b.def, b.grade, q);
    }
}

fn engineering_of(v: &Value) -> Option<&Value> {
    v.get("Engineering").filter(|e| !e.is_null())
}

/// `shipFromLoadoutJSON`: the ship Coriolis builds from a Loadout event.
///
/// # Errors
/// No `Modules`, unknown ship or unknown bulkheads.
pub fn coriolis_ship_from_loadout(json: &Value) -> Result<Ship, JournalError> {
    let modules = json
        .get("Modules")
        .and_then(Value::as_array)
        .ok_or(JournalError::NotALoadout)?;
    let ed_name = json
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| str_of(json, "Ship"));
    let ship_id =
        ship_id_from_fd(ed_name).ok_or_else(|| JournalError::UnknownShip(ed_name.into()))?;
    let mut ship = Ship::empty(ship_id).map_err(|_| JournalError::UnknownShip(ed_name.into()))?;
    let template = &DATA.ships[ship_id];
    let has_hardpoint_entries = modules
        .iter()
        .any(|m| str_of(m, "Slot").to_lowercase().contains("hardpoint"));

    for module in modules {
        let slot = str_of(module, "Slot").to_lowercase();
        let on = module.get("On").and_then(Value::as_bool).unwrap_or(false);
        match slot.as_str() {
            "cargohatch" => {
                ship.hatch_enabled = on;
                ship.hatch_priority = priority_of(module);
            }
            "armour" => {
                let item = str_of(module, "Item").to_lowercase();
                let offset = usize::from(template.bulkheads.len() > 5);
                let index = if item.ends_with("_armour_grade1_default") {
                    0
                } else if item.ends_with("_armour_grade1") {
                    offset
                } else if item.ends_with("_armour_grade2") {
                    1 + offset
                } else if item.ends_with("_armour_grade3") {
                    2 + offset
                } else if item.ends_with("_armour_mirrored") {
                    3 + offset
                } else if item.ends_with("_armour_reactive") {
                    4 + offset
                } else {
                    return Err(JournalError::UnknownBulkheads(
                        str_of(module, "Item").into(),
                    ));
                };
                ship.use_bulkhead(index)
                    .map_err(|_| JournalError::UnknownBulkheads(str_of(module, "Item").into()))?;
                if let Some(eng) = engineering_of(module) {
                    add_modifications(&mut ship.bulkhead, eng);
                }
            }
            _ => {
                let standard = [
                    ("powerplant", 0, "Int_Missing_Powerplant"),
                    ("mainengines", 1, "Int_Missing_Engine"),
                    ("frameshiftdrive", 2, "Int_Missing_Hyperdrive"),
                    ("lifesupport", 3, "Int_Missing_LifeSupport"),
                    ("powerdistributor", 4, "Int_Missing_PowerDistributor"),
                    ("radar", 5, "Int_Missing_Sensors"),
                    ("fueltank", 6, "Int_Missing_FuelTank"),
                ];
                if let Some(&(_, index, missing)) = standard.iter().find(|(n, _, _)| *n == slot) {
                    let found = module_from_fd_name(str_of(module, "Item"));
                    let def = found.or_else(|| module_from_fd_name(missing));
                    let s = &mut ship.standard[index];
                    place(s, def);
                    if index == 6 {
                        s.enabled = true;
                        s.priority = 0;
                    } else {
                        // Life support is only on when `On === true`; the others take `On` as is.
                        s.enabled = on;
                        s.priority = priority_of(module);
                        if found.is_some()
                            && let (Some(m), Some(eng)) =
                                (s.module.as_mut(), engineering_of(module))
                        {
                            add_modifications(m, eng);
                        }
                    }
                }
            }
        }
    }

    // Hardpoints: placed once per hardpoint entry in Coriolis; the last pass is what remains, and
    // its modules get their engineering once.
    let mut deferred: Vec<(SlotKind, usize, &Value)> = Vec::new();
    if has_hardpoint_entries {
        let (names, _) = coriolis_hardpoint_names(ship_id);
        for (i, name) in names.iter().enumerate() {
            let Some(entry) = modules
                .iter()
                .find(|m| str_of(m, "Slot").eq_ignore_ascii_case(name))
            else {
                continue;
            };
            let found = module_from_fd_name(str_of(entry, "Item"));
            let def = found.or_else(|| {
                let tiny_first = str_of(entry, "Slot").to_lowercase().find("tiny") == Some(0);
                module_from_fd_name(if tiny_first {
                    "Hpt_Missing_Utility"
                } else {
                    "Hpt_Missing_Hardpoint"
                })
            });
            let s = &mut ship.hardpoints[i];
            place(s, def);
            s.enabled = entry.get("On").and_then(Value::as_bool).unwrap_or(false);
            s.priority = priority_of(entry);
            if found.is_some() {
                deferred.push((SlotKind::Hardpoint, i, entry));
            }
        }
    }

    // Internals, by authoritative slot name (or Coriolis' reconstruction).
    let names = coriolis_internal_names(ship_id);
    for (i, name) in names.iter().enumerate() {
        let planetary = template.slots.internal[i].name() == Some("PlanetaryApproachSuite");
        let entry = name.as_deref().and_then(|n| {
            modules
                .iter()
                .find(|m| str_of(m, "Slot").eq_ignore_ascii_case(n))
        });
        let Some(entry) = entry else {
            if planetary {
                let apas = module_from_fd_name("int_planetapproachsuite_advanced");
                if apas.is_some() {
                    place(&mut ship.internal[i], apas);
                }
            }
            continue;
        };
        let mut found = module_from_fd_name(str_of(entry, "Item"));
        let mut engineering = engineering_of(entry);
        // An Expanded Capacity cargo rack is the pre-engineered rack of that class.
        if let (Some(m), Some(eng)) = (found, engineering)
            && str_of(eng, "BlueprintName").eq_ignore_ascii_case("cargorack_increasedcapacity")
            && let Some(rack) = DATA.modules.internal.get("cr").and_then(|racks| {
                racks.iter().find(|r| {
                    r.class == m.class
                        && r.pre_engineered.as_ref().is_some_and(|pe| {
                            pe.blueprints
                                .iter()
                                .any(|b| b == "CargoRack_IncreasedCapacity")
                        })
                })
            })
        {
            found = Some(rack);
            engineering = None;
        }
        let def = found.or_else(|| module_from_fd_name("Int_Missing_Module"));
        let s = &mut ship.internal[i];
        place(s, def);
        s.enabled = entry.get("On").and_then(Value::as_bool) == Some(true);
        s.priority = priority_of(entry);
        if found.is_some() && engineering.is_some() {
            deferred.push((SlotKind::Internal, i, entry));
        }
    }

    for (kind, i, entry) in deferred {
        let slot = if kind == SlotKind::Internal {
            &mut ship.internal[i]
        } else {
            &mut ship.hardpoints[i]
        };
        let is_rack_swap = kind == SlotKind::Internal
            && engineering_of(entry).is_some_and(|e| {
                str_of(e, "BlueprintName").eq_ignore_ascii_case("cargorack_increasedcapacity")
            });
        if let (Some(m), Some(eng)) = (slot.module.as_mut(), engineering_of(entry))
            && !is_rack_swap
        {
            add_modifications(m, eng);
        }
    }
    Ok(ship)
}

/// Coriolis' importer naming for hardpoints: the authoritative table, else its reconstruction.
fn coriolis_hardpoint_names(ship_id: &str) -> (Vec<String>, bool) {
    let template = &DATA.ships[ship_id];
    if let Some(t) = ship_slot_table(ship_id)
        && t.hardpoints.len() == template.slots.hardpoints.len()
    {
        return (t.hardpoints.iter().map(|s| s.name.clone()).collect(), true);
    }
    let mut counters = [0u32; 5];
    let names = template
        .slots
        .hardpoints
        .iter()
        .map(|def| {
            let cls = usize::from(def.class());
            counters[cls] += 1;
            if ship_id == "type_8_transport" && cls == 1 && counters[cls] == 3 {
                counters[cls] += 1;
            }
            format!(
                "{}{}Hardpoint{}",
                HARDPOINT_SIZES[cls],
                def.name().unwrap_or_default(),
                counters[cls]
            )
        })
        .collect();
    (names, false)
}

/// Coriolis' importer naming for internals: the authoritative table, else its reconstruction.
fn coriolis_internal_names(ship_id: &str) -> Vec<Option<String>> {
    let template = &DATA.ships[ship_id];
    if let Some(t) = ship_slot_table(ship_id)
        && t.internal.len() == template.slots.internal.len()
    {
        return t.internal.iter().map(|s| Some(s.name.clone())).collect();
    }
    let mut n = if ship_id == "type_9_heavy" { 0 } else { 1 };
    let (mut military, mut cargo, mut limpets, mut fighter, mut passenger) = (1, 1, 1, 1, 1);
    template
        .slots
        .internal
        .iter()
        .map(|def| {
            let numbered = |prefix: &str, k: &mut u32| {
                let s = format!("{prefix}0{k}");
                *k += 1;
                Some(s)
            };
            match def.name() {
                Some("Military") => numbered("Military", &mut military),
                Some("PlanetaryApproachSuite") => Some("PlanetaryApproachSuite".into()),
                Some("Cargo") => numbered("Cargo", &mut cargo),
                Some("Limpets") => numbered("LimpetController", &mut limpets),
                Some("Fighter") => numbered("FighterBay", &mut fighter),
                Some("MkIIPassenger") => numbered("Passenger", &mut passenger),
                _ => {
                    if (n == 11 && ship_id == "anaconda")
                        || (n == 7 && ship_id == "federal_dropship")
                        || (n == 9 && (ship_id == "type_9_heavy" || ship_id == "type_10_defender"))
                    {
                        n += 2;
                    } else if n == 4 && ship_id == "vulture" {
                        n += 1;
                    }
                    let s = format!("Slot{n:02}_Size{}", def.class());
                    n += 1;
                    Some(s)
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------- import (corai correction pass)

/// Coriolis attributes a journal modifier label maps to (exact label, case-insensitive).
fn label_attributes(label: &str) -> Vec<&'static str> {
    let want = format!("outfittingfieldtype_{label}").to_lowercase();
    DATA.modifications
        .modifier_actions
        .iter()
        .find(|(k, _)| k.to_lowercase() == want)
        .map(|(_, actions)| actions.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

/// The imported module in the journal's slot (several slots often hold the same item).
fn find_imported<'a>(ship: &'a mut Ship, slot: &str) -> Option<&'a mut Module> {
    let slot = slot.to_lowercase();
    if slot == "armour" {
        return Some(&mut ship.bulkhead);
    }
    if let Some(i) = STANDARD_SLOTS.iter().position(|n| n.to_lowercase() == slot) {
        return ship.standard.get_mut(i).and_then(|s| s.module.as_mut());
    }
    let (hardpoints, internal) = journal_slot_names(ship.id);
    if let Some(i) = hardpoints.iter().position(|n| n.to_lowercase() == slot) {
        return ship.hardpoints.get_mut(i).and_then(|s| s.module.as_mut());
    }
    if let Some(i) = internal
        .iter()
        .position(|n| n.as_deref().is_some_and(|n| n.to_lowercase() == slot))
    {
        return ship.internal.get_mut(i).and_then(|s| s.module.as_mut());
    }
    None
}

fn matches(got: Option<f64>, want: f64) -> bool {
    got.is_some_and(|g| (g - want).abs() <= 1e-6 * want.abs().max(1.0))
}

/// Sets the stored modification of `attr` so that `get(attr)` equals `target`, by bisection
/// (Coriolis' `set()` misses for some attributes); leaves it unchanged when no bracket holds it.
fn solve_modification(m: &mut Module, attr: &str, target: f64) {
    let before = m.mod_value(attr, true).unwrap_or(0.0);
    let at = |m: &mut Module, x: f64| {
        m.set_mod_value_js(attr, x, true);
        m.get(attr)
            .filter(|v| v.is_finite())
            .map_or(f64::NAN, |v| v - target)
    };
    let sign = |x: f64| {
        if x > 0.0 {
            1
        } else if x < 0.0 {
            -1
        } else {
            0
        }
    };
    let edges = [-9999.0, -5000.0, 0.0, 5000.0, 50000.0, 1e6];
    for w in edges.windows(2) {
        let (mut lo, mut hi) = (w[0], w[1]);
        let (mut flo, fhi) = (at(m, lo), at(m, hi));
        if !(flo.is_finite() && fhi.is_finite()) || sign(flo) == sign(fhi) {
            continue;
        }
        for _ in 0..80 {
            if hi - lo <= 1e-4 {
                break;
            }
            let mid = (lo + hi) / 2.0;
            let fm = at(m, mid);
            if sign(fm) == sign(flo) {
                lo = mid;
                flo = fm;
            } else {
                hi = mid;
            }
        }
        // Coriolis stores whole numbers; take the closer of the two neighbours.
        let a = ((lo + hi) / 2.0).floor();
        let best = if at(m, a).abs() <= at(m, a + 1.0).abs() {
            a
        } else {
            a + 1.0
        };
        m.set_mod_value_js(attr, best, true);
        return;
    }
    m.set_mod_value_js(attr, before, true);
}

/// Sets each engineered attribute to exactly the journal's final value.
fn apply_journal_modifiers(ship: &mut Ship, loadout: &Value) {
    let Some(modules) = loadout.get("Modules").and_then(Value::as_array) else {
        return;
    };
    for entry in modules {
        let Some(eng) = engineering_of(entry) else {
            continue;
        };
        let Some(modifiers) = eng
            .get("Modifiers")
            .and_then(Value::as_array)
            .filter(|l| !l.is_empty())
        else {
            continue;
        };
        let Some(m) = find_imported(ship, str_of(entry, "Slot")) else {
            continue;
        };
        let blueprint_name = str_of(eng, "BlueprintName");
        if m.blueprint().is_none()
            && !blueprint_name.is_empty()
            && let Some((key, def)) = find_blueprint(blueprint_name, m.def)
        {
            let grade = eng
                .get("Level")
                .and_then(Value::as_f64)
                .filter(|g| *g != 0.0)
                .map_or(0, |g| g as u8);
            let special = m.special();
            m.set_engineering_records(
                Some(AppliedBlueprint {
                    key,
                    def,
                    grade,
                    coded: true,
                }),
                special,
            );
        }
        let effect = str_of(eng, "ExperimentalEffect");
        if !effect.is_empty() && m.blueprint().is_some() && m.special().is_none() {
            let special = find_special(effect);
            let blueprint = m.blueprint().cloned();
            m.set_engineering_records(blueprint, special);
        }
        for mf in modifiers {
            let Some(value) = mf.get("Value").and_then(Value::as_f64) else {
                continue;
            };
            for attr in label_attributes(str_of(mf, "Label")) {
                // The journal value already includes any experimental effect.
                m.set_attribute(attr, value, true);
                if !matches(m.get(attr), value) {
                    solve_modification(m, attr, value);
                }
            }
        }
    }
}

/// Engineered values the imported ship does not reproduce (each is an import defect).
pub fn import_discrepancies(ship: &mut Ship, loadout: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(modules) = loadout.get("Modules").and_then(Value::as_array) else {
        return out;
    };
    for entry in modules {
        let Some(modifiers) = engineering_of(entry)
            .and_then(|e| e.get("Modifiers"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let slot = str_of(entry, "Slot");
        for mf in modifiers {
            let Some(m) = find_imported(ship, slot) else {
                continue;
            };
            let label = str_of(mf, "Label");
            for attr in label_attributes(label) {
                let (Some(want), Some(got)) =
                    (mf.get("Value").and_then(Value::as_f64), m.get(attr))
                else {
                    continue;
                };
                if (got - want).abs() > 1e-3 * want.abs().max(1.0) {
                    out.push(format!(
                        "{slot} {label}: game {}, imported {}",
                        js_number(want),
                        js_number(js_round(got * 1e4) / 1e4)
                    ));
                }
            }
        }
    }
    out
}

/// A number as JavaScript prints it (integers without a decimal point).
fn js_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e21 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// A Loadout event as corai reads it: Coriolis' import, then exact journal values, then the
/// discrepancy check. Returns the ship and its import issues.
///
/// # Errors
/// See [`coriolis_ship_from_loadout`].
pub fn ship_from_loadout(loadout: &Value) -> Result<(Ship, Vec<String>), JournalError> {
    let mut ship = coriolis_ship_from_loadout(loadout)?;
    apply_journal_modifiers(&mut ship, loadout);
    ship.recalc();
    let issues = import_discrepancies(&mut ship, loadout);
    Ok((ship, issues))
}

// ---------------------------------------------------------------- export

/// Coriolis `getPercent`: the roll quality (0..100) all engineered features agree on; `None` when
/// they disagree.
fn quality_percent(m: &Module) -> Option<f64> {
    let data = &*DATA;
    let Some(b) = m.blueprint() else {
        return Some(0.0);
    };
    let Some(grade) = b.def.grades.get(b.grade.to_string().as_str()) else {
        return Some(0.0);
    };
    let mut result: Option<f64> = None;
    for (name, feature) in &grade.features {
        let corai_data::Feature::Range([a, z]) = *feature else {
            continue;
        };
        if a == z {
            continue;
        }
        let Some(def) = data.modifications.modifications.get(name) else {
            continue;
        };
        let raw = m.mod_value(name, true).unwrap_or(0.0);
        let value = match def.kind {
            corai_data::ModificationKind::Percentage => raw / 10000.0,
            corai_data::ModificationKind::Numeric => raw / 100.0,
            corai_data::ModificationKind::Object => raw,
        };
        let worse_first = a < 0.0 || (a == 0.0 && z < 0.0);
        let mult = js_round(match (def.higherbetter, worse_first) {
            (true, true) | (false, false) => (value - z) / (a - z) * 100.0,
            (true, false) | (false, true) => (value - a) / (z - a) * 100.0,
        });
        match result {
            Some(r) if r != 0.0 && r != mult => return None,
            Some(r) if r == mult => {}
            _ => result = Some(mult),
        }
    }
    result
}

/// Journal modifier label for a Coriolis attribute (first match in the data's modifierActions).
fn fd_label(attr: &str) -> Option<String> {
    DATA.modifications
        .modifier_actions
        .iter()
        .filter(|(k, _)| !k.starts_with("special_") && !k.starts_with("trade_"))
        .find(|(_, actions)| actions.contains_key(attr))
        .map(|(k, _)| k.trim_start_matches("OutfittingFieldType_").to_string())
}

fn round6(x: f64) -> f64 {
    js_round(x * 1e6) / 1e6
}

/// A number that serialises like JavaScript's (whole numbers without `.0`).
fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9e15 {
        Value::from(x as i64)
    } else {
        Value::from(x)
    }
}

fn modifiers(m: &Module) -> Option<Value> {
    let data = &*DATA;
    let out: Vec<Value> = m
        .mods()
        .keys()
        .filter_map(|attr| {
            let def = data.modifications.modifications.get(attr)?;
            if def.hidden || def.kind == corai_data::ModificationKind::Object {
                return None;
            }
            let label = fd_label(attr)?;
            let value = m.get(attr).filter(|v| v.is_finite())?;
            let base = m.base(attr).filter(|v| !v.is_nan()).unwrap_or(0.0);
            Some(json!({
                "Label": label,
                "Value": num(round6(value)),
                "OriginalValue": num(round6(base)),
                "LessIsGood": u8::from(!def.higherbetter),
            }))
        })
        .collect();
    (!out.is_empty()).then(|| Value::Array(out))
}

fn engineering(m: &Module) -> Option<Value> {
    let b = m.blueprint()?;
    let mut e = Map::new();
    e.insert("BlueprintName".into(), b.def.fdname.clone().into());
    e.insert(
        "Level".into(),
        Value::from(if b.grade == 0 { 1 } else { b.grade }),
    );
    let quality = quality_percent(m)
        .filter(|q| q.is_finite())
        .map_or(1.0, |q| js_round(q * 100.0) / 10000.0);
    e.insert("Quality".into(), num(quality));
    if let Some(s) = m.special() {
        e.insert("ExperimentalEffect".into(), s.def.edname.clone().into());
    }
    if let Some(mods) = modifiers(m) {
        e.insert("Modifiers".into(), mods);
    }
    Some(Value::Object(e))
}

fn entry(slot: &str, m: &Module, enabled: bool, priority: usize) -> Value {
    let mut e = Map::new();
    e.insert("Slot".into(), slot.into());
    e.insert(
        "Item".into(),
        m.def
            .symbol
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .into(),
    );
    e.insert("On".into(), enabled.into());
    e.insert("Priority".into(), priority.into());
    if let Some(eng) = engineering(m) {
        e.insert("Engineering".into(), eng);
    }
    Value::Object(e)
}

fn bulkhead_item(ship: &Ship) -> String {
    let all = &DATA.ships[ship.id].bulkheads;
    let index = all
        .iter()
        .position(|b| b.id == ship.bulkhead.def.id)
        .unwrap_or(0);
    let suffixes: &[&str] = if all.len() > 5 {
        &[
            "grade1_default",
            "grade1",
            "grade2",
            "grade3",
            "mirrored",
            "reactive",
        ]
    } else {
        &["grade1", "grade2", "grade3", "mirrored", "reactive"]
    };
    format!(
        "{}_armour_{}",
        fd_ship_name(ship.id).to_lowercase(),
        suffixes.get(index).copied().unwrap_or("grade1")
    )
}

/// The build as a Frontier journal Loadout event.
pub fn to_loadout(ship: &Ship, name: &str) -> Value {
    let (hardpoint_names, internal_names) = journal_slot_names(ship.id);
    let mut modules = Vec::new();
    let mut bh = Map::new();
    bh.insert("Slot".into(), "Armour".into());
    bh.insert("Item".into(), bulkhead_item(ship).into());
    bh.insert("On".into(), true.into());
    bh.insert("Priority".into(), 1.into());
    if let Some(eng) = engineering(&ship.bulkhead) {
        bh.insert("Engineering".into(), eng);
    }
    modules.push(Value::Object(bh));
    modules.push(json!({ "Slot": "CargoHatch", "Item": "modularcargobaydoor", "On": ship.hatch_enabled, "Priority": ship.hatch_priority }));
    for (i, slot) in ship.standard.iter().enumerate() {
        if let Some(m) = slot.module.as_ref().filter(|m| m.def.symbol.is_some())
            && i < STANDARD_GROUPS.len()
        {
            modules.push(entry(STANDARD_SLOTS[i], m, slot.enabled, slot.priority));
        }
    }
    for (slot, name) in ship.hardpoints.iter().zip(&hardpoint_names) {
        if let Some(m) = slot.module.as_ref().filter(|m| m.def.symbol.is_some()) {
            modules.push(entry(name, m, slot.enabled, slot.priority));
        }
    }
    for (slot, name) in ship.internal.iter().zip(&internal_names) {
        if let (Some(m), Some(name)) = (
            slot.module.as_ref().filter(|m| m.def.symbol.is_some()),
            name,
        ) {
            modules.push(entry(name, m, slot.enabled, slot.priority));
        }
    }
    json!({
        "event": "Loadout",
        "Ship": fd_ship_name(ship.id).to_lowercase(),
        "ShipName": name,
        "ShipIdent": "",
        "Modules": modules,
    })
}
