//! Build summaries serialise to the same JSON as the Node server (`summarize` in describe.ts).
//! Links compare by the build they describe: compressed bytes differ between zlib builds.

use corai_engine::{codec, interchange};
use corai_opt::metrics::MetricContext;
use corai_service::{SummaryOptions, summarize};
use serde_json::Value;

fn same_json(path: &str, want: &Value, got: &Value, out: &mut Vec<String>) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            let (wk, gk): (Vec<_>, Vec<_>) = (w.keys().collect(), g.keys().collect());
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
                out.push(format!("{path}: {} items vs {}", w.len(), g.len()));
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

fn same_build(ship_id: &str, a: &str, b: &str) -> bool {
    let decode = |url: &str| {
        interchange::parse_build(url, Some(ship_id))
            .ok()
            .map(|i| i.ship)
    };
    match (decode(a), decode(b)) {
        (Some(x), Some(y)) => {
            let head = |s: &corai_engine::Ship| {
                codec::encode(s)
                    .split('.')
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(".")
            };
            head(&x) == head(&y)
                && codec::encode_modifications(&x) == codec::encode_modifications(&y)
        }
        _ => false,
    }
}

#[test]
fn summaries_match_node() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../corai-opt/tests/fixtures/summary.json"
    );
    let fx: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses");
    let ctx = MetricContext::default();
    let mut failures = Vec::new();
    for (i, c) in fx["cases"].as_array().expect("cases").iter().enumerate() {
        let ship_id = c["ship"].as_str().expect("ship");
        let ship = codec::decode(ship_id, c["code"].as_str().expect("code")).expect("decodes");
        let got = summarize(
            &ship,
            &ctx,
            &SummaryOptions {
                include_empty: false,
                build_name: Some("Oracle"),
            },
            &[],
        );
        let mut got = serde_json::to_value(&got).expect("serialises");
        let mut want = c["summary"].clone();
        for link in ["coriolisUrl", "edsyUrl"] {
            let (w, g) = (
                want[link].as_str().unwrap_or_default().to_string(),
                got[link].as_str().unwrap_or_default().to_string(),
            );
            if !same_build(ship_id, &w, &g) || w.contains("bn=Oracle") != g.contains("bn=Oracle") {
                failures.push(format!(
                    "case {i} {ship_id}: {link} describes another build"
                ));
            }
            want[link] = Value::Null;
            got[link] = Value::Null;
        }
        let mut d = Vec::new();
        same_json("summary", &want, &got, &mut d);
        failures.extend(d.into_iter().map(|m| format!("case {i} {ship_id}: {m}")));
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
