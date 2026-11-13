//! `csd.new`: a new drawing with a title block and line key, optionally filled from a `.cpsd` template.

use serde_json::{Value, json};

use super::{cpsd, sheet};
use crate::cmd::str_param;
use crate::{Result, Session};

pub(super) fn run_new(s: &mut Session, p: &Value) -> Result<Value> {
    let previous = s.active;
    let template = str_param(p, "template");
    let ortho = s.settings.orthomode;
    let i = s.new_drawing(false);
    let built: Result<Value> = (|| {
        if let Some(path) = template {
            cpsd::run_import(s, &json!({ "path": path }))?;
        }
        let tb = sheet::run_titleblock(s, &json!({ "paper": p.get("paper") }))?;
        sheet::run_legend(s, &Value::Null)?;
        Ok(tb)
    })();
    match built {
        Ok(tb) => Ok(json!({ "index": i, "title": s.docs.get(i).map(|d| d.title.clone()), "paper": tb["paper"], "template": template })),
        Err(e) => {
            // A template that cannot be read leaves no half-built drawing behind.
            s.docs.remove(i);
            s.active = previous.min(s.docs.len().saturating_sub(1));
            s.settings.orthomode = ortho;
            s.bump_doc_epoch();
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn new_drawing_has_title_block_and_legend() {
        let mut s = Session::new();
        let before = s.docs.len();
        let r = s.execute("csd.new", &json!({ "paper": "letter" })).unwrap();
        assert_eq!(r["paper"], "letter");
        assert_eq!(s.docs.len(), before + 1);
        assert!(s.execute("csd.project", &json!({})).unwrap()["exists"].as_bool().unwrap());
        assert!(s.execute("csd.legend", &json!({})).is_ok());
    }

    #[test]
    fn the_new_sheet_is_17_by_11_inches() {
        let mut s = Session::new();
        s.execute("csd.new", &json!({})).unwrap();
        let (_, tb) = super::sheet::find(&s, cadcraft_csd::TITLEBLOCK).unwrap();
        let (w, h) = cadcraft_csd::Paper::Tabloid.size();
        assert!((w * tb.scale.x - 17.0).abs() < 1e-9 && (h * tb.scale.y - 11.0).abs() < 1e-9, "{:?}", tb.scale);
        let (_, lg) = super::sheet::find(&s, cadcraft_csd::LEGEND).unwrap();
        assert!((lg.scale.x - tb.scale.x).abs() < 1e-12);
        assert!((lg.insert.x - cadcraft_csd::MARGIN / 96.0).abs() < 1e-9, "{:?}", lg.insert);
    }

    #[test]
    fn fitting_an_inch_sheet_reports_its_scale_against_its_own_size() {
        let mut s = Session::new();
        s.execute("csd.new", &json!({})).unwrap();
        assert_eq!(s.execute("csd.titleblock.fit", &json!({})).unwrap()["changed"], false);
        s.execute("line", &json!({ "points": [[1.0, 1.0], [34.0, 20.0]] })).unwrap();
        let r = s.execute("csd.titleblock.fit", &json!({})).unwrap();
        assert!(r["changed"].as_bool().unwrap() && (r["scale"].as_f64().unwrap() - 2.2).abs() < 0.3, "{r}");
        let (_, tb) = super::sheet::find(&s, cadcraft_csd::TITLEBLOCK).unwrap();
        assert!((tb.scale.x - r["scale"].as_f64().unwrap() / 96.0).abs() < 1e-9);
        assert_eq!(s.execute("csd.titleblock.fit", &json!({})).unwrap()["changed"], false);
    }

    #[test]
    fn a_failed_new_drawing_keeps_the_routing_mode() {
        let mut s = Session::new();
        let path = std::env::temp_dir().join(format!("csd-new-ortho-{}.cpsd", std::process::id()));
        std::fs::write(&path, r#"{"ortho":true}"#).unwrap();
        let p = json!({ "template": path.to_str().unwrap(), "paper": "a4" });
        assert!(s.execute("csd.new", &p).is_err());
        let _ = std::fs::remove_file(&path);
        assert!(!s.settings.orthomode);
    }

    #[test]
    fn template_content_comes_along_and_a_bad_one_leaves_nothing() {
        let mut s = Session::new();
        let before = s.docs.len();
        let dir = std::env::temp_dir().join(format!("csd-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.cpsd");
        std::fs::write(&path, r#"{"title":{"project":"Starter"},"devices":[]}"#).unwrap();
        s.execute("csd.new", &json!({ "template": path.to_str().unwrap() })).unwrap();
        assert_eq!(s.docs.len(), before + 1);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(s.execute("csd.new", &json!({ "template": "/nonexistent/x.cpsd" })).is_err());
        assert_eq!(s.docs.len(), before + 1);
    }
}
