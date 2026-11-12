//! The product library as JSON: an array of records in the web app's shape (camelCase keys), read
//! with the same defaulting rules as its `parseProduct`, plus the clamping its spreadsheet import
//! applies. Keys this crate does not model (notably `img`) are kept in [`Product::extra`].

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::Product;

/// Largest library file or string accepted; the bundled library is 13 MB.
pub const MAX_LIBRARY_BYTES: usize = 64 << 20;
/// Most records accepted; the bundled library has 428.
pub const MAX_PRODUCTS: usize = 10_000;
const MAX_PORTS: f64 = 1000.0;
const MAX_WATTS: f64 = 1e6;
const MAX_CHANNELS: f64 = 1e6;

/// (in, out) record keys per protocol, in [`crate::Protocol::ALL`] order.
const PORT_KEYS: [(&str, &str); 10] = [
    ("netIn", "netOut"),
    ("netDataIn", "netDataOut"),
    ("scnIn", "scnOut"),
    ("dmxIn", "dmxOut"),
    ("v10In", "v10Out"),
    ("ctIn", "ctOut"),
    ("pwrIn", "pwrOut"),
    ("midiIn", "midiOut"),
    ("ep3In", "ep3Out"),
    ("ep4In", "ep4Out"),
];
const OWN_KEYS: [&str; 12] = ["id", "name", "code", "cat", "brand", "color", "notes", "modes", "channels", "universes", "poeWatts", "poePortWatts"];

fn text(v: Option<&Value>, default: &str) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => default.into(),
    }
}

/// A number or numeric string, clamped to `0..=max`; anything else is 0.
fn num(v: Option<&Value>, max: f64) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    n.filter(|n| n.is_finite()).unwrap_or(0.0).clamp(0.0, max)
}

fn count(v: Option<&Value>, max: f64) -> u32 {
    num(v, max).floor() as u32
}

fn color(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() == 6 { u32::from_str_radix(h, 16).ok() } else { None }
}

fn parse_record(v: &Value) -> Option<Product> {
    let o = v.as_object()?;
    let id = text(o.get("id"), "").trim().to_string();
    let name = text(o.get("name"), "").trim().to_string();
    if id.is_empty() || name.is_empty() {
        return None;
    }
    let mut counts = [(0, 0); 10];
    for (c, (i, o_)) in counts.iter_mut().zip(PORT_KEYS) {
        *c = (count(o.get(i), MAX_PORTS), count(o.get(o_), MAX_PORTS));
    }
    let extra = o
        .iter()
        .filter(|(k, _)| !OWN_KEYS.contains(&k.as_str()) && !PORT_KEYS.iter().any(|(i, o)| k == i || k == o))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Some(Product {
        code: text(o.get("code"), &id).trim().to_string(),
        cat: text(o.get("cat"), "Uncategorized").trim().to_string(),
        brand: text(o.get("brand"), "").trim().to_string(),
        color: color(&text(o.get("color"), "")).unwrap_or(0x555555),
        notes: text(o.get("notes"), ""),
        modes: text(o.get("modes"), ""),
        channels: count(o.get("channels"), MAX_CHANNELS),
        universes: count(o.get("universes"), MAX_CHANNELS),
        poe_watts: num(o.get("poeWatts"), MAX_WATTS),
        poe_port_watts: num(o.get("poePortWatts"), MAX_WATTS),
        counts,
        extra,
        id,
        name,
        missing: false,
    })
}

/// Products from records. Malformed records (not an object, no id or no name) are skipped and a
/// repeated id keeps its first record. More than [`MAX_PRODUCTS`] records is an error.
pub fn parse_products(records: &[Value]) -> Result<Vec<Product>, String> {
    if records.len() > MAX_PRODUCTS {
        return Err(format!("library has {} records, the limit is {MAX_PRODUCTS}", records.len()));
    }
    let mut seen = HashSet::new();
    Ok(records.iter().filter_map(parse_record).filter(|p| seen.insert(p.id.clone())).collect())
}

pub fn parse_library(json: &str) -> Result<Vec<Product>, String> {
    if json.len() > MAX_LIBRARY_BYTES {
        return Err(format!("library is larger than {} MB", MAX_LIBRARY_BYTES >> 20));
    }
    match serde_json::from_str::<Value>(json).map_err(|e| format!("library is not valid JSON: {e}"))? {
        Value::Array(records) => parse_products(&records),
        _ => Err("library must be a JSON array of product records".into()),
    }
}

/// One product as a record, with its extra keys.
pub fn product_record(p: &Product) -> Map<String, Value> {
    let mut o = p.extra.clone();
    for (key, v) in
        [("id", &p.id), ("name", &p.name), ("code", &p.code), ("cat", &p.cat), ("brand", &p.brand), ("notes", &p.notes), ("modes", &p.modes)]
    {
        o.insert(key.into(), json!(v));
    }
    o.insert("color".into(), json!(format!("#{:06x}", p.color & 0xffffff)));
    o.insert("channels".into(), json!(p.channels));
    o.insert("universes".into(), json!(p.universes));
    o.insert("poeWatts".into(), json!(p.poe_watts));
    o.insert("poePortWatts".into(), json!(p.poe_port_watts));
    for ((i, o_), (n_in, n_out)) in PORT_KEYS.iter().zip(p.counts) {
        o.insert((*i).into(), json!(n_in));
        o.insert((*o_).into(), json!(n_out));
    }
    o
}

/// The library file text; products marked missing are placeholders and are left out.
pub fn library_json(products: &[Product]) -> String {
    Value::Array(products.iter().filter(|p| !p.missing).map(|p| Value::Object(product_record(p))).collect()).to_string()
}
