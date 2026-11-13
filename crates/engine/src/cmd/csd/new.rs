//! `csd.new`: a new drawing with a title block and line key, optionally filled from a `.cpsd` template.

use serde_json::{Value, json};

use super::{cpsd, sheet};
use crate::cmd::str_param;
use crate::{Result, Session};

pub(super) fn run_new(s: &mut Session, p: &Value) -> Result<Value> {
    let previous = s.active;
    let template = str_param(p, "template");
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
