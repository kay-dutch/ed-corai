//! Every recorded Node tool call (`scripts/oracle-tools.mjs`) gives the same JSON or the same
//! error message from the Rust service. Links (and the SLEF's embedded link) compare by the build
//! they describe, since compressed bytes differ between zlib builds.

use corai_engine::{codec, interchange};
use corai_service::dispatch::call;
use serde_json::Value;

fn same_build(a: &str, b: &str) -> bool {
    let decode = |url: &str| interchange::parse_build(url, None).ok().map(|i| i.ship);
    match (decode(a), decode(b)) {
        (Some(x), Some(y)) => {
            let head = |s: &corai_engine::Ship| {
                codec::encode(s)
                    .split('.')
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(".")
            };
            x.id == y.id
                && head(&x) == head(&y)
                && codec::encode_modifications(&x) == codec::encode_modifications(&y)
        }
        _ => false,
    }
}

fn is_link(k: &str) -> bool {
    matches!(k, "coriolisUrl" | "edsyUrl" | "appURL")
}

fn same_json(path: &str, want: &Value, got: &Value, out: &mut Vec<String>) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            let (wk, gk): (Vec<_>, Vec<_>) = (w.keys().collect(), g.keys().collect());
            if wk != gk {
                out.push(format!("{path}: keys {wk:?} vs {gk:?}"));
                return;
            }
            for (k, v) in w {
                let p = format!("{path}.{k}");
                if is_link(k) {
                    let (a, b) = (
                        v.as_str().unwrap_or_default(),
                        g[k].as_str().unwrap_or_default(),
                    );
                    if !same_build(a, b) || a.contains("bn=") != b.contains("bn=") {
                        out.push(format!("{p}: links describe different builds"));
                    }
                } else if k == "json" {
                    // export_build: the document as a string.
                    let parse = |x: &Value| {
                        serde_json::from_str::<Value>(x.as_str().unwrap_or_default())
                            .unwrap_or_default()
                    };
                    same_json(&p, &parse(v), &parse(&g[k]), out);
                } else {
                    same_json(&p, v, &g[k], out);
                }
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
        (a, b) => out.push(format!(
            "{path}: {} vs {}",
            a.to_string().chars().take(120).collect::<String>(),
            b.to_string().chars().take(120).collect::<String>()
        )),
    }
}

#[test]
fn tools_match_node() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tools.json");
    let fx: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("parses");
    let mut failures = Vec::new();
    for (i, c) in fx["cases"].as_array().expect("cases").iter().enumerate() {
        let tool = c["tool"].as_str().expect("tool");
        let label = format!(
            "case {i} {tool} {}",
            c["args"].to_string().chars().take(60).collect::<String>()
        );
        match (
            call(tool, &c["args"]),
            c.get("error").and_then(Value::as_str),
        ) {
            (Err(e), Some(msg)) if e.to_string() == msg => {}
            (Err(e), Some(msg)) => {
                failures.push(format!("{label}: error\n    want {msg}\n    got  {e}"))
            }
            (Err(e), None) => failures.push(format!("{label}: unexpected error {e}")),
            (Ok(_), Some(msg)) => failures.push(format!("{label}: expected error {msg}")),
            (Ok(got), None) => {
                let mut d = Vec::new();
                same_json("", &c["result"], &got, &mut d);
                failures.extend(d.into_iter().take(6).map(|m| format!("{label}: {m}")));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
