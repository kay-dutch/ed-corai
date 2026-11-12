//! One-line descriptions of tool calls for the window's activity list.

use serde_json::Value;

/// The first key figure (the objective) as "metric value".
fn objective(key: &Value) -> Option<String> {
    let (k, v) = key.as_object()?.iter().next()?;
    Some(format!("{k} {v}"))
}

/// What a call is about, from its arguments: the ship (or "a build") and the objective.
pub fn describe_call(args: &Value) -> String {
    let ship = args["ship"]
        .as_str()
        .map(str::to_string)
        .or_else(|| args["build"].is_string().then(|| "a build".to_string()));
    let metric = args["objective"]["metric"].as_str();
    match (ship, metric) {
        (Some(s), Some(m)) => format!("{s}, best {m}"),
        (Some(s), None) => s,
        (None, Some(m)) => format!("best {m}"),
        (None, None) => String::new(),
    }
}

/// What a call produced, and a Coriolis link to the resulting build when there is one.
pub fn describe_result(tool: &str, r: &Value) -> (String, Option<String>) {
    let link = |v: &Value| v["coriolisUrl"].as_str().map(str::to_string);
    match tool {
        "optimize_build" => {
            let ship = r["best"]["ship"].as_str().unwrap_or("build");
            let text = if r["feasible"] == false {
                format!("{ship}: no build meets every constraint")
            } else {
                objective(&r["key"]).map_or_else(|| ship.to_string(), |o| format!("{ship}: {o}"))
            };
            (text, link(&r["best"]))
        }
        "compare_ships" => {
            let top = &r["ranked"][0];
            match top["name"].as_str() {
                Some(name) => (
                    objective(&top["key"]).map_or_else(
                        || format!("{name} ranks first"),
                        |o| format!("{name} ranks first: {o}"),
                    ),
                    link(top),
                ),
                None => ("no ship qualifies".into(), None),
            }
        }
        "check_for_updates" => (r["message"].as_str().unwrap_or_default().into(), None),
        _ => {
            let summary = if r["build"].is_object() {
                &r["build"]
            } else {
                r
            };
            match summary["ship"].as_str() {
                Some(ship) => (ship.into(), link(summary)),
                None => (String::new(), None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calls_name_the_ship_and_the_goal() {
        assert_eq!(
            describe_call(&json!({ "ship": "python", "objective": { "metric": "jumpLaden" } })),
            "python, best jumpLaden"
        );
        assert_eq!(
            describe_call(&json!({ "build": "https://coriolis.io/x" })),
            "a build"
        );
        assert_eq!(describe_call(&json!({})), "");
    }

    #[test]
    fn results_name_the_build_and_link_it() {
        let opt = json!({ "feasible": true, "key": { "jumpLaden": 31.2, "cargo": 64 },
            "best": { "ship": "Python", "coriolisUrl": "https://coriolis.io/outfit/python?code=A" } });
        assert_eq!(
            describe_result("optimize_build", &opt),
            (
                "Python: jumpLaden 31.2".into(),
                Some("https://coriolis.io/outfit/python?code=A".into())
            )
        );
        let none = json!({ "feasible": false, "key": {}, "best": { "ship": "Python" } });
        assert_eq!(
            describe_result("optimize_build", &none).0,
            "Python: no build meets every constraint"
        );
        let cmp = json!({ "ranked": [{ "name": "Krait Mk II", "key": { "dps": 120 }, "coriolisUrl": "u" }] });
        assert_eq!(
            describe_result("compare_ships", &cmp),
            ("Krait Mk II ranks first: dps 120".into(), Some("u".into()))
        );
        let details =
            json!({ "properties": {}, "build": { "ship": "Cobra Mk III", "coriolisUrl": "c" } });
        assert_eq!(
            describe_result("ship_details", &details),
            ("Cobra Mk III".into(), Some("c".into()))
        );
        assert_eq!(
            describe_result("list_ships", &json!([])),
            (String::new(), None)
        );
    }
}
