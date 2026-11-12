//! Post-search arrangement (port of `src/arrange.ts`): power priorities and internal slot
//! placement. Neither changes any figure the search scores; both make the build look and behave
//! like one a player would set up.

use corai_engine::{Module, Ship};

use crate::catalog::{SlotRef, fits};

/// Modules that do nothing in a fight. With priority-aware power they sit in the lowest priority
/// group, which the game sheds first when hardpoints are deployed.
pub const NON_COMBAT_GROUPS: &[&str] = &[
    "fs", "dc", "sua", "pv", "rf", "cc", "pc", "mlc", "fx", "hb", "ss", "sc", "pce", "pci", "pcm",
    "pcq",
];

pub fn is_non_combat(m: &Module) -> bool {
    NON_COMBAT_GROUPS.contains(&m.grp())
}

/// Power the build needs in a fight: every enabled combat module, hardpoints deployed.
pub fn combat_deployed_draw(ship: &Ship) -> f64 {
    ship.standard
        .iter()
        .chain(&ship.internal)
        .chain(&ship.hardpoints)
        .filter(|s| s.enabled)
        .filter_map(|s| s.module.as_ref())
        .filter(|m| !is_non_combat(m))
        .map(|m| corai_engine::jsmath::num(m.r().power))
        .sum()
}

/// Combat modules in priority group 1, non-combat modules (and the cargo hatch) in group 5.
pub fn assign_priorities(ship: &mut Ship) {
    for slot in ship
        .standard
        .iter_mut()
        .chain(ship.internal.iter_mut())
        .chain(ship.hardpoints.iter_mut())
    {
        slot.priority = if slot.module.as_ref().is_some_and(is_non_combat) {
            4
        } else {
            0
        };
    }
    ship.hatch_priority = 4;
    ship.recalc();
}

/// Minimum-cost assignment of `n` rows to `m >= n` columns (Hungarian algorithm, the same
/// formulation as the Node version so ties resolve identically). Returns the column per row.
fn assign(cost: &[Vec<f64>]) -> Vec<Option<usize>> {
    let n = cost.len();
    let m = cost.first().map_or(0, Vec::len);
    const INF: f64 = 1e18;
    let (mut u, mut v) = (vec![0.0; n + 1], vec![0.0; m + 1]);
    let (mut p, mut way) = (vec![0usize; m + 1], vec![0usize; m + 1]);
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut minv = vec![INF; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let (mut delta, mut j1) = (INF, 0);
            for j in 1..=m {
                if used[j] {
                    continue;
                }
                let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                if cur < minv[j] {
                    minv[j] = cur;
                    way[j] = j0;
                }
                if minv[j] < delta {
                    delta = minv[j];
                    j1 = j;
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut row_to_col = vec![None; n];
    for j in 1..=m {
        if p[j] != 0 {
            row_to_col[p[j] - 1] = Some(j - 1);
        }
    }
    row_to_col
}

/// Moves internal modules into the smallest slots they fit (respecting restricted and military
/// slots), so a size-1 module does not sit in a size-5 slot while a size-1 slot is free. Locked
/// slots stay put. Returns true when anything moved.
pub fn tidy_internal_slots(ship: &mut Ship, is_locked: impl Fn(SlotRef) -> bool) -> bool {
    let refs: Vec<usize> = (0..ship.internal.len())
        .filter(|&i| !is_locked(SlotRef::Internal(i)))
        .collect();
    // Modules in slot order, with the slot each one sits in.
    let mods: Vec<usize> = refs
        .iter()
        .copied()
        .filter(|&i| ship.internal[i].module.is_some())
        .collect();
    if mods.len() < 2 {
        return false;
    }
    const BAD: f64 = 1e9;
    // Squared unused slot size, so one size-1 module in a size-7 slot costs more than spreading the
    // gap; a hair for moving at all keeps the current layout on ties.
    let cost: Vec<Vec<f64>> = mods
        .iter()
        .map(|&from| {
            let m = ship.internal[from]
                .module
                .as_ref()
                .expect("filtered on module");
            refs.iter()
                .map(|&to| {
                    let slot = &ship.internal[to];
                    if fits(ship, slot, m.def) {
                        let gap = f64::from(slot.max_class) - f64::from(m.def.class);
                        gap * gap + if to == from { 0.0 } else { 1e-3 }
                    } else {
                        BAD
                    }
                })
                .collect()
        })
        .collect();
    let plan = assign(&cost);
    let Some(plan) = plan
        .iter()
        .enumerate()
        .map(|(i, c)| c.filter(|&c| cost[i][c] < BAD))
        .collect::<Option<Vec<usize>>>()
    else {
        return false;
    };
    if plan.iter().zip(&mods).all(|(&c, &from)| refs[c] == from) {
        return false;
    }
    // Take every module out (Coriolis enables a slot it empties and keeps its priority), then
    // place each one with the power state of the slot it came from.
    let state: Vec<(Option<Module>, bool, usize)> = mods
        .iter()
        .map(|&i| {
            let slot = &mut ship.internal[i];
            let taken = (slot.module.take(), slot.enabled, slot.priority);
            slot.enabled = true;
            taken
        })
        .collect();
    for ((module, enabled, priority), &c) in state.into_iter().zip(&plan) {
        let slot = &mut ship.internal[refs[c]];
        slot.module = module;
        slot.enabled = enabled;
        slot.priority = priority;
    }
    ship.recalc();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_finds_the_cheapest_matching() {
        let cost = vec![
            vec![4.0, 1.0, 3.0],
            vec![2.0, 0.0, 5.0],
            vec![3.0, 2.0, 2.0],
        ];
        assert_eq!(assign(&cost), vec![Some(1), Some(0), Some(2)]);
    }

    #[test]
    fn non_combat_modules_go_to_the_last_priority_group() {
        let mut ship = Ship::stock("diamondback_explorer").expect("ship");
        assign_priorities(&mut ship);
        assert_eq!(ship.hatch_priority, 4);
        for slot in ship.internal.iter().filter(|s| s.module.is_some()) {
            let m = slot.module.as_ref().expect("module");
            assert_eq!(
                slot.priority,
                if is_non_combat(m) { 4 } else { 0 },
                "{}",
                m.grp()
            );
        }
    }
}
