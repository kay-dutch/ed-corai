//! The Rust server's tool definitions and instructions (`src/tools.json`) are identical to the
//! Node server's, as recorded by `scripts/oracle-tools.mjs`. One deliberate difference: the
//! `install_update` description, since the program updates itself instead of a git clone. The
//! journal tools exist only here: they come after Node's tools, and the instructions add a line.

use serde_json::Value;

#[test]
fn tool_definitions_match_node() {
    let ours: Value =
        serde_json::from_str(include_str!("../src/tools.json")).expect("tools.json parses");
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../corai-service/tests/fixtures/tool-definitions.json"
    );
    let node: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("fixture"))
        .expect("fixture parses");
    let (ours_i, node_i) = (
        ours["instructions"].as_str().unwrap_or_default(),
        node["instructions"].as_str().unwrap_or_default(),
    );
    assert!(
        ours_i.starts_with(node_i) && ours_i.len() > node_i.len(),
        "server instructions differ"
    );
    const RUST_ONLY: &[&str] = &["current_ship", "recent_ships"];
    let names = |v: &Value| {
        v["tools"]
            .as_array()
            .map(|t| t.iter().map(|x| x["name"].clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let mut expected = names(&node);
    expected.extend(RUST_ONLY.iter().map(|n| Value::from(*n)));
    assert_eq!(names(&ours), expected, "tool lists differ");
    for (a, b) in ours["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(node["tools"].as_array().into_iter().flatten())
    {
        let (mut a, mut b) = (a.clone(), b.clone());
        if a["name"] == "install_update" {
            assert!(
                a["description"]
                    .as_str()
                    .is_some_and(|d| d.contains("Only call after the user explicitly agreed"))
            );
            a["description"] = Value::Null;
            b["description"] = Value::Null;
        }
        assert_eq!(a, b, "definition of {} differs", a["name"]);
    }
}
