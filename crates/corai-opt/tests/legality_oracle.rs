//! The legality verdict and messages for every build in `tests/fixtures/legality.json` match the
//! Node validator: stock, random and deliberately illegal builds.

use corai_engine::codec;
use corai_opt::validate::validate;
use serde_json::Value;

#[test]
fn legality_matches_node() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/legality.json");
    let fx: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses");
    let mut failures = Vec::new();
    let cases = fx["cases"].as_array().expect("cases");
    for (i, c) in cases.iter().enumerate() {
        let ship_id = c["ship"].as_str().expect("ship");
        let ship = codec::decode(ship_id, c["code"].as_str().expect("code")).expect("decodes");
        let want: Vec<&str> = c["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let got = validate(&ship);
        if got != want {
            failures.push(format!(
                "case {i} {ship_id} [{}]:\n  want {want:?}\n  got  {got:?}",
                c["label"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} differ\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
