//! `csd.print`: plots a sheet (title block extents, else the drawing's) to PDF, SVG or PNG.

use super::{Result, Session, Value, bad, json, page, sheet};
use crate::cmd::str_param;

const CMD: &str = "csd.print";
const MM_PER_IN: f64 = 25.4;
/// Sheet units per inch (tabloid is 1632 x 1056 units for 17 x 11 in).
const UNITS_PER_IN: f64 = 96.0;

pub(super) fn run_print(s: &mut Session, p: &Value) -> Result<Value> {
    let q = plot_query(s, p)?;
    s.execute("plot", &q)
}

fn plot_query(s: &Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    let format = str_param(p, "format").map(str::to_ascii_lowercase).or(ext).unwrap_or_default();
    if !matches!(format.as_str(), "pdf" | "svg" | "png") {
        return Err(bad(CMD, "`format` must be pdf, svg or png (or give a `path` ending in one)"));
    }
    let paper = sheet::paper_of(s.doc()?);
    let (w, h) = paper.size();
    let ps = page::pages(s);
    let window = |g: &page::Page| json!([[g.at.x, g.at.y], [g.at.x + w * g.scale.x, g.at.y + h * g.scale.y]]);
    if str_param(p, "page") == Some("all") {
        if format != "pdf" {
            return Err(bad(CMD, "`page: \"all\"` needs a PDF (png and svg hold one sheet)"));
        }
        let pages: Vec<Value> = ps.iter().map(|g| json!({ "window": window(g), "paper": paper.id(), "landscape": true })).collect();
        return Ok(json!({ "path": path, "output": "pdf", "pages": pages, "margins": [0, 0, 0, 0] }));
    }
    let mut q = json!({
        "path": path, "output": format, "fit": true, "landscape": true, "margins": [0, 0, 0, 0], "physicalSize": true,
        "width": w.min(h) / UNITS_PER_IN * MM_PER_IN, "height": w.max(h) / UNITS_PER_IN * MM_PER_IN,
    });
    let n = if p.get("page").is_some() { page::page_param(CMD, p, &ps)? } else { 0 };
    // Without a title block the plot falls back to the drawing's extents.
    if let Some(g) = ps.get(n) {
        q["plotArea"] = json!("window");
        q["window"] = window(g);
    }
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_picks_a_sheet_and_all_lists_every_sheet() {
        let mut s = Session::new();
        s.execute("csd.page.add", &json!({ "name": "Riser" })).unwrap();
        let q = |p: Value| plot_query(&s, &p);
        assert_eq!(q(json!({ "path": "a.pdf" })).unwrap()["window"][0], json!([0.0, 0.0]));
        assert_eq!(q(json!({ "path": "a.pdf", "page": "Riser" })).unwrap()["window"][0], json!([1832.0, 0.0]));
        assert_eq!(q(json!({ "path": "a.pdf", "page": 2 })).unwrap()["window"][1], json!([3464.0, 1056.0]));
        assert!(q(json!({ "path": "a.pdf", "page": 3 })).is_err());
        let all = q(json!({ "path": "a.pdf", "page": "all" })).unwrap();
        assert_eq!(all["pages"].as_array().map(Vec::len), Some(2));
        assert_eq!(all["pages"][1]["paper"], "tabloid");
        assert!(q(json!({ "path": "a.png", "page": "all" })).unwrap_err().to_string().contains("PDF"));
    }
}
