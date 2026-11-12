//! Independent legality check (port of `src/validate.ts`): every fitted module must appear in
//! Coriolis' own outfitting list for its slot, and every blueprint in Coriolis' engineering menu
//! for that module. Deliberately not derived from [`crate::catalog`], so a gap in the search's own
//! rules shows up here. Messages match the Node version word for word.

use corai_data::{DATA, ModuleDef};
use corai_engine::{ModuleSet, Ship, SlotKind};

use crate::catalog::{MAX_EXPERIMENTAL_WEAPONS, SlotRef, can_engineer};

/// Modules Coriolis allows once per ship (shield generators of every kind count together).
const UNIQUE: &[&[&str]] = &[
    &["sg", "bsg", "psg"],
    &["rf"],
    &["fs"],
    &["fh"],
    &["gfsb"],
    &["dc"],
    &["ews"],
    &["mlc"],
];

fn label(m: &ModuleDef) -> String {
    format!(
        "{}{} {}",
        m.class,
        m.rating,
        m.name.as_deref().unwrap_or(&m.grp)
    )
}

/// The game and Coriolis name a blueprint differently per module family (journal
/// `Weapon_Overcharged` is Coriolis `MC_Overcharged` on multi-cannons), so compare the part after
/// the family prefix, as Coriolis' lookup does.
fn blueprint_offered(grp: &str, name: &str) -> bool {
    let data = &*DATA;
    let suffix = |n: &str| {
        let lower = n.to_lowercase();
        match lower.split_once('_') {
            Some((_, rest)) if !rest.is_empty() => rest.to_string(),
            _ => lower,
        }
    };
    data.modifications.modules.get(grp).is_some_and(|g| {
        g.blueprints.keys().any(|k| {
            k == name
                || data
                    .modifications
                    .blueprints
                    .get(k)
                    .is_some_and(|b| b.fdname == name)
                || suffix(k) == suffix(name)
        })
    })
}

/// Problems that make the build impossible to fit in game; empty for a legal build.
pub fn validate(ship: &Ship) -> Vec<String> {
    let mut problems = Vec::new();
    let Ok(set) = ModuleSet::for_ship(ship.id) else {
        return vec![format!("unknown ship {}", ship.id)];
    };
    for r in SlotRef::all(ship) {
        let Some(slot) = r.slot(ship) else { continue }; // bulkheads
        let Some(m) = &slot.module else { continue };
        let key = r.key(ship);
        let def = m.def;
        let offered = match (r, slot.kind) {
            (SlotRef::Standard(i), _) => set.standard(i).to_vec(),
            (_, SlotKind::Internal) => set.internal(slot.max_class, slot.eligible),
            _ => set.hardpoints(slot.max_class, slot.eligible, slot.mvr),
        };
        if !offered.iter().any(|o| o.id == def.id && o.grp == def.grp) {
            problems.push(format!(
                "{key}: {} is not offered by Coriolis for this slot",
                label(def)
            ));
        }
        if let Some(bp) = m.blueprint()
            && def.pre_engineered.is_none()
        {
            if !can_engineer(def) {
                problems.push(format!(
                    "{key}: {} cannot be engineered but carries {}",
                    label(def),
                    bp.def.fdname
                ));
            } else if !blueprint_offered(&def.grp, &bp.def.fdname) {
                problems.push(format!(
                    "{key}: {} is not an engineering option for {}",
                    bp.def.fdname, def.grp
                ));
            }
        }
        if !def.ship.is_empty() && !def.ship.iter().any(|s| s == ship.id) {
            problems.push(format!(
                "{key}: {} is restricted to another ship",
                label(def)
            ));
        }
    }
    for family in UNIQUE {
        let keys: Vec<String> = SlotRef::all(ship)
            .into_iter()
            .filter(|r| matches!(r, SlotRef::Internal(_)))
            .filter(|r| {
                r.slot(ship)
                    .and_then(|s| s.module.as_ref())
                    .is_some_and(|m| family.contains(&m.grp()))
            })
            .map(|r| r.key(ship))
            .collect();
        if keys.len() > 1 {
            problems.push(format!(
                "{}: only one {} allowed",
                keys.join(", "),
                family.join("/")
            ));
        }
    }
    let experimental = ship
        .hardpoints
        .iter()
        .filter(|h| h.module.as_ref().is_some_and(|m| m.def.experimental))
        .count();
    if experimental > MAX_EXPERIMENTAL_WEAPONS {
        problems.push(format!(
            "{experimental} experimental weapons fitted (max {MAX_EXPERIMENTAL_WEAPONS})"
        ));
    }
    problems
}
