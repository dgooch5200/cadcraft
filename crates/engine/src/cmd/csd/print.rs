//! `csd.print`: plots the sheet (title block extents, else the drawing's) to PDF, SVG or PNG.

use super::{Result, Session, Value, Vec2, bad, json, sheet};
use crate::cmd::str_param;

const CMD: &str = "csd.print";
const MM_PER_IN: f64 = 25.4;
/// Sheet units per inch (tabloid is 1632 x 1056 units for 17 x 11 in).
const UNITS_PER_IN: f64 = 96.0;

pub(super) fn run_print(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    let format = str_param(p, "format").map(str::to_ascii_lowercase).or(ext).unwrap_or_default();
    if !matches!(format.as_str(), "pdf" | "svg" | "png") {
        return Err(bad(CMD, "`format` must be pdf, svg or png (or give a `path` ending in one)"));
    }
    let (w, h) = sheet::paper_of(s.doc()?).size();
    let mut q = json!({
        "path": path, "output": format, "fit": true, "landscape": true, "margins": [0, 0, 0, 0], "physicalSize": true,
        "width": w.min(h) / UNITS_PER_IN * MM_PER_IN, "height": w.max(h) / UNITS_PER_IN * MM_PER_IN,
    });
    // Without a title block the plot falls back to the drawing's extents.
    if let Some((_, i)) = sheet::find(s, cadcraft_csd::TITLEBLOCK) {
        let lo = Vec2::new(i.insert.x, i.insert.y);
        q["plotArea"] = json!("window");
        q["window"] = json!([[lo.x, lo.y], [lo.x + w * i.scale.x, lo.y + h * i.scale.y]]);
    }
    s.execute("plot", &q)
}
