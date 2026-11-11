//! Coriolis build codes and share URLs (port of `Ship.toString` / `buildFrom`).
//!
//! A code is `A<bulkhead><module ids>.<enabled>.<priorities>.<modifications>`:
//! module ids are two characters (`-` for an empty slot) in standard, hardpoint, internal
//! order; `enabled` and `priorities` are LZString-compressed digit strings; `modifications`
//! is a gzipped binary record of engineering per slot (see [`encode_modifications`]).

use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use corai_data::DATA;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use indexmap::IndexMap;

use crate::jsmath::or;
use crate::module::{AppliedBlueprint, AppliedSpecial, Module, find_blueprint, quality_values};
use crate::ship::{Ship, ShipError, Slot, hardpoint_module, internal_module, standard_module};

const SLOT_DONE: i8 = -1;
const MOD_DONE: i8 = -1;
const MOD_BLUEPRINT: i8 = -2;
const MOD_GRADE: i8 = -3;
const MOD_SPECIAL: i8 = -4;
/// The object-typed `damagedist` modification; its value is a 4-byte string, never a number.
const MOD_OBJECT_ID: i8 = 40;
/// Pre-engineered modules Coriolis rebuilds at grade 1 rather than 5.
const GRADE1_EACH: [&str; 3] = [
    "Hpt_HeatSinkLauncher_Turret_Tiny",
    "Hpt_Mining_AbrBlstr_Fixed_Small",
    "Hpt_MultiCannon_Fixed_Medium",
];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum CodecError {
    #[error("not a Coriolis outfit URL: {0}")]
    NotAnOutfitUrl(String),
    #[error("legacy (version 1) build codes are not supported")]
    LegacyVersion,
    #[error("build code is truncated")]
    Truncated,
    #[error(transparent)]
    Ship(#[from] ShipError),
}

// ---------------------------------------------------------------- encoding

/// The Coriolis build code for a ship.
pub fn encode(ship: &Ship) -> String {
    let mut code = format!("A{}", ship.bulkhead_index);
    for slot in ship.all_slots() {
        match &slot.module {
            Some(m) => code.push_str(&m.def.id),
            None => code.push('-'),
        }
    }
    let flags = |f: &dyn Fn(&Slot) -> String, hatch: String| {
        lz_base64(&(hatch + &ship.all_slots().map(f).collect::<String>()))
    };
    let enabled = flags(
        &|s| if s.enabled { "1" } else { "0" }.to_string(),
        if ship.hatch_enabled { "1" } else { "0" }.to_string(),
    );
    let priorities = flags(&|s| s.priority.to_string(), ship.hatch_priority.to_string());
    let mods = encode_modifications(ship).map(|bytes| {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&bytes).expect("writing to a Vec cannot fail");
        BASE64.encode(gz.finish().expect("writing to a Vec cannot fail"))
    });
    format!("{code}.{enabled}.{priorities}.{}", mods.unwrap_or_default())
}

/// lz-string's `compressToBase64`, with JS lz-string 1.5's standard `=` padding.
fn lz_base64(s: &str) -> String {
    let mut out = lz_str::compress_to_base64(s)
        .trim_end_matches('=')
        .to_string();
    while out.len() % 4 != 0 {
        out.push('=');
    }
    out
}

/// `https://coriolis.io/outfit/<ship>?code=<code>`.
pub fn outfit_url(ship: &Ship) -> String {
    format!(
        "https://coriolis.io/outfit/{}?code={}",
        ship.id,
        percent_encode(&encode(ship))
    )
}

/// The uncompressed modification record, or `None` when no module is engineered.
///
/// Per engineered slot (index 0 = bulkheads, then standard, hardpoints, internal):
/// `i8 slot`, optional `-2 i32 blueprint id`, `-3 i32 grade`, `-4 i32 special id`, then
/// `i8 modification id, i32 value` pairs, terminated by `-1`; the record ends with `-1`.
pub fn encode_modifications(ship: &Ship) -> Option<Vec<u8>> {
    let data = &*DATA;
    let modules =
        std::iter::once(Some(&ship.bulkhead)).chain(ship.all_slots().map(|s| s.module.as_ref()));
    let mut buf = Vec::new();
    let mut slots_written = 0;
    for (i, m) in modules.enumerate() {
        let Some(m) = m else { continue };
        let allowed = data
            .modifications
            .modules
            .get(m.grp())
            .map(|g| &g.modifications);
        let mods: Vec<(i8, f64)> = m
            .mods
            .keys()
            .filter(|k| allowed.is_some_and(|a| a.contains(k)))
            .filter_map(|k| {
                let def = data.modifications.modifications.get(k)?;
                Some((def.id as i8, m.mod_value(k, true).unwrap_or(0.0)))
            })
            .collect();
        let blueprint = m.blueprint.as_ref().filter(|b| b.def.id != 0);
        if mods.is_empty() && blueprint.is_none() {
            continue;
        }
        slots_written += 1;
        buf.push(i as u8);
        if let Some(b) = blueprint {
            buf.push(MOD_BLUEPRINT as u8);
            buf.extend_from_slice(&b.def.id.to_le_bytes());
            buf.push(MOD_GRADE as u8);
            buf.extend_from_slice(&i32::from(b.grade).to_le_bytes());
        }
        if let Some(s) = m.special {
            buf.push(MOD_SPECIAL as u8);
            buf.extend_from_slice(&s.def.id.to_le_bytes());
        }
        for (id, value) in mods {
            buf.push(id as u8);
            // Node's writeInt32LE truncates towards zero; values are already rounded.
            buf.extend_from_slice(&(value as i32).to_le_bytes());
        }
        buf.push(MOD_DONE as u8);
    }
    if buf.is_empty() {
        return None;
    }
    buf.push(SLOT_DONE as u8);
    // Coriolis sizes its buffer one byte too large per slot and compresses the zero tail too.
    buf.resize(buf.len() + slots_written, 0);
    Some(buf)
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------- decoding

/// Engineering recovered from a code for one slot.
#[derive(Debug, Clone, Default)]
struct SavedSlot {
    mods: IndexMap<String, f64>,
    blueprint: Option<(i32, u8)>,
    special: Option<i32>,
}

/// Builds a ship from a full Coriolis outfit URL.
///
/// # Errors
/// Not an outfit URL, unknown ship, or malformed code.
pub fn from_url(url: &str) -> Result<Ship, CodecError> {
    let path = url
        .split_once("/outfit/")
        .map(|(_, p)| p)
        .ok_or_else(|| CodecError::NotAnOutfitUrl(url.into()))?;
    let (ship_id, query) = path.split_once('?').unwrap_or((path, ""));
    let ship_id = ship_id.trim_end_matches('/');
    let code = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .map(percent_decode)
        .unwrap_or_default();
    if code.is_empty() {
        return Ok(Ship::stock(ship_id)?);
    }
    decode(ship_id, &code)
}

fn from_url_safe(s: &str) -> String {
    s.replace('-', "/").replace('_', "+")
}

fn digits(part: Option<&str>) -> Option<Vec<Option<u32>>> {
    let part = part.filter(|p| !p.is_empty())?;
    let chars = lz_str::decompress_from_base64(&from_url_safe(part))?;
    Some(
        String::from_utf16_lossy(&chars)
            .chars()
            .map(|c| c.to_digit(10))
            .collect(),
    )
}

/// Builds a ship from a Coriolis build code (`buildFrom` + `buildWith`).
///
/// # Errors
/// Unknown ship, legacy code version, or truncated code.
pub fn decode(ship_id: &str, code: &str) -> Result<Ship, CodecError> {
    let mut ship = Ship::empty(ship_id)?;
    let mut parts = code.split('.');
    let ids = parts.next().unwrap_or_default();
    if ids.starts_with(|c: char| ('0'..='4').contains(&c)) {
        return Err(CodecError::LegacyVersion);
    }
    let ids = ids.get(1..).ok_or(CodecError::Truncated)?;
    let enabled = digits(parts.next());
    let priorities = digits(parts.next());
    let saved = parts
        .next()
        .filter(|p| !p.is_empty())
        .map(decode_modifications)
        .unwrap_or_default();

    let n_std = ship.standard.len();
    let n_hp = ship.hardpoints.len();
    let mut pos = 1;
    let mut read = || -> Result<Option<String>, CodecError> {
        if ids.get(pos..pos + 1) == Some("-") {
            pos += 1;
            return Ok(None);
        }
        let id = ids
            .get(pos..pos + 2)
            .ok_or(CodecError::Truncated)?
            .to_string();
        pos += 2;
        Ok(Some(id))
    };
    let std_ids: Vec<_> = (0..n_std).map(|_| read()).collect::<Result<_, _>>()?;
    let hp_ids: Vec<_> = (0..n_hp).map(|_| read()).collect::<Result<_, _>>()?;
    let int_ids: Vec<_> = (0..ship.internal.len())
        .map(|_| read())
        .collect::<Result<_, _>>()?;
    let bulkhead: usize = ids.get(0..1).and_then(|c| c.parse().ok()).unwrap_or(0);

    // `enabled[i] * 1` / `priorities[i] * 1`; a missing entry is NaN (disabled / band 0).
    let flag = |i: usize| {
        enabled
            .as_ref()
            .is_none_or(|e| e.get(i).copied().flatten().is_some_and(|d| d != 0))
    };
    let band = |i: usize| {
        priorities
            .as_ref()
            .and_then(|p| p.get(i).copied().flatten())
            .map_or(0, |d| (d as usize).min(4))
    };
    let slot_saved = |i: usize| saved.get(i).cloned().flatten();

    ship.use_bulkhead(bulkhead)?;
    if let Some(s) = slot_saved(0) {
        restore(&mut ship.bulkhead, s, Category::Other);
        ship.bulkhead.invalidate();
    }
    ship.hatch_enabled = flag(0);
    ship.hatch_priority = band(0);

    for (i, id) in std_ids.iter().enumerate() {
        let k = 1 + i;
        let slot = &mut ship.standard[i];
        slot.module = id
            .as_deref()
            .and_then(|id| standard_module(i, id))
            .map(Module::new);
        if let Some(m) = slot.module.as_mut() {
            restore(m, slot_saved(k).unwrap_or_default(), Category::Other);
            m.invalidate();
        }
        slot.priority = band(k);
        slot.enabled = flag(k);
    }
    let base = 1 + n_std;
    for (i, id) in hp_ids.iter().enumerate() {
        let k = base + i;
        let slot = &mut ship.hardpoints[i];
        slot.module = id.as_deref().and_then(hardpoint_module).map(Module::new);
        if let Some(m) = slot.module.as_mut() {
            restore(m, slot_saved(k).unwrap_or_default(), Category::Hardpoint);
            m.invalidate();
        }
        slot.priority = band(k);
        slot.enabled = flag(k);
    }
    let base = base + n_hp;
    for (i, id) in int_ids.iter().enumerate() {
        let k = base + i;
        let slot = &mut ship.internal[i];
        slot.module = id.as_deref().and_then(internal_module).map(Module::new);
        if let Some(m) = slot.module.as_mut() {
            restore(m, slot_saved(k).unwrap_or_default(), Category::Other);
            m.invalidate();
        }
        slot.priority = band(k);
        slot.enabled = flag(k);
    }
    ship.recalc();
    Ok(ship)
}

/// Which `buildWith` branch handles a pre-engineered module without saved engineering.
#[derive(Clone, Copy, PartialEq)]
enum Category {
    Hardpoint,
    Other,
}

/// Re-applies saved engineering to a freshly created module, following `buildWith`.
fn restore(m: &mut Module, saved: SavedSlot, category: Category) {
    let data = &*DATA;
    m.mods = saved.mods;
    let blueprint = saved.blueprint.and_then(|(id, grade)| {
        let fdname = &data
            .modifications
            .blueprints
            .values()
            .find(|b| b.id == id)?
            .fdname;
        let (key, def) = find_blueprint(fdname, m.def)?;
        Some(AppliedBlueprint { key, def, grade })
    });
    let special = saved.special.and_then(|id| {
        data.modifications
            .specials
            .iter()
            .find(|(_, s)| s.id == id)
            .map(|(k, def)| AppliedSpecial {
                key: k.as_str(),
                def,
            })
    });
    let Some(pe) = m.def.pre_engineered.as_ref() else {
        m.blueprint = blueprint;
        m.special = special.filter(|_| m.blueprint.is_some());
        return;
    };
    let symbol = m.def.symbol.as_deref().unwrap_or_default();
    let cumulative = |m: &mut Module, values: Vec<(&'static str, f64)>, force: bool| {
        for (feature, value) in values {
            let current = or(m.mod_value(feature, true), 0.0);
            m.set_modification(feature, current + value, force);
        }
    };
    if let Some(b) = blueprint {
        // Saved pre-engineered module: rebuild its fixed engineering from scratch.
        m.blueprint = Some(b);
        m.special = special;
        m.mods.clear();
        let grade = if GRADE1_EACH.contains(&symbol) { 1 } else { 5 };
        for name in &pe.blueprints {
            if let Some((_, def)) = find_blueprint(name.trim(), m.def) {
                cumulative(m, quality_values(def, grade, 1.0), true);
            }
        }
        return;
    }
    // No saved engineering (old codes): Coriolis' default pre-engineering, which differs by category.
    m.blueprint = pe
        .blueprints
        .first()
        .and_then(|n| find_blueprint(n.trim(), m.def))
        .map(|(key, def)| AppliedBlueprint {
            key,
            def,
            grade: pe.grade,
        });
    m.special = pe
        .experimental_effects
        .first()
        .and_then(|e| {
            data.modifications
                .specials
                .iter()
                .find(|(_, s)| &s.edname == e)
        })
        .map(|(k, def)| AppliedSpecial {
            key: k.as_str(),
            def,
        });
    for name in &pe.blueprints {
        let Some((_, def)) = find_blueprint(name.trim(), m.def) else {
            continue;
        };
        match category {
            Category::Hardpoint => cumulative(m, quality_values(def, pe.grade, 1.0), true),
            // Upstream applies grade 5's first bound unscaled here; reproduced as-is.
            Category::Other => {
                let Some(g5) = def.grades.get("5") else {
                    continue;
                };
                let values = g5
                    .features
                    .iter()
                    .filter_map(|(f, feature)| match feature {
                        corai_data::Feature::Range([a, _]) => Some((f.as_str(), *a)),
                        corai_data::Feature::DamageDist(_) => None,
                    })
                    .collect();
                cumulative(m, values, false);
            }
        }
    }
}

/// Parses the base64 gzip modification record into per-slot engineering.
/// Like Coriolis, a malformed record keeps whatever slots decoded before the problem.
fn decode_modifications(part: &str) -> Vec<Option<SavedSlot>> {
    let mut out: Vec<Option<SavedSlot>> = Vec::new();
    let Ok(gz) = BASE64.decode(from_url_safe(part)) else {
        return out;
    };
    let mut bytes = Vec::new();
    if GzDecoder::new(gz.as_slice())
        .read_to_end(&mut bytes)
        .is_err()
    {
        return out;
    }
    let data = &*DATA;
    let mut cursor = Cursor {
        bytes: &bytes,
        pos: 0,
    };
    while let Some(slot) = cursor.i8() {
        if slot == SLOT_DONE {
            break;
        }
        let mut saved = SavedSlot::default();
        let mut blueprint_id = None;
        let mut grade = None;
        loop {
            let Some(id) = cursor.i8() else { return out };
            if id == MOD_DONE {
                break;
            }
            if id == MOD_OBJECT_ID {
                cursor.pos += 4;
                continue;
            }
            let Some(value) = cursor.i32() else {
                return out;
            };
            match id {
                MOD_BLUEPRINT if value != 0 => blueprint_id = Some(value),
                MOD_GRADE if value != 0 => grade = u8::try_from(value).ok(),
                MOD_SPECIAL => saved.special = Some(value),
                MOD_BLUEPRINT | MOD_GRADE => {}
                _ => {
                    // Unknown ids make Coriolis throw, abandoning the rest of the record.
                    let Some(def) = data
                        .modifications
                        .modifications
                        .values()
                        .find(|m| m.id == i32::from(id))
                    else {
                        return out;
                    };
                    saved.mods.insert(def.name.clone(), f64::from(value));
                }
            }
        }
        if let Some(id) = blueprint_id {
            saved.blueprint = Some((id, grade.unwrap_or(0)));
        }
        let idx = slot as usize;
        if out.len() <= idx {
            out.resize(idx + 1, None);
        }
        out[idx] = Some(saved);
    }
    out
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn i8(&mut self) -> Option<i8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b as i8)
    }

    fn i32(&mut self) -> Option<i32> {
        let b: [u8; 4] = self.bytes.get(self.pos..self.pos + 4)?.try_into().ok()?;
        self.pos += 4;
        Some(i32::from_le_bytes(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outfit_url_round_trips() {
        let mut ship = Ship::stock("krait_mkii").expect("ship");
        let fsd = ship.standard[2].module.as_mut().expect("fsd");
        fsd.apply_blueprint("FSD_LongRange", 5, 1.0)
            .expect("blueprint");
        fsd.set_special(Some("special_fsd_heavy")).expect("special");
        ship.recalc();
        let url = outfit_url(&ship);
        assert!(url.starts_with("https://coriolis.io/outfit/krait_mkii?code=A"));
        let back = from_url(&url).expect("parses");
        assert_eq!(back.stats, ship.stats);
        assert_eq!(encode(&back), encode(&ship));
    }

    #[test]
    fn bare_outfit_url_is_stock() {
        let ship = from_url("https://coriolis.io/outfit/python").expect("parses");
        assert_eq!(ship.stats, Ship::stock("python").expect("python").stats);
    }

    #[test]
    fn decode_errors() {
        assert!(matches!(
            decode("python", "04p3td5R"),
            Err(CodecError::LegacyVersion)
        ));
        assert!(matches!(
            decode("python", "A0p3t"),
            Err(CodecError::Truncated)
        ));
        assert!(matches!(
            decode("no_such_ship", "A0"),
            Err(CodecError::Ship(ShipError::UnknownShip(_)))
        ));
    }

    #[test]
    fn corrupt_modification_record_keeps_modules() {
        let ship = Ship::stock("python").expect("python");
        let code = encode(&ship);
        let ids = code.split('.').next().expect("ids");
        let damaged =
            decode("python", &format!("{ids}...not-base64!")).expect("modules still decode");
        assert_eq!(damaged.stats, ship.stats);
    }

    #[test]
    fn rejects_non_outfit_urls() {
        assert!(matches!(
            from_url("https://example.com/x"),
            Err(CodecError::NotAnOutfitUrl(_))
        ));
    }
}
