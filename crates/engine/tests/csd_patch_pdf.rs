//! The patch sheet as a PDF, through the real plotter.

use cadcraft_csd::Product;
use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::json;

static TEXT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
// The io hook and TEXT are process-global, so the tests take turns.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn capture_text() {
    set_io(IoHooks {
        read: |_, _| Err("no reader in this test".into()),
        write: |_, _| Err("no writer in this test".into()),
        plot: Some(|d, space, opts| {
            let t: Vec<String> = d
                .space(&cadcraft_doc::Space::Model)
                .into_iter()
                .flat_map(|st| st.iter())
                .filter_map(|e| if let cadcraft_doc::EntityKind::Text(t) = &e.kind { Some(t.value.clone()) } else { None })
                .collect();
            *TEXT.lock().unwrap() = t.join("\n");
            cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())
        }),
    });
}

#[test]
fn patch_exports_a_pdf() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    capture_text();
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
    assert!(TEXT.lock().unwrap().contains("Universe 1  20 of 512 channels used"));
}

#[test]
fn patch_pdf_has_a_page_column_only_with_two_pages() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    capture_text();
    let mut s = Session::new();
    let fix = Product { id: "FIX".into(), name: "Spot".into(), code: "FIX".into(), channels: 20, ..Product::default() };
    s.csd_library = std::sync::Arc::new(vec![fix]);
    let path = std::env::temp_dir().join(format!("csd-patch-pages-{}.pdf", std::process::id()));
    let export = |s: &mut Session| {
        s.execute("csd.patch.export", &json!({ "path": path, "format": "pdf" })).unwrap();
        TEXT.lock().unwrap().clone()
    };
    s.execute("csd.place", &json!({ "product": "FIX", "at": [100, 500], "label": "A" })).unwrap();
    assert!(!export(&mut s).contains("Page"));
    s.execute("csd.page.add", &json!({})).unwrap();
    s.execute("csd.page.rename", &json!({ "page": 2, "name": "Roof" })).unwrap();
    s.execute("csd.place", &json!({ "product": "FIX", "at": [2000, 500], "label": "B" })).unwrap();
    let text = export(&mut s);
    let _ = std::fs::remove_file(&path);
    assert!(text.contains("Page") && text.contains("Drawing 1") && text.contains("Roof"), "{text}");
}
