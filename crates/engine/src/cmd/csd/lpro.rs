//! `csd.import.lpro`: reads a Luminary Pro design for its fixture schedule (which luminaires, how
//! many) and adds the library products it matches to the unplaced list. The file is hostile input:
//! read through a size cap, walked leniently as JSON, every count bounded.

use cadcraft_csd::{Product, is_builtin};
use serde_json::{Map, Value, json};

use super::{bad, find_product, schedule, str_param};
use crate::{Result, Session};

const CMD: &str = "csd.import.lpro";
#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: usize = 50 * 1024 * 1024;
const MAX_TYPES: usize = 10_000;
const MAX_FIXTURES: usize = 1_000_000;

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> std::io::Result<Vec<u8>> {
    crate::capped_read::read_capped(std::path::Path::new(path), MAX_BYTES)
}

#[cfg(target_arch = "wasm32")]
fn read(_: &str) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "files are not available on the web"))
}

struct LuminaryType {
    key: String,
    name: String,
    manufacturer: String,
    luminaire: String,
    catalog: String,
    file_name: String,
    quantity: u64,
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn has_photometry(p: Option<&Value>) -> Option<&Value> {
    p.filter(|p| p.is_object() && ["vAngles", "hAngles", "candela"].iter().all(|k| !array(p, k).is_empty()))
}

fn strip_ies(s: &str) -> String {
    match s.len().checked_sub(4).and_then(|i| Some((i, s.get(i..)?))) {
        Some((i, t)) if t.eq_ignore_ascii_case(".ies") => s.get(..i).unwrap_or_default().to_string(),
        _ => s.to_string(),
    }
}

fn read_type(key: String, name: String, file_name: String, photometry: &Value) -> LuminaryType {
    let meta = photometry.get("meta").cloned().unwrap_or(Value::Null);
    let shown = strip_ies(if name.is_empty() { &file_name } else { &name });
    LuminaryType {
        name: if shown.is_empty() { key.clone() } else { shown },
        key,
        manufacturer: str_of(&meta, "MANUFAC"),
        luminaire: str_of(&meta, "LUMINAIRE"),
        catalog: str_of(&meta, "LUMCAT"),
        file_name,
        quantity: 0,
    }
}

/// Every fixture type with at least one fixture, in file order. Counts follow what Luminary Pro
/// shows: a fixture with no position, or naming a type that is not there, is not counted.
fn read_file(file: &Value) -> Option<Vec<LuminaryType>> {
    let fixtures = file.get("fixtures").and_then(Value::as_array);
    if !file.is_object() || (fixtures.is_none() && !file.get("photometry").is_some_and(Value::is_object)) {
        return None;
    }
    let mut types: Vec<LuminaryType> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, t) in array(file, "fixtureTypes").iter().take(MAX_TYPES).enumerate() {
        if let Some(ph) = has_photometry(t.get("photometry")) {
            let id = str_of(t, "id");
            // "#n" is never a real id by accident, so a nameless type can't collide with one.
            let key = if id.is_empty() { format!("#{}", i + 1) } else { id };
            if seen.insert(key.clone()) {
                types.push(read_type(key, str_of(t, "name"), str_of(t, "fileName"), ph));
            }
        }
    }
    // Format 1: one photometry, and fixtures that name no type.
    if types.is_empty()
        && let Some(ph) = has_photometry(file.get("photometry"))
    {
        types.push(read_type("t1".into(), String::new(), str_of(file, "photometryName"), ph));
    }
    let fallback = types.first().map(|t| t.key.clone()).unwrap_or_default();
    let finite = |f: &Value, k: &str| f.get(k).and_then(Value::as_f64).is_some_and(f64::is_finite);
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for f in fixtures.into_iter().flatten().take(MAX_FIXTURES) {
        if f.is_object() && finite(f, "x") && finite(f, "y") {
            let id = str_of(f, "typeId");
            *counts.entry(if id.is_empty() { fallback.clone() } else { id }).or_default() += 1;
        }
    }
    for t in &mut types {
        t.quantity = counts.get(&t.key).copied().unwrap_or(0);
    }
    types.retain(|t| t.quantity > 0);
    Some(types)
}

// ── Matching ────────────────────────────────────────────────────────────────

// ponytail: ASCII only, so an accented letter splits a word; add accent folding if a library needs it.
fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).map(String::from).collect()
}

#[derive(Clone, Copy)]
struct Span {
    source: usize,
    start: usize,
    end: usize,
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.source == inner.source && outer.start <= inner.start && outer.end >= inner.end
}

fn find_run(sources: &[Vec<String>], needle: &[String]) -> Option<Span> {
    if needle.is_empty() {
        return None;
    }
    for (source, hay) in sources.iter().enumerate() {
        for start in 0..hay.len() {
            if hay.get(start..start + needle.len()) == Some(needle) {
                return Some(Span { source, start, end: start + needle.len() });
            }
        }
    }
    None
}

/// `code` with punctuation and spaces taken out, spelled by a contiguous run of tokens.
fn find_code(sources: &[Vec<String>], code: &str) -> Option<Span> {
    if code.is_empty() {
        return None;
    }
    for (source, hay) in sources.iter().enumerate() {
        for start in 0..hay.len() {
            let mut joined = String::new();
            for (end, t) in hay.iter().enumerate().skip(start) {
                if joined.len() >= code.len() {
                    break;
                }
                joined.push_str(t);
                if joined == code {
                    return Some(Span { source, start, end: end + 1 });
                }
            }
        }
    }
    None
}

/// A short code turns up inside ordinary words ("0-10V"), so it counts only as a model number.
fn distinctive(code: &str) -> bool {
    code.len() >= 6 || (code.len() >= 4 && code.bytes().any(|b| b.is_ascii_digit()) && code.bytes().any(|b| b.is_ascii_lowercase()))
}

fn without_parens(s: &str) -> String {
    let mut depth = 0u32;
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// A library product's match tokens, computed once per import rather than once per type.
struct ProductTokens {
    brand: Vec<String>,
    model: Vec<String>,
    code: String,
}

fn product_tokens(products: &[Product]) -> Vec<ProductTokens> {
    products
        .iter()
        .map(|p| {
            let brand = tokens(&p.brand);
            let model = tokens(&without_parens(&p.name)).into_iter().filter(|w| !brand.contains(w)).collect();
            ProductTokens { brand, model, code: tokens(&p.code).concat() }
        })
        .collect()
}

/// The product a type is when the evidence is specific (`Some`), plus best guesses. A wrong model
/// on a quote is worse than a blank one, so a tie or a partial overlap never chooses.
fn match_type<'a>(t: &LuminaryType, products: &'a [Product], toks: &[ProductTokens]) -> (Option<&'a str>, Vec<&'a str>) {
    let sources: Vec<Vec<String>> =
        [t.luminaire.as_str(), &t.catalog, &t.name, &strip_ies(&t.file_name)].into_iter().map(tokens).filter(|s| !s.is_empty()).collect();
    let haystack: std::collections::HashSet<&String> = sources.iter().flatten().collect();
    let maker = tokens(&t.manufacturer);
    let brand_hit = |k: &ProductTokens| k.brand.iter().any(|b| maker.contains(b));
    let brand_known = !maker.is_empty() && toks.iter().any(brand_hit);

    let mut hits: Vec<(&str, Span)> = Vec::new();
    let mut scored: Vec<(&str, f64)> = Vec::new();
    for (p, k) in products.iter().zip(toks).filter(|(p, k)| !is_builtin(&p.id) && p.brand != "Generic" && (!brand_known || brand_hit(k))) {
        let ProductTokens { model, code, .. } = k;
        let by_name = find_run(&sources, model);
        let by_code = if distinctive(code) { find_code(&sources, code) } else { None };
        if let Some(span) = by_name.or(by_code) {
            let best = match (by_name, by_code) {
                (Some(n), Some(c)) if c.end - c.start > n.end - n.start => c,
                _ => span,
            };
            hits.push((&p.id, best));
            continue;
        }
        let shared = model.iter().filter(|w| haystack.contains(w) && w.bytes().any(|b| b.is_ascii_digit())).count();
        let overlap = model.iter().filter(|w| haystack.contains(w)).count();
        if shared > 0 || overlap >= 2 {
            scored.push((&p.id, overlap as f64 / model.len() as f64 + shared as f64));
        }
    }
    let winner = hits
        .iter()
        .enumerate()
        .find(|(i, (_, h))| hits.iter().enumerate().all(|(j, (_, o))| *i == j || (contains(*h, *o) && !contains(*o, *h))))
        .map(|(_, (id, _))| *id);
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let suggestions = hits.iter().map(|h| h.0).chain(scored.iter().map(|s| s.0)).take(6).collect();
    (winner, suggestions)
}

pub(super) fn run_import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let bytes = read(path).map_err(|e| bad(CMD, format!("{path}: {e}")))?;
    let file: Value = serde_json::from_slice(&bytes).map_err(|e| bad(CMD, format!("{path}: not a Luminary Pro file ({e})")))?;
    let types = read_file(&file).ok_or_else(|| bad(CMD, format!("{path}: not a Luminary Pro file: no fixtures in it")))?;
    let choices = match p.get("choices") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(bad(CMD, "`choices` must be an object of type key to product id (\"\" skips)")),
    };
    let library = s.csd_library.clone();

    // Resolve every type before touching the drawing, so a bad choice changes nothing.
    let mut rows: Vec<Value> = Vec::new();
    let mut picked: Vec<(String, u64)> = Vec::new();
    let toks = product_tokens(&library);
    for t in &types {
        let (sure, suggestions) = match_type(t, &library, &toks);
        let product = match choices.get(&t.key) {
            Some(Value::String(c)) if c.is_empty() => None,
            Some(Value::String(c)) => {
                let found = find_product(&library, c).filter(|f| !is_builtin(&f.id));
                Some(found.ok_or_else(|| bad(CMD, format!("choice for \"{}\": no library product \"{c}\"", t.key)))?.id.clone())
            }
            Some(_) => return Err(bad(CMD, format!("choice for \"{}\" must be a product id string", t.key))),
            None => sure.map(String::from),
        };
        if let Some(id) = &product {
            picked.push((id.clone(), t.quantity));
        }
        rows.push(json!({
            "key": t.key, "name": t.name, "manufacturer": t.manufacturer, "luminaire": t.luminaire, "catalog": t.catalog,
            "quantity": t.quantity, "product": product, "matched": sure.is_some(), "suggestions": suggestions,
        }));
    }
    let skipped = rows.len() - picked.len();
    if p.get("preview").and_then(Value::as_bool) == Some(true) {
        return Ok(json!({ "types": rows, "fixtures": 0, "skipped": skipped }));
    }
    // Refuse before adding anything: the schedule holds at most MAX_ROWS products.
    let existing = schedule::read(s.doc()?);
    let mut ids: std::collections::HashSet<&str> = existing.iter().map(|(id, _)| id.as_str()).collect();
    ids.extend(picked.iter().map(|(id, _)| id.as_str()));
    if ids.len() > schedule::MAX_ROWS {
        return Err(bad(CMD, format!("the import would take the unplaced list past {} items; nothing was added", schedule::MAX_ROWS)));
    }
    let mut fixtures = 0;
    for (id, qty) in &picked {
        schedule::run_unplaced_add(s, &json!({ "product": id, "qty": qty }))?;
        fixtures += qty;
    }
    Ok(json!({ "types": rows, "fixtures": fixtures, "skipped": skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PH: &str = r#""vAngles":[0],"hAngles":[0],"candela":[[1]]"#;

    fn import(file: &str, params: Value) -> Result<Value> {
        let path = std::env::temp_dir().join(format!("cadcraft-lpro-{}-{:p}.lpro", std::process::id(), file));
        std::fs::write(&path, file).unwrap();
        let mut s = Session::new();
        let mut params = params;
        if let Some(m) = params.as_object_mut() {
            m.entry("path").or_insert(json!(path.to_str().unwrap()));
        }
        let r = s.execute("csd.import.lpro", &params);
        let _ = std::fs::remove_file(&path);
        r
    }

    fn counted(src: &str) -> Vec<(String, u64)> {
        let v: Value = serde_json::from_str(src).unwrap();
        read_file(&v).unwrap().into_iter().map(|t| (t.key, t.quantity)).collect()
    }

    #[test]
    fn drops_what_luminary_drops_and_counts_entries_not_ids() {
        let src = format!(
            r#"{{"fixtureTypes":[{{"id":"a","name":"A","photometry":{{{PH}}}}},{{"id":"b","photometry":{{"meta":{{}}}}}}],
            "fixtures":[{{"id":"f","typeId":"a","x":0,"y":0}},{{"id":"f","typeId":"a","x":1,"y":1}},{{"typeId":"a","x":0}},{{"typeId":"b","x":0,"y":0}},{{"typeId":"gone","x":0,"y":0}}]}}"#
        );
        assert_eq!(counted(&src), [("a".to_string(), 2)]);
    }

    #[test]
    fn format_one_has_a_single_luminaire() {
        let src = format!(
            r#"{{"photometry":{{{PH},"meta":{{"LUMINAIRE":"Ovation E-910FC"}}}},"photometryName":"e910.ies","fixtures":[{{"x":0,"y":0}},{{"x":1,"y":1}}]}}"#
        );
        let v: Value = serde_json::from_str(&src).unwrap();
        let t = read_file(&v).unwrap();
        assert_eq!((t[0].name.as_str(), t[0].luminaire.as_str(), t[0].quantity), ("e910", "Ovation E-910FC", 2));
    }

    #[test]
    fn imports_matched_types_to_unplaced_and_honours_choices() {
        let lib = cadcraft_csd::sample_products();
        let Some(p) = lib.iter().find(|p| !is_builtin(&p.id) && p.brand != "Generic" && !tokens(&without_parens(&p.name)).is_empty()) else {
            return;
        };
        let file = format!(
            r#"{{"fixtureTypes":[{{"id":"a","name":"x","photometry":{{{PH},"meta":{{"LUMINAIRE":{}}}}}}},{{"id":"b","name":"zzz","photometry":{{{PH}}}}}],
            "fixtures":[{{"typeId":"a","x":0,"y":0}},{{"typeId":"a","x":1,"y":0}},{{"typeId":"b","x":1,"y":0}}]}}"#,
            json!(p.name)
        );
        let r = import(&file, json!({ "preview": true })).unwrap();
        assert_eq!(r["types"][1]["product"], Value::Null, "{r}");
        let r = import(&file, json!({ "choices": { "a": "", "b": p.id } })).unwrap();
        assert_eq!((r["fixtures"].clone(), r["skipped"].clone()), (json!(1), json!(1)));
        assert!(import(&file, json!({ "choices": { "a": "NOPE" } })).is_err());
    }

    #[test]
    fn model_name_ties_and_short_codes_do_not_choose() {
        let prod =
            |id: &str, name: &str, code: &str| Product { id: id.into(), name: name.into(), code: code.into(), brand: "Acme".into(), ..sample() };
        let lib = [
            prod("ww", "Acme E-260WW", "E260WW"),
            prod("ip", "Acme E-260WW IP", "E260IP"),
            prod("s", "Acme Cove S", "CS1"),
            prod("m", "Acme Cove M", "CM1"),
            prod("patch", "Acme Bay", "Patch"),
        ];
        let ty = |lum: &str, cat: &str| LuminaryType {
            key: "t".into(),
            name: String::new(),
            manufacturer: String::new(),
            luminaire: lum.into(),
            catalog: cat.into(),
            file_name: String::new(),
            quantity: 1,
        };
        assert_eq!(match_type(&ty("Acme E-260WW IP", ""), &lib, &product_tokens(&lib)).0, Some("ip"));
        assert_eq!(match_type(&ty("Acme E-260WW", ""), &lib, &product_tokens(&lib)).0, Some("ww"));
        assert_eq!(match_type(&ty("Cove RGBW", ""), &lib, &product_tokens(&lib)).0, None);
        assert_eq!(match_type(&ty("", "Patch lead included"), &lib, &product_tokens(&lib)).0, None);
    }

    fn sample() -> Product {
        cadcraft_csd::sample_products().remove(0)
    }

    #[test]
    fn duplicate_type_ids_are_skipped_and_nameless_types_get_their_own_key() {
        let src = format!(
            r##"{{"fixtureTypes":[{{"id":"a","photometry":{{{PH}}}}},{{"id":"a","photometry":{{{PH}}}}},{{"photometry":{{{PH}}}}},{{"id":"t3","photometry":{{{PH}}}}}],
            "fixtures":[{{"typeId":"a","x":0,"y":0}},{{"typeId":"t3","x":0,"y":0}},{{"typeId":"#3","x":0,"y":0}}]}}"##
        );
        assert_eq!(counted(&src), [("a".to_string(), 1), ("#3".to_string(), 1), ("t3".to_string(), 1)]);
    }

    #[test]
    fn an_import_past_the_row_cap_adds_nothing() {
        let n = schedule::MAX_ROWS + 1;
        let mut s = Session::new();
        s.csd_library = std::sync::Arc::new(
            (0..n).map(|i| Product { id: format!("P{i}"), name: format!("P{i}"), code: format!("P{i}"), ..Product::default() }).collect(),
        );
        let types: Vec<String> = (0..n).map(|i| format!(r#"{{"id":"k{i}","photometry":{{{PH}}}}}"#)).collect();
        let fixtures: Vec<String> = (0..n).map(|i| format!(r#"{{"typeId":"k{i}","x":0,"y":0}}"#)).collect();
        let path = std::env::temp_dir().join(format!("cadcraft-lpro-cap-{}.lpro", std::process::id()));
        std::fs::write(&path, format!(r#"{{"fixtureTypes":[{}],"fixtures":[{}]}}"#, types.join(","), fixtures.join(","))).unwrap();
        let choices: Map<String, Value> = (0..n).map(|i| (format!("k{i}"), json!(format!("P{i}")))).collect();
        let r = s.execute("csd.import.lpro", &json!({ "path": path.to_str().unwrap(), "choices": choices }));
        let _ = std::fs::remove_file(&path);
        assert!(r.is_err());
        assert!(schedule::read(s.doc().unwrap()).is_empty());
    }

    #[test]
    fn hostile_files_do_not_panic() {
        for file in [
            "[]",
            "null",
            "{}",
            "not json",
            r#"{"fixtures":5,"fixtureTypes":7}"#,
            r#"{"fixtures":[null,1,{"x":1e999,"y":0},{"x":"a","y":0}],"fixtureTypes":[null,{"photometry":3},{"id":5,"name":["é"],"fileName":"é.ies","photometry":{"vAngles":[1],"hAngles":[1],"candela":[1],"meta":9}}]}"#,
            r#"{"fixtures":[{"x":1,"y":1,"typeId":"é"}],"fixtureTypes":[{"fileName":"ié","photometry":{"vAngles":[1],"hAngles":[1],"candela":[1]}}]}"#,
        ] {
            let r = import(file, json!({}));
            assert!(r.is_ok() || r.is_err(), "{file}");
        }
        assert!(import("{}", json!({})).is_err());
        assert!(import("{\"fixtures\":[]}", json!({ "choices": 3 })).is_err());
        assert!(import("{}", json!({ "path": "/nonexistent/x.lpro" })).is_err());
    }
}
