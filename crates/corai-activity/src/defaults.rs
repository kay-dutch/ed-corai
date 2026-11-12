//! The player's saved defaults, set in the window and used by every MCP server for settings the
//! AI app leaves out of a call. Every field is optional: unset means the tool's own default.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Saved in `defaults.json` in the state folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Defaults {
    /// SYS pips while fighting (0 to 4 in halves).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sys_pips: Option<f64>,
    /// WEP pips while fighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wep_pips: Option<f64>,
    /// `even` or `pve`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_preset: Option<String>,
    /// `full` or `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engineering: Option<String>,
}

pub fn defaults_path() -> Option<PathBuf> {
    super::state_dir().map(|d| d.join("defaults.json"))
}

impl Defaults {
    /// The saved defaults; none when the file is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// # Errors
    /// The file cannot be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(tmp, path)
    }

    /// Fills in what the call leaves out: `context` settings when the tool takes a context,
    /// `engineering` when it takes that. Returns what was filled in, e.g. `["sysPips 4"]`.
    pub fn apply(
        &self,
        args: &mut Value,
        takes_context: bool,
        takes_engineering: bool,
    ) -> Vec<String> {
        let mut applied = Vec::new();
        let Some(obj) = args.as_object_mut() else {
            return applied;
        };
        if takes_context {
            let context = obj
                .entry("context")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(ctx) = context.as_object_mut() {
                // An explicit damage mix outranks the preset.
                let preset = self
                    .damage_preset
                    .clone()
                    .filter(|_| !ctx.contains_key("damage"));
                for (key, value) in [
                    ("sysPips", self.sys_pips.map(Value::from)),
                    ("wepPips", self.wep_pips.map(Value::from)),
                    ("damagePreset", preset.map(Value::from)),
                ] {
                    if let Some(v) = value
                        && !ctx.contains_key(key)
                    {
                        let shown = v.as_str().map(str::to_string).unwrap_or_else(|| {
                            // 4, not 4.0
                            v.as_f64().map_or_else(|| v.to_string(), |f| f.to_string())
                        });
                        applied.push(format!("{key} {shown}"));
                        ctx.insert(key.into(), v);
                    }
                }
            }
            if context.as_object().is_some_and(Map::is_empty) {
                obj.remove("context");
            }
        }
        if takes_engineering
            && let Some(e) = &self.engineering
            && !obj.contains_key("engineering")
        {
            obj.insert("engineering".into(), e.clone().into());
            applied.push(format!("engineering {e}"));
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn saved() -> Defaults {
        Defaults {
            sys_pips: Some(4.0),
            wep_pips: Some(2.0),
            damage_preset: Some("pve".into()),
            engineering: Some("none".into()),
        }
    }

    #[test]
    fn defaults_fill_only_what_the_call_leaves_out() {
        let mut args =
            json!({ "ship": "python", "context": { "wepPips": 4, "damage": { "kinetic": 1 } } });
        let applied = saved().apply(&mut args, true, true);
        assert_eq!(applied, vec!["sysPips 4", "engineering none"]);
        assert_eq!(
            args["context"],
            json!({ "wepPips": 4, "damage": { "kinetic": 1 }, "sysPips": 4.0 })
        );

        let mut plain = json!({ "ship": "python" });
        saved().apply(&mut plain, false, false);
        assert_eq!(
            plain,
            json!({ "ship": "python" }),
            "tools without these settings are untouched"
        );

        let mut none = json!({ "ship": "python" });
        assert!(Defaults::default().apply(&mut none, true, true).is_empty());
        assert_eq!(none, json!({ "ship": "python" }), "no empty context added");
    }

    #[test]
    fn defaults_round_trip_and_tolerate_a_bad_file() {
        let dir = std::env::temp_dir().join(format!("corai-defaults-{}", std::process::id()));
        let path = dir.join("defaults.json");
        assert_eq!(Defaults::load(&path), Defaults::default());
        saved().save(&path).expect("saved");
        assert_eq!(Defaults::load(&path), saved());
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(Defaults::load(&path), Defaults::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
