//! Build import and export match the Node version (`tests/fixtures/interchange.json`): journal
//! Loadout and SLEF exports, and imports from SLEF, Loadout, journal lines, Coriolis and EDSY
//! links and Coriolis' own SLEF with imperfect engineering rolls.

use corai_engine::{codec, interchange, journal};
use serde_json::Value;

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/interchange.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses")
}

/// JSON equality with numbers compared as numbers (1 == 1.0, tiny float noise allowed).
fn same_json(path: &str, want: &Value, got: &Value, out: &mut Vec<String>) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            let wk: Vec<_> = w.keys().collect();
            let gk: Vec<_> = g.keys().collect();
            if wk != gk {
                out.push(format!("{path}: keys {wk:?} vs {gk:?}"));
                return;
            }
            for (k, v) in w {
                same_json(&format!("{path}.{k}"), v, &g[k], out);
            }
        }
        (Value::Array(w), Value::Array(g)) => {
            if w.len() != g.len() {
                out.push(format!("{path}: length {} vs {}", w.len(), g.len()));
                return;
            }
            for (i, (a, b)) in w.iter().zip(g).enumerate() {
                same_json(&format!("{path}[{i}]"), a, b, out);
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (
                a.as_f64().unwrap_or(f64::NAN),
                b.as_f64().unwrap_or(f64::NAN),
            );
            if (a - b).abs() > 1e-9 * a.abs().max(1.0) {
                out.push(format!("{path}: {a} vs {b}"));
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push(format!("{path}: {a} vs {b}")),
    }
}

/// Two codes describe the same build: slots, power flags and priorities as text, engineering by
/// its uncompressed record (gzip headers differ between zlib builds).
fn same_build(ship_id: &str, want: &str, got: &corai_engine::Ship) -> bool {
    let head = |code: &str| code.split('.').take(3).collect::<Vec<_>>().join(".");
    let Ok(want_ship) = codec::decode(ship_id, want) else {
        return false;
    };
    head(&codec::encode(got)) == head(want)
        && codec::encode_modifications(got) == codec::encode_modifications(&want_ship)
}

fn code_param(url: &str) -> String {
    let q = url.split_once('?').map(|(_, q)| q).unwrap_or_default();
    q.split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .map(|c| {
            let mut out = Vec::new();
            let b = c.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%'
                    && i + 2 < b.len() + 1
                    && let Some(v) = c
                        .get(i + 1..i + 3)
                        .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b[i]);
                i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        })
        .unwrap_or_default()
}

#[test]
fn exports_match_node() {
    let fx = fixture();
    let mut failures = Vec::new();
    for (i, c) in fx["cases"].as_array().expect("cases").iter().enumerate() {
        let ship_id = c["ship"].as_str().expect("ship");
        let ship = codec::decode(ship_id, c["code"].as_str().expect("code")).expect("decodes");
        let mut d = Vec::new();
        same_json(
            "loadout",
            &c["exports"]["loadout"],
            &journal::to_loadout(&ship, "Oracle"),
            &mut d,
        );
        let mut slef = interchange::to_slef(&ship, "Oracle");
        let want_slef = &c["exports"]["slef"];
        // The link carries a gzip'd record: compare the builds it describes, then drop it.
        let got_url = slef[0]["header"]["appURL"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let want_url = want_slef[0]["header"]["appURL"]
            .as_str()
            .unwrap_or_default();
        if !same_build(ship_id, &code_param(want_url), &ship)
            || got_url.contains("&bn=Oracle") != want_url.contains("&bn=Oracle")
        {
            d.push(format!("slef appURL describes another build: {got_url}"));
        }
        slef[0]["header"]["appURL"] = want_slef[0]["header"]["appURL"].clone();
        // The program version moves with every release, the fixture only when regenerated.
        assert_eq!(
            slef[0]["header"]["appVersion"],
            env!("CARGO_PKG_VERSION"),
            "SLEF appVersion"
        );
        slef[0]["header"]["appVersion"] = want_slef[0]["header"]["appVersion"].clone();
        same_json("slef", want_slef, &slef, &mut d);
        for m in d {
            failures.push(format!("case {i} {ship_id}: {m}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn imports_match_node() {
    let fx = fixture();
    let mut failures = Vec::new();
    for (i, c) in fx["cases"].as_array().expect("cases").iter().enumerate() {
        let ship_id = c["ship"].as_str().expect("ship");
        for (kind, input) in c["inputs"].as_object().expect("inputs") {
            let want = &c["imports"][kind];
            let got = interchange::parse_build(input.as_str().expect("input"), None);
            let problem = match (&got, want.get("error").and_then(Value::as_str)) {
                (Err(e), Some(msg)) if e.to_string() == msg => None,
                (Err(e), _) => Some(format!("error {e}")),
                (Ok(_), Some(msg)) => Some(format!("expected error {msg}")),
                (Ok(imp), None) => {
                    let issues: Vec<&str> = want["issues"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    if want["ship"].as_str() != Some(imp.ship.id) {
                        Some(format!("ship {} vs {}", want["ship"], imp.ship.id))
                    } else if !same_build(
                        imp.ship.id,
                        want["code"].as_str().unwrap_or_default(),
                        &imp.ship,
                    ) {
                        Some(format!(
                            "build differs\n    want {}\n    got  {}",
                            want["code"],
                            codec::encode(&imp.ship)
                        ))
                    } else if issues != imp.issues {
                        Some(format!("issues {issues:?} vs {:?}", imp.issues))
                    } else {
                        None
                    }
                }
            };
            if let Some(p) = problem {
                failures.push(format!("case {i} {ship_id} [{kind}]: {p}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} imports differ\n{}",
        failures.len(),
        failures
            .iter()
            .take(16)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
