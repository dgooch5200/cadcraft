//! The library as a spreadsheet CSV with the web app's column headers (and its older header
//! spellings on import). Cells go through [`crate::parse_products`], so clamping is the same as for
//! JSON. Pictures are not carried (the Image column is ignored).

use serde_json::{Map, Value, json};

use crate::{MAX_LIBRARY_BYTES, MAX_PRODUCTS, Product, product_record};

const MAX_COLUMNS: usize = 256;

/// (record key, header spellings; the first is written, all are read, in order of preference).
const COLUMNS: [(&str, &[&str]); 33] = [
    ("brand", &["Brand"]),
    ("cat", &["Category"]),
    ("name", &["Product Name", "name"]),
    ("id", &["Model ID", "id"]),
    ("code", &["Short Code", "Model ID"]),
    ("color", &["Brand Color (hex)"]),
    ("dmxIn", &["DMX In"]),
    ("dmxOut", &["DMX Out", "DMX Ports"]),
    ("netOut", &["PoE Ports", "Network Ports"]),
    ("netDataOut", &["Data Ports", "Net Data"]),
    ("scnIn", &["10Scene In"]),
    ("scnOut", &["10Scene Out", "10Scene Bus Ports"]),
    ("midiIn", &["MIDI In"]),
    ("midiOut", &["MIDI Out"]),
    ("v10In", &["0-10V In"]),
    ("v10Out", &["0-10V Out"]),
    ("ctIn", &["Contact In"]),
    ("ctOut", &["Contact Out"]),
    ("pwrIn", &["Power In"]),
    ("pwrOut", &["Power Out", "Power Ports"]),
    ("ep3In", &["EPIX 3P In"]),
    ("ep3Out", &["EPIX 3P Out"]),
    ("ep4In", &["EPIX 4P In"]),
    ("ep4Out", &["EPIX 4P Out"]),
    ("modes", &["DMX Modes", "modes"]),
    ("channels", &["DMX Channels", "channels"]),
    ("universes", &["Universes", "universes"]),
    ("poeWatts", &["PoE Watts", "poeWatts"]),
    ("poePortWatts", &["PoE Port Watts", "poePortWatts"]),
    ("iconW", &["Icon W"]),
    ("iconH", &["Icon H"]),
    ("notes", &["Notes"]),
    ("deviceNotes", &["Device Notes"]),
];

/// The header row this module writes.
pub fn csv_headers() -> Vec<&'static str> {
    COLUMNS.iter().filter_map(|c| c.1.first().copied()).collect()
}

/// RFC 4180 rows: quoted cells, `""` escapes, CRLF or LF, an optional BOM. Blank lines are skipped.
fn rows(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let (mut row, mut cell) = (Vec::<String>::new(), String::new());
    let (mut quoted, mut was_quoted) = (false, false);
    let mut it = text.strip_prefix('\u{feff}').unwrap_or(text).chars().peekable();
    let mut end_row = |row: &mut Vec<String>, cell: &mut String, was_quoted: &mut bool| -> Result<(), String> {
        if row.len() < MAX_COLUMNS {
            row.push(std::mem::take(cell));
        }
        cell.clear();
        *was_quoted = false;
        let r = std::mem::take(row);
        if r.iter().any(|c| !c.is_empty()) {
            if out.len() > MAX_PRODUCTS {
                return Err(format!("more than {MAX_PRODUCTS} rows"));
            }
            out.push(r);
        }
        Ok(())
    };
    while let Some(c) = it.next() {
        match (c, quoted) {
            ('"', true) if it.peek() == Some(&'"') => {
                it.next();
                cell.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if cell.is_empty() && !was_quoted => (quoted, was_quoted) = (true, true),
            (',', false) => {
                if row.len() < MAX_COLUMNS {
                    row.push(std::mem::take(&mut cell));
                }
                cell.clear();
                was_quoted = false;
            }
            ('\r', false) if it.peek() == Some(&'\n') => {}
            ('\n' | '\r', false) => end_row(&mut row, &mut cell, &mut was_quoted)?,
            _ => cell.push(c),
        }
    }
    if quoted {
        return Err("a quoted cell is never closed".into());
    }
    end_row(&mut row, &mut cell, &mut was_quoted)?;
    Ok(out)
}

/// Product records from CSV text. A row with no name or no Model ID is skipped (it cannot be
/// referenced); ids lose their whitespace to `_`, and a code equal to the raw id follows it.
pub fn csv_records(text: &str) -> Result<Vec<Value>, String> {
    if text.len() > MAX_LIBRARY_BYTES {
        return Err(format!("CSV is larger than {} MB", MAX_LIBRARY_BYTES >> 20));
    }
    let mut table = rows(text)?.into_iter();
    let header: Vec<String> = table.next().ok_or("the CSV is empty")?.iter().map(|h| h.trim().to_lowercase()).collect();
    let col = |names: &[&str]| -> Vec<usize> { names.iter().filter_map(|n| header.iter().position(|h| h == &n.to_lowercase())).collect() };
    let cols: Vec<(&str, Vec<usize>)> = COLUMNS.iter().map(|(k, names)| (*k, col(names))).collect();
    if cols.iter().all(|(_, c)| c.is_empty()) {
        return Err("no known column headers (expected Model ID, Product Name, ...)".into());
    }
    let mut out = Vec::new();
    for r in table {
        let cell = |idx: &[usize]| idx.iter().filter_map(|i| r.get(*i)).map(|s| s.trim()).find(|s| !s.is_empty()).unwrap_or_default();
        let mut o = Map::new();
        for (key, idx) in &cols {
            let v = cell(idx);
            if !v.is_empty() {
                o.insert((*key).into(), json!(v));
            }
        }
        let (Some(id), true) = (o.get("id").and_then(Value::as_str), o.contains_key("name")) else { continue };
        let norm = id.split_whitespace().collect::<Vec<_>>().join("_");
        if o.get("code").and_then(Value::as_str).is_none_or(|c| c == id) {
            o.insert("code".into(), json!(norm));
        }
        o.insert("id".into(), json!(norm));
        out.push(Value::Object(o));
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.into() }
}

/// The library as CSV text under [`csv_headers`]; products marked missing are left out.
pub fn library_csv(products: &[Product]) -> String {
    let mut out = csv_headers().iter().map(|h| quote(h)).collect::<Vec<_>>().join(",");
    for p in products.iter().filter(|p| !p.missing) {
        let rec = product_record(p);
        let cells: Vec<String> = COLUMNS
            .iter()
            .map(|(key, _)| match (rec.get(*key), *key) {
                (Some(Value::String(s)), _) => quote(s),
                (Some(Value::Number(n)), _) => n.to_string(),
                (_, "iconW") => "70".into(),
                (_, "iconH") => "55".into(),
                _ => String::new(),
            })
            .collect();
        out.push('\n');
        out.push_str(&cells.join(","));
    }
    out.push('\n');
    out
}
