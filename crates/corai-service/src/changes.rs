//! Slot-level edits to a build ("what if I swap this module / blueprint?"), port of `changes.ts`.

use corai_data::{DATA, ModuleDef};
use corai_engine::{Module, Ship};
use corai_opt::catalog::{Filters, SlotRef, candidates};
use serde::{Deserialize, Serialize};

use crate::error::ServiceError;

/// One change; fields in the Node schema order (error messages print it as JSON).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildChange {
    /// Slot key from ship_details, e.g. internal:3, hardpoint:1, utility:2, standard:fsd, bulkheads.
    pub slot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special: Option<String>,
}

/// Highest grade a blueprint defines for a group (`maxBlueprintGrade`; 0 when none).
pub fn max_blueprint_grade(grp: &str, blueprint: &str) -> u8 {
    let data = &*DATA;
    let numeric =
        |keys: &mut dyn Iterator<Item = &String>| keys.filter_map(|k| k.parse::<u8>().ok()).max();
    data.modifications
        .modules
        .get(grp)
        .and_then(|g| g.blueprints.get(blueprint))
        .map(|b| numeric(&mut b.grades.keys()))
        .or_else(|| {
            data.modifications
                .blueprints
                .get(blueprint)
                .map(|b| numeric(&mut b.grades.keys()))
        })
        .flatten()
        .unwrap_or(0)
}

/// `applyEngineering`: blueprint at best roll plus an optional experimental; pre-engineered modules
/// only take an experimental they allow.
fn engineer(
    m: &mut Module,
    blueprint: &str,
    grade: u8,
    special: Option<&str>,
) -> Result<(), ServiceError> {
    let special = special.filter(|s| !s.is_empty());
    if let Some(s) = special
        && !DATA.modifications.specials.contains_key(s)
    {
        return Err(ServiceError::UnknownSpecial(s.into()));
    }
    if let Some(pe) = &m.def.pre_engineered {
        if special.is_some() && pe.can_apply_experimental && m.blueprint().is_some() {
            m.set_special(special)
                .map_err(|_| ServiceError::UnknownSpecial(special.unwrap_or_default().into()))?;
        }
        return Ok(());
    }
    m.apply_blueprint(blueprint, grade, 1.0)
        .map_err(|e| match e {
            corai_engine::EngineeringError::UnknownGrade { .. } => ServiceError::UnknownGrade {
                blueprint: blueprint.into(),
                grade,
            },
            _ => ServiceError::UnknownBlueprint(blueprint.into()),
        })?;
    m.set_special(special)
        .map_err(|_| ServiceError::UnknownSpecial(special.unwrap_or_default().into()))
}

/// Applies one change to the ship and recalculates it.
///
/// # Errors
/// Unknown slot, nothing matching fits, or unknown engineering.
pub fn apply_change(ship: &mut Ship, c: &BuildChange) -> Result<(), ServiceError> {
    let r = SlotRef::parse(ship, &c.slot).ok_or_else(|| ServiceError::UnknownSlot {
        slot: c.slot.clone(),
        ship: ship.def.properties.name.clone(),
    })?;
    if c.empty == Some(true) {
        if let Some(slot) = r.slot_mut(ship) {
            Ship::place(slot, None);
        }
        ship.recalc();
        return Ok(());
    }
    if r == SlotRef::Bulkheads {
        let index = c.bulkhead_index.unwrap_or(0);
        ship.use_bulkhead(index)
            .map_err(|_| ServiceError::NoModuleFits {
                change: change_json(c),
                slot: c.slot.clone(),
            })?;
        if let Some(bp) = c.blueprint.as_deref() {
            let grade = c.grade.unwrap_or_else(|| max_blueprint_grade("bh", bp));
            engineer(&mut ship.bulkhead, bp, grade, c.special.as_deref())?;
        }
        ship.recalc();
        return Ok(());
    }
    let slot = r.slot(ship).expect("parsed slot refs exist");
    let def: &'static ModuleDef = if c.module_id.is_some() || c.group.is_some() {
        let index = match r {
            SlotRef::Standard(i) | SlotRef::Hardpoint(i) | SlotRef::Internal(i) => i,
            SlotRef::Bulkheads => 0,
        };
        candidates(ship, slot, index, &Filters::default(), None)
            .into_iter()
            .find(|m| match &c.module_id {
                Some(id) => &m.id == id,
                None => {
                    Some(&m.grp) == c.group.as_ref()
                        && c.class.is_none_or(|k| m.class == k)
                        && c.rating.as_ref().is_none_or(|k| &m.rating == k)
                        && c.mount.as_ref().is_none_or(|k| m.mount.as_ref() == Some(k))
                }
            })
            .ok_or_else(|| ServiceError::NoModuleFits {
                change: change_json(c),
                slot: c.slot.clone(),
            })?
    } else {
        // Re-engineer the current module: a fresh copy of it.
        slot.module
            .as_ref()
            .map(|m| m.def)
            .ok_or_else(|| ServiceError::EmptySlot(c.slot.clone()))?
    };
    let slot = r.slot_mut(ship).expect("parsed slot refs exist");
    Ship::place(slot, Some(def));
    if let (Some(bp), Some(m)) = (c.blueprint.as_deref(), slot.module.as_mut()) {
        let grade = c.grade.unwrap_or_else(|| max_blueprint_grade(&def.grp, bp));
        engineer(m, bp, grade, c.special.as_deref())?;
    }
    ship.recalc();
    Ok(())
}

fn change_json(c: &BuildChange) -> String {
    serde_json::to_string(c).unwrap_or_default()
}
