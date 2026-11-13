//! The bill of materials as a PDF, through the real plotter.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::json;

#[test]
fn bom_exports_a_pdf_with_footer_lines() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Err("no writer in this test".into()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    let mut s = Session::new();
    s.execute("csd.place", &json!({ "product": "CHAMMQ50", "at": [0, 0] })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": "epix-flex-drive", "qty": 2 })).unwrap();
    let path = std::env::temp_dir().join(format!("csd-bom-{}.pdf", std::process::id()));
    let r = s.execute("csd.bom.export", &json!({ "path": path, "format": "pdf", "footer": ["Venue", "Rev A"] })).unwrap();
    assert_eq!(r["rows"], 2);
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(bytes.starts_with(b"%PDF-") && bytes.len() > 500);
}
