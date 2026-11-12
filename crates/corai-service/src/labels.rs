//! Human-readable module and engineering labels (`moduleLabel`, `engineeringLabel`).

use corai_engine::Module;

use crate::names::group_name;

/// `4A Plasma Accelerator (fixed)`, `2E Missile Rack (fixed) [seeker]`, ...
pub fn module_label(m: Option<&Module>) -> String {
    let Some(m) = m else { return "empty".into() };
    let d = m.def;
    let base = d
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .or_else(|| group_name(&d.grp).map(str::to_string))
        .unwrap_or_else(|| d.grp.clone());
    let class_rating = if d.rating.is_empty() {
        String::new()
    } else {
        format!("{}{} ", d.class, d.rating)
    };
    let mount = match d.mount.as_deref() {
        None | Some("") => String::new(),
        Some("F") => " (fixed)".into(),
        Some("G") => " (gimballed)".into(),
        Some("T") => " (turret)".into(),
        Some(other) => format!(" ({other})"),
    };
    let missile = match d.missile.as_deref() {
        None | Some("") => String::new(),
        Some("D") => " [dumbfire]".into(),
        Some("S") => " [seeker]".into(),
        Some(other) => format!(" [{other}]"),
    };
    format!("{class_rating}{base}{mount}{missile}")
        .trim()
        .to_string()
}

/// `Dirty Drives G5 + Drag Drives (pre-engineered)`, or `None` without engineering.
pub fn engineering_label(m: &Module) -> Option<String> {
    let b = m.blueprint()?;
    let name = b.def.name.clone().unwrap_or_else(|| b.def.fdname.clone());
    let special = m
        .special()
        .map(|s| format!(" + {}", s.def.name))
        .unwrap_or_default();
    let pre = if m.def.pre_engineered.is_some() {
        " (pre-engineered)"
    } else {
        ""
    };
    Some(format!("{name} G{}{special}{pre}", b.grade))
}
