//! The patch sheet as a PDF, through the real plotter.

use cadcraft_csd::Product;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::json;

#[test]
fn patch_exports_a_pdf() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Err("no writer in this test".into()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    let mut s = Session::new();
    let fix = Product { id: "FIX".into(), name: "Spot".into(), code: "FIX".into(), channels: 20, ..Product::default() };
    s.csd_library = std::sync::Arc::new(vec![fix]);
    s.execute("csd.place", &json!({ "product": "FIX", "at": [0, 0], "label": "A" })).unwrap();
    let path = std::env::temp_dir().join(format!("csd-patch-{}.pdf", std::process::id()));
    let r = s.execute("csd.patch.export", &json!({ "path": path, "format": "pdf", "footer": ["Venue", "Rev A"] })).unwrap();
    assert!(r["rows"].as_u64().unwrap() >= 1);
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(bytes.starts_with(b"%PDF-") && bytes.len() > 500);
}
