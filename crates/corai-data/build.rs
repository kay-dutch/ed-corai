//! Merges the upstream coriolis-data JSON files into one document embedded in the crate.
//!
//! The upstream package assembles its distribution through `index.js` files of the form
//! `key: require('./dir/file').key`. Following those files (rather than globbing the
//! directories) reproduces the exact object key order the Coriolis app sees, which matters
//! for its "first matching key" lookups.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

struct Entry {
    key: String,
    path: String,
    prop: String,
}

/// Extracts `key: require('<path>').<prop>` entries from an index.js file.
fn parse_index(file: &Path) -> Vec<Entry> {
    let text =
        fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    text.lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let start = rest.find("require(")? + "require(".len();
            let rest = &rest[start..];
            let quote = rest.chars().next()?;
            let end = rest[1..].find(quote)? + 1;
            let path = rest[1..end].to_string();
            let after = &rest[end + 1..];
            let prop = after.strip_prefix(").").map(|p| {
                p.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            });
            Some(Entry {
                key: key.trim().to_string(),
                path,
                prop: prop.unwrap_or_default(),
            })
        })
        .collect()
}

fn read_json(dir: &Path, rel: &str) -> Value {
    let mut path = dir.join(rel.trim_start_matches("./"));
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn take(value: Value, prop: &str) -> Value {
    if prop.is_empty() {
        return value;
    }
    match value {
        Value::Object(mut map) => map
            .remove(prop)
            .unwrap_or_else(|| panic!("missing property {prop}")),
        _ => panic!("expected object when taking {prop}"),
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../upstream/coriolis-data");
    for dir in ["ships", "modules", "modifications"] {
        println!("cargo::rerun-if-changed={}", root.join(dir).display());
    }

    let mut ships = Map::new();
    let ships_dir = root.join("ships");
    for e in parse_index(&ships_dir.join("index.js")) {
        ships.insert(e.key, take(read_json(&ships_dir, &e.path), &e.prop));
    }

    let modules_dir = root.join("modules");
    let mut modules = Map::new();
    for category in ["standard", "hardpoints", "internal"] {
        modules.insert(category.to_string(), Value::Object(Map::new()));
    }
    for e in parse_index(&modules_dir.join("index.js")) {
        let category = e
            .path
            .trim_start_matches("./")
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let Some(Value::Object(group)) = modules.get_mut(&category) else {
            panic!("module {} has unknown category {category}", e.key);
        };
        group.insert(e.key, take(read_json(&modules_dir, &e.path), &e.prop));
    }

    let mods_dir = root.join("modifications");
    let mut modifications = Map::new();
    for e in parse_index(&mods_dir.join("index.js")) {
        modifications.insert(e.key, take(read_json(&mods_dir, &e.path), &e.prop));
    }

    let mut doc = Map::new();
    doc.insert("ships".into(), Value::Object(ships));
    doc.insert("modules".into(), Value::Object(modules));
    doc.insert("modifications".into(), Value::Object(modifications));

    let out =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("coriolis.json");
    fs::write(
        &out,
        serde_json::to_string(&Value::Object(doc)).expect("serialisable"),
    )
    .unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}
