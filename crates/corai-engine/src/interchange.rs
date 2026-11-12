//! Reading and writing builds in the formats players paste: Coriolis outfit and import links,
//! bare Coriolis codes, EDSY import links, SLEF, journal Loadout events and journal files.
//! Port of `src/interchange.ts`; error messages match it.

use std::io::{Read, Write};

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use serde_json::{Value, json};

use crate::codec::{self, CodecError};
use crate::journal::{self, JournalError};
use crate::ship::{Ship, ShipError};

/// Version written into SLEF headers.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errors reading a build. Messages match the Node version.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum InterchangeError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error(transparent)]
    Ship(#[from] ShipError),
    #[error("SLEF contains no build.")]
    EmptySlef,
    #[error(
        "Not a recognised build: expected a Coriolis or EDSY link, SLEF, or a journal Loadout."
    )]
    NotRecognised,
    #[error(
        "EDSY short links (#/L=) cannot be read. In EDSY use Export → SLEF and paste that text instead."
    )]
    EdsyShortLink,
    #[error("This EDSY link carries no build data.")]
    EdsyNoData,
    #[error("Could not decompress the build data in the link.")]
    Decompress,
    #[error(
        "Cannot tell the ship type: pass a full Coriolis outfit URL or a ship id with the code."
    )]
    UnknownShipType,
}

/// A build read from player input, with any engineered values the import could not reproduce.
#[derive(Debug, Clone)]
pub struct Imported {
    pub ship: Ship,
    pub issues: Vec<String>,
}

// ---------------------------------------------------------------- export

/// `https://coriolis.io/outfit/<ship>?code=<code>[&bn=<name>]`.
pub fn coriolis_url(ship: &Ship, build_name: Option<&str>) -> String {
    let mut url = codec::outfit_url(ship);
    if let Some(name) = build_name.filter(|n| !n.is_empty()) {
        url.push_str("&bn=");
        url.push_str(&encode_uri_component(name));
    }
    url
}

/// The build in EDCD's Ship Loadout Exchange Format.
pub fn to_slef(ship: &Ship, name: &str) -> Value {
    json!([{
        "header": { "appName": "ED Corai", "appVersion": APP_VERSION, "appURL": coriolis_url(ship, Some(name)) },
        "data": journal::to_loadout(ship, name),
    }])
}

/// Opens the build in EDSY through its import endpoint (zlib-compressed SLEF, base64url).
pub fn edsy_url(ship: &Ship, name: &str) -> String {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(to_slef(ship, name).to_string().as_bytes())
        .expect("writing to a Vec cannot fail");
    let bytes = z.finish().expect("writing to a Vec cannot fail");
    format!(
        "https://edsy.org/import?import={}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

// ---------------------------------------------------------------- import

fn encode_uri_component(s: &str) -> String {
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

/// `decodeURIComponent` (`+` stays a plus; malformed escapes are kept as written).
fn decode_uri_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// URL query parameter as `URLSearchParams.get` returns it (`+` is a space).
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (decode_uri_component(&k.replace('+', " ")) == key)
            .then(|| decode_uri_component(&v.replace('+', " ")))
    })
}

/// Node's `Buffer.from(s, 'base64')`: either alphabet, padding optional.
fn lenient_base64(s: &str) -> Option<Vec<u8>> {
    let engine = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let cleaned: String = s
        .replace('-', "+")
        .replace('_', "/")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    engine.decode(cleaned.trim_end_matches('=')).ok()
}

/// Decompresses link data in whatever container it came in (gzip or zlib).
fn inflate_any(b64: &str) -> Result<String, InterchangeError> {
    let bytes = lenient_base64(b64).ok_or(InterchangeError::Decompress)?;
    let mut out = String::new();
    if GzDecoder::new(bytes.as_slice())
        .read_to_string(&mut out)
        .is_ok()
    {
        return Ok(out);
    }
    out.clear();
    if ZlibDecoder::new(bytes.as_slice())
        .read_to_string(&mut out)
        .is_ok()
    {
        return Ok(out);
    }
    Err(InterchangeError::Decompress)
}

fn from_loadout(json: &Value) -> Result<Imported, InterchangeError> {
    let (ship, issues) = journal::ship_from_loadout(json)?;
    Ok(Imported { ship, issues })
}

/// A parsed JSON document: SLEF (first entry with a build), `{ data: Loadout }` or a Loadout.
fn from_json_value(json: &Value) -> Result<Imported, InterchangeError> {
    if let Some(list) = json.as_array() {
        let first = list
            .iter()
            .find(|x| x.get("data").and_then(|d| d.get("Modules")).is_some())
            .ok_or(InterchangeError::EmptySlef)?;
        return from_loadout(&first["data"]);
    }
    match json.get("data").filter(|d| d.get("Modules").is_some()) {
        Some(data) => from_loadout(data),
        None => from_loadout(json),
    }
}

/// JSON text, or a journal file (one event per line; the last Loadout counts).
fn from_json_text(text: &str) -> Result<Imported, InterchangeError> {
    let trimmed = text.trim();
    let first_error = match serde_json::from_str::<Value>(trimmed) {
        Ok(json) => match from_json_value(&json) {
            Ok(imported) => return Ok(imported),
            Err(e) => e,
        },
        Err(_) => InterchangeError::NotRecognised,
    };
    let loadouts: Vec<Value> = trimmed
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.get("event").and_then(Value::as_str) == Some("Loadout"))
        .collect();
    match loadouts.last() {
        Some(last) => from_loadout(last),
        None => Err(first_error),
    }
}

/// A Coriolis outfit URL or a bare code with a ship id hint.
fn from_coriolis(text: &str, ship_hint: Option<&str>) -> Result<Imported, InterchangeError> {
    let mut ship_id = ship_hint.map(str::to_string);
    let mut code = text.trim().to_string();
    if let Some(at) = code.to_lowercase().find("/outfit/") {
        let rest = code[at + "/outfit/".len()..].to_string();
        let (path, query) = rest.split_once('?').unwrap_or((&rest, ""));
        let id: String = path
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let tail = &path[id.len()..];
        if !id.is_empty() && (tail.is_empty() || tail == "/") {
            ship_id = Some(id);
            code = query_param(query, "code").unwrap_or_default();
        }
    }
    let ship_id = ship_id.ok_or(InterchangeError::UnknownShipType)?;
    let ship = if code.is_empty() {
        Ship::stock(&ship_id)?
    } else {
        codec::decode(&ship_id, &code)?
    };
    Ok(Imported {
        ship,
        issues: Vec::new(),
    })
}

/// Reads a build from anything a player is likely to paste: a Coriolis outfit or import link, a
/// bare Coriolis code (with `ship_hint`), an EDSY import link, SLEF JSON, a journal Loadout event
/// or journal lines.
///
/// # Errors
/// Unrecognised input, unknown ship or modules, or undecodable link data.
pub fn parse_build(input: &str, ship_hint: Option<&str>) -> Result<Imported, InterchangeError> {
    let text = input.trim();
    let lower = text.to_lowercase();
    // JSON first: a SLEF header itself contains a Coriolis link (appURL).
    if text.starts_with('[') || text.starts_with('{') {
        return from_json_text(text);
    }
    let is_http = lower.starts_with("http:") || lower.starts_with("https:");
    if lower.contains("coriolis.io/outfit/")
        || (!text.contains('{') && !is_http && ship_hint.is_some())
    {
        return from_coriolis(text, ship_hint);
    }
    if let Some(at) = lower.find("coriolis.io/import/") {
        let data: String = text[at + "coriolis.io/import/".len()..]
            .chars()
            .take_while(|c| !matches!(c, '?' | '#') && !c.is_whitespace())
            .collect();
        if !data.is_empty() {
            return from_json_text(&inflate_any(&decode_uri_component(&data))?);
        }
    }
    if lower.contains("edsy.org") || lower.contains("edshipyard") {
        if text.contains("#/L=") {
            return Err(InterchangeError::EdsyShortLink);
        }
        let data = text
            .find("#/I=")
            .map(|at| {
                text[at + 4..]
                    .chars()
                    .take_while(|c| *c != '#' && !c.is_whitespace())
                    .collect::<String>()
            })
            .filter(|d| !d.is_empty())
            .or_else(|| {
                let query = text
                    .split_once('?')
                    .map(|(_, q)| q.split('#').next().unwrap_or_default())?;
                query_param(query, "import")
            })
            .ok_or(InterchangeError::EdsyNoData)?;
        let decoded = decode_uri_component(&data);
        let json = if decoded.trim_start().starts_with(['[', '{']) {
            decoded
        } else {
            inflate_any(&decoded)?
        };
        return from_json_text(&json);
    }
    from_json_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_component_round_trips_and_keeps_plus() {
        assert_eq!(
            decode_uri_component(&encode_uri_component("a b+c/é")),
            "a b+c/é"
        );
        assert_eq!(decode_uri_component("a+b"), "a+b");
        assert_eq!(
            query_param("code=a+b%2Bc&bn=x", "code").as_deref(),
            Some("a b+c")
        );
    }

    #[test]
    fn edsy_short_links_are_refused_with_guidance() {
        assert_eq!(
            parse_build("https://edsy.org/#/L=abc", None).err(),
            Some(InterchangeError::EdsyShortLink)
        );
    }

    #[test]
    fn exported_builds_read_back() {
        let ship = Ship::stock("python").expect("python");
        let back = parse_build(&edsy_url(&ship, "x"), None).expect("edsy link reads");
        assert_eq!(back.ship.stats, ship.stats);
        let back = parse_build(&to_slef(&ship, "x").to_string(), None).expect("slef reads");
        assert_eq!(back.ship.stats, ship.stats);
    }
}
