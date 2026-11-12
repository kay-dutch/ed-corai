//! The Rust server's tool definitions and instructions (`src/tools.json`) are identical to the
//! Node server's, as recorded by `scripts/oracle-tools.mjs`.

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
    assert_eq!(
        ours["instructions"], node["instructions"],
        "server instructions differ"
    );
    let names = |v: &Value| {
        v["tools"]
            .as_array()
            .map(|t| t.iter().map(|x| x["name"].clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    assert_eq!(names(&ours), names(&node), "tool lists differ");
    for (a, b) in ours["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(node["tools"].as_array().into_iter().flatten())
    {
        assert_eq!(a, b, "definition of {} differs", a["name"]);
    }
}
