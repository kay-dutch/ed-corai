//! Power priorities and internal slot tidying produce the same build as the Node version, with and
//! without locked slots (compared through the Coriolis share code: positions, power, engineering).

use corai_engine::codec;
use corai_opt::arrange::{assign_priorities, tidy_internal_slots};
use corai_opt::catalog::SlotRef;
use serde_json::Value;

/// A named lock rule: which slots the tidy step must leave alone.
type Lock = (&'static str, fn(SlotRef) -> bool);

#[test]
fn arrangement_matches_node() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/arrange.json");
    let fx: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses");
    let mut failures = Vec::new();
    let cases = fx["cases"].as_array().expect("cases");
    for (i, c) in cases.iter().enumerate() {
        let ship_id = c["ship"].as_str().expect("ship");
        let locks: [Lock; 2] = [
            ("free", |_| false),
            (
                "everyThirdLocked",
                |r| matches!(r, SlotRef::Internal(i) if i % 3 == 0),
            ),
        ];
        for (name, locked) in locks {
            let mut ship =
                codec::decode(ship_id, c["code"].as_str().expect("code")).expect("decodes");
            assign_priorities(&mut ship);
            let moved = tidy_internal_slots(&mut ship, locked);
            let want = &c[name];
            let want_code = want["code"].as_str().expect("code");
            let got = codec::encode(&ship);
            // Slots, power flags and priorities compare as text; the gzip'd engineering record by
            // its content (gzip headers differ between zlib builds and operating systems).
            let head = |code: &str| code.split('.').take(3).collect::<Vec<_>>().join(".");
            let want_ship = codec::decode(ship_id, want_code).expect("node code decodes");
            let same = head(&got) == head(want_code)
                && codec::encode_modifications(&ship) == codec::encode_modifications(&want_ship);
            if Some(moved) != want["moved"].as_bool() || !same {
                failures.push(format!(
                    "case {i} {ship_id} {name}: moved {moved} vs {}\n  want {}\n  got  {got}",
                    want["moved"], want["code"]
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ\n{}",
        failures.len(),
        failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
