//! `csd.print` through the real plotter: each format lands on the sheet's paper.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::json;

fn text(path: &std::path::Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

#[test]
fn prints_each_format_on_the_sheet_paper() {
    prints(Some(1.0));
}

#[test]
fn prints_the_same_paper_from_an_inch_drawing() {
    prints(None);
}

/// `px`: drawing units per pixel, pinned in the drawing; `None` lets the inch drawing choose.
fn prints(px: Option<f64>) {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Err("no writer in this test".into()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    let dir = std::env::temp_dir().join(format!("csd-print-{}-{}", std::process::id(), px.is_some()));
    let mut s = Session::new();
    if let Some(k) = px {
        s.doc_mut().unwrap().header.set_f64("CSDPX", k);
    }
    s.execute("csd.titleblock", &json!({ "paper": "letter" })).unwrap();
    s.execute("csd.place", &json!({ "product": "CHAMMQ50", "at": [300.0 * px.unwrap_or(1.0 / 96.0), 400.0 * px.unwrap_or(1.0 / 96.0)] })).unwrap();
    for ext in ["pdf", "svg", "png"] {
        let path = dir.join(format!("sheet.{ext}"));
        s.execute("csd.print", &json!({ "path": path.to_string_lossy() })).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 100, "{ext}");
    }
    // 8.5 x 11 in landscape, in points.
    assert!(text(&dir.join("sheet.pdf")).contains("/MediaBox[0 0 792 612]"));
    let svg = text(&dir.join("sheet.svg"));
    if px.is_some() {
        assert!(svg.contains("viewBox=\"0 0 1056") && svg.contains(" 816"), "{}", svg.chars().take(200).collect::<String>());
    }
    assert!(svg.contains("width=\"11.0000in\" height=\"8.5000in\""), "{}", svg.chars().take(300).collect::<String>());
    if let Ok(keep) = std::env::var("CSD_PRINT_KEEP") {
        let _ = std::fs::copy(dir.join("sheet.png"), keep);
    }
    assert!(s.execute("csd.print", &json!({ "path": dir.join("x.txt").to_string_lossy() })).is_err());
    assert!(s.execute("csd.print", &json!({})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
