//! The search's module catalog matches the Node version: the modules offered in every slot of
//! every hull (default filters) and the engineering options of every module.

use std::collections::BTreeMap;

use corai_data::DATA;
use corai_engine::Ship;
use corai_opt::catalog::{Filters, SlotRef, candidates, engineering_options};
use serde_json::Value;

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalog.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

#[test]
fn slot_candidates_match_node() {
    let fx = fixture();
    let filters = Filters::default();
    let mut failures = Vec::new();
    for (id, slots) in fx["ships"].as_object().expect("ships") {
        let ship = Ship::stock(id).expect("ship");
        for (key, want) in slots.as_object().expect("slots") {
            let r = SlotRef::parse(&ship, key).unwrap_or_else(|| panic!("{id} {key}"));
            let got: Vec<String> = match r {
                SlotRef::Bulkheads => (0..DATA.ships[id.as_str()].bulkheads.len())
                    .map(|i| format!("bh/{i}"))
                    .collect(),
                _ => {
                    let slot = r.slot(&ship).expect("slot");
                    let index = match r {
                        SlotRef::Standard(i) | SlotRef::Hardpoint(i) | SlotRef::Internal(i) => i,
                        SlotRef::Bulkheads => 0,
                    };
                    candidates(&ship, slot, index, &filters, None)
                        .iter()
                        .map(|m| format!("{}/{}", m.grp, m.id))
                        .collect()
                }
            };
            let mut want = strings(want);
            let mut got_sorted = got.clone();
            want.sort();
            got_sorted.sort();
            if want != got_sorted {
                let missing: Vec<_> = want.iter().filter(|w| !got_sorted.contains(w)).collect();
                let extra: Vec<_> = got_sorted.iter().filter(|g| !want.contains(g)).collect();
                failures.push(format!("{id} {key}: missing {missing:?} extra {extra:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} slots differ\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn engineering_options_match_node() {
    let fx = fixture();
    let mut by_module: BTreeMap<&str, (&'static corai_data::ModuleDef,)> = BTreeMap::new();
    for groups in [
        &DATA.modules.standard,
        &DATA.modules.hardpoints,
        &DATA.modules.internal,
    ] {
        for m in groups.values().flatten() {
            by_module
                .entry(Box::leak(format!("{}/{}", m.grp, m.id).into_boxed_str()))
                .or_insert((m,));
        }
    }
    let mut failures = Vec::new();
    for (key, want) in fx["engineering"].as_object().expect("engineering") {
        let (m,) = by_module
            .get(key.as_str())
            .copied()
            .unwrap_or_else(|| panic!("module {key}"));
        let got: Vec<String> = engineering_options(m)
            .iter()
            .map(|e| format!("{}:{}:{}", e.blueprint, e.grade, e.special.unwrap_or("")))
            .collect();
        let want = strings(want);
        if got != want {
            failures.push(format!(
                "{key}: want {} got {} (first diff {:?} vs {:?})",
                want.len(),
                got.len(),
                want.iter().find(|w| !got.contains(w)),
                got.iter().find(|g| !want.contains(g))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} modules differ\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
