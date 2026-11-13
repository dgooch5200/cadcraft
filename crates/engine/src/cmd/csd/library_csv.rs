//! Spreadsheet CSV import and export of the product library.

use super::{CommandSpec, Result, Session, Value, always, bad, json, read_library, run_library_set, str_param, write_library};
use cadcraft_csd::{csv_records, library_csv};

pub(super) fn specs() -> [CommandSpec; 2] {
    [
        CommandSpec::new("csd.library.import_csv", "Import Library CSV", run_import)
            .params("{path | text: CSV with the web app's headers (older spellings accepted)} → {count, skipped}; replaces the library like csd.library.set, keeping pictures of ids it already had")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.library.export_csv", "Export Library CSV", run_export)
            .params("{path?} → {path, count} when `path` is given, else {csv, count}")
            .noundo()
            .enabled(always),
    ]
}

fn run_import(s: &mut Session, p: &Value) -> Result<Value> {
    let text = match (str_param(p, "text"), str_param(p, "path")) {
        (Some(t), _) => t.to_string(),
        (None, Some(path)) => {
            let bytes = read_library(path).map_err(|e| bad("csd.library.import_csv", format!("{path}: {e}")))?;
            String::from_utf8(bytes).map_err(|_| bad("csd.library.import_csv", format!("{path}: not UTF-8 text")))?
        }
        _ => return Err(bad("csd.library.import_csv", "`path` or `text` is required")),
    };
    let records = csv_records(&text).map_err(|e| bad("csd.library.import_csv", e))?;
    let r = run_library_set(s, &json!({ "products": records }))?;
    let count = r["count"].as_u64().unwrap_or(0);
    Ok(json!({ "count": count, "skipped": (records.len() as u64).saturating_sub(count) }))
}

fn run_export(s: &mut Session, p: &Value) -> Result<Value> {
    let csv = library_csv(&s.csd_library);
    let count = s.csd_library.iter().filter(|x| !x.missing).count();
    match str_param(p, "path") {
        Some(path) => {
            write_library(path, csv.as_bytes()).map_err(|e| bad("csd.library.export_csv", format!("{path}: {e}")))?;
            Ok(json!({ "path": path, "count": count }))
        }
        None => Ok(json!({ "csv": csv, "count": count })),
    }
}
