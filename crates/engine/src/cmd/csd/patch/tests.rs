use std::sync::Arc;

use cadcraft_csd::Product;
use cadcraft_doc::Handle;
use serde_json::{Value, json};

use super::*;

fn prod(id: &str, name: &str, modes: &str, channels: u32, universes: u32) -> Product {
    Product {
        id: id.into(),
        name: name.into(),
        code: id.into(),
        brand: "Acme".into(),
        modes: modes.into(),
        channels,
        universes,
        ..Product::default()
    }
}

fn session() -> Session {
    let mut s = Session::new();
    s.csd_library = Arc::new(vec![
        prod("FIX", "Spot", "Std:300, Ext:200, bad, :5, Zero:0", 0, 0),
        prod("PIX", "Épix Strip", "Pixel:720", 0, 0),
        prod("PLAIN", "Plain", "", 10, 0),
        prod("DESK", "Desk", "", 0, 2),
    ]);
    s
}

fn place(s: &mut Session, product: &str, at: [f64; 2], label: &str) -> Handle {
    let r = s.execute("csd.place", &json!({ "product": product, "at": at, "label": label })).unwrap();
    Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap()
}

fn rows(s: &mut Session) -> Vec<Value> {
    s.execute("csd.patch", &json!({})).unwrap()["rows"].as_array().unwrap().clone()
}

fn addr(r: &Value) -> (u64, u64) {
    (r["universe"].as_u64().unwrap(), r["address"].as_u64().unwrap())
}

fn export(s: &mut Session, format: &str, order: &str) -> String {
    let path = std::env::temp_dir().join(format!("csd-patch-{}-{format}-{order}.csv", std::process::id()));
    s.execute("csd.patch.export", &json!({ "path": path, "format": format, "order": order })).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    text
}

#[test]
fn modes_parse_leniently_and_the_default_is_the_largest() {
    let p = prod("X", "X", "Std:300, Ext:200, bad, :5, Zero:0, Pixel: 1:1:720", 0, 0);
    let names: Vec<_> = parse_modes(&p).into_iter().collect();
    assert_eq!(names, [("Std".to_string(), 300), ("Ext".to_string(), 200), ("Pixel: 1:1".to_string(), 720)]);
    assert_eq!(widest(&names).map(|m| m.1), Some(720));
}

#[test]
fn a_chosen_mode_sets_the_channels_and_is_one_undo_step() {
    let mut s = session();
    let h = place(&mut s, "FIX", [0.0, 0.0], "A");
    assert_eq!(rows(&mut s)[0]["channels"], 300);
    s.execute("csd.device.mode", &json!({ "handles": [h.hex()], "mode": "Ext" })).unwrap();
    let r = &rows(&mut s)[0];
    assert_eq!((r["channels"].clone(), r["mode"].clone()), (json!(200), json!("Ext")));
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(rows(&mut s)[0]["mode"], Value::Null);
    s.execute("csd.device.mode", &json!({ "handles": [h.hex()], "mode": "Ext" })).unwrap();
    s.execute("csd.device.mode", &json!({ "handles": [h.hex()], "mode": "" })).unwrap();
    assert_eq!(rows(&mut s)[0]["channels"], 300);
}

#[test]
fn a_mode_the_product_does_not_offer_is_refused_and_changes_nothing() {
    let mut s = session();
    let a = place(&mut s, "FIX", [0.0, 0.0], "A");
    let b = place(&mut s, "PLAIN", [300.0, 0.0], "B");
    assert!(s.execute("csd.device.mode", &json!({ "handles": [a.hex(), b.hex()], "mode": "Ext" })).is_err());
    assert!(s.execute("csd.device.mode", &json!({ "handles": [a.hex()], "mode": "Nope" })).is_err());
    assert!(s.execute("csd.device.mode", &json!({ "handles": [a.hex()], "mode": 5 })).is_err());
    assert_eq!(rows(&mut s)[0]["mode"], Value::Null);
}

#[test]
fn fixtures_pack_into_universes_without_straddling() {
    let mut s = session();
    for (i, l) in ["A", "B", "C"].iter().enumerate() {
        place(&mut s, "FIX", [i as f64 * 300.0, 0.0], l);
    }
    place(&mut s, "DESK", [0.0, 300.0], "Desk");
    let r = rows(&mut s);
    assert_eq!(r.len(), 3, "the desk takes no channels");
    assert_eq!(r.iter().map(addr).collect::<Vec<_>>(), [(1, 1), (2, 1), (3, 1)]);
    let u = s.execute("csd.patch", &json!({})).unwrap()["universes"].clone();
    assert_eq!(u, json!({ "used": 3, "budget": 2, "over": true }));
}

#[test]
fn a_fixture_wider_than_a_universe_takes_whole_universes() {
    assert_eq!(assign(&[100, 720, 50, 0]), [(1, 1, 1), (2, 1, 2), (4, 1, 1), (4, 51, 1)]);
}

#[test]
fn patch_order_name_product_and_position() {
    let mut s = session();
    place(&mut s, "PLAIN", [600.0, 0.0], "Bar 10");
    place(&mut s, "FIX", [0.0, 0.0], "Bar 2");
    place(&mut s, "PLAIN", [300.0, -500.0], "Bar 1");
    let order = |s: &mut Session, o: &str| {
        let v = s.execute("csd.patch", &json!({ "order": o })).unwrap();
        v["rows"].as_array().unwrap().iter().map(|r| r["label"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(order(&mut s, "drawing"), ["Bar 10", "Bar 2", "Bar 1"]);
    assert_eq!(order(&mut s, "name"), ["Bar 1", "Bar 2", "Bar 10"]);
    assert_eq!(order(&mut s, "product"), ["Bar 1", "Bar 10", "Bar 2"]);
    assert_eq!(order(&mut s, "position"), ["Bar 2", "Bar 10", "Bar 1"]);
    assert!(s.execute("csd.patch", &json!({ "order": "random" })).is_err());
}

#[test]
fn magicq_csv_matches_the_consoles_own_shape() {
    let mut s = session();
    place(&mut s, "PIX", [0.0, 0.0], "Cove, \"N\"");
    let h = place(&mut s, "PLAIN", [300.0, 0.0], "B");
    place(&mut s, "FIX", [600.0, 0.0], "C");
    s.execute("csd.device.mode", &json!({ "handles": [h.hex()], "mode": null })).unwrap();
    let csv = export(&mut s, "magicq", "drawing");
    assert_eq!(
        csv,
        "Head No ,DMX ,Position ,Hang ,Manufacturer ,Model ,Mode\n\
         1 ,01-001 ,\"Cove, \"\"N\"\"\" , ,Acme ,Epix Strip ,Pixel\n\
         2 ,03-001 ,B , ,Acme ,Plain ,10 ch\n\
         3 ,03-011 ,C , ,Acme ,Spot ,Std\n"
    );
    let plain = export(&mut s, "csv", "drawing");
    assert!(plain.starts_with("Head No,Universe,Address,Label,Manufacturer,Product,Mode,Channels\r\n1,1,1,"));
    assert!(s.execute("csd.patch.export", &json!({ "path": "x", "format": "pdf" })).is_err());
}

#[test]
fn html_export_of_a_huge_spanning_fixture_stays_small() {
    let mut s = Session::new();
    let mut big = prod("BIG", "Big", "", 524_288, 0);
    big.modes = "Huge:524288".into();
    s.csd_library = Arc::new(vec![big]);
    for _ in 0..50 {
        s.execute("csd.unplaced.add", &json!({ "product": "BIG", "qty": 100 })).unwrap();
    }
    let t = std::time::Instant::now();
    let html = export(&mut s, "html", "drawing");
    assert!(t.elapsed().as_secs() < 5);
    assert!(html.len() < 5_000_000, "{}", html.len());
    assert!(html.contains("continued through universe 1024"));
}

#[test]
fn html_export_groups_by_universe_and_escapes_names() {
    let mut s = session();
    place(&mut s, "PIX", [0.0, 0.0], "<script>x</script> & Co");
    place(&mut s, "PLAIN", [300.0, 0.0], "B");
    let html = export(&mut s, "html", "drawing");
    assert!(html.contains("@page { size: letter landscape"));
    for u in ["Universe 1", "Universe 3"] {
        assert!(html.contains(u), "{u}");
    }
    assert!(html.contains("spans 2 universes, continued through universe 2"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("http"));
}

#[test]
fn ascii_folds_accents_and_drops_the_rest() {
    assert_eq!(ascii("ÉPIX Ø – “Ünï” ×2 \u{4e2d}"), "EPIX O - \"Uni\" x2");
}

#[test]
fn the_chosen_mode_survives_a_dxf_save_and_reopen() {
    let mut s = session();
    let h = place(&mut s, "FIX", [0.0, 0.0], "A");
    s.execute("csd.device.mode", &json!({ "handles": [h.hex()], "mode": "Ext" })).unwrap();
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.csd_library = s.csd_library.clone();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    let r = rows(&mut t);
    assert_eq!((r[0]["mode"].clone(), r[0]["channels"].clone()), (json!("Ext"), json!(200)));
}

#[test]
fn accessories_and_unplaced_products_take_addresses() {
    let mut s = session();
    let a = place(&mut s, "FIX", [0.0, 0.0], "A");
    place(&mut s, "PLAIN", [300.0, 0.0], "B");
    s.execute("csd.device.set", &json!({ "handles": [a.hex()], "accessories": [{ "product": "PLAIN", "qty": 2 }] })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": "PLAIN", "qty": 2 })).unwrap();
    let r = rows(&mut s);
    let labels: Vec<_> =
        r.iter().map(|x| (x["label"].as_str().unwrap().to_string(), x["carriedBy"].as_str().map(String::from), x["handle"].is_null())).collect();
    let (a, plain) = ("A".to_string(), "PLAIN".to_string());
    assert_eq!(
        labels,
        [
            (a.clone(), None, false),
            (plain.clone(), Some(a.clone()), true),
            (plain.clone(), Some(a), true),
            ("B".into(), None, false),
            (plain.clone(), None, true),
            (plain, None, true),
        ]
    );
    assert_eq!(addr(&r[1]), (1, 301));
}

#[test]
fn the_patch_order_and_addressing_switch_are_saved_with_the_drawing() {
    let mut s = session();
    place(&mut s, "FIX", [300.0, 0.0], "B");
    place(&mut s, "FIX", [0.0, 0.0], "A");
    assert_eq!(rows(&mut s)[0]["label"], "B");
    s.execute("csd.patch.setup", &json!({ "order": "name" })).unwrap();
    assert_eq!(rows(&mut s)[0]["label"], "A", "the saved order is the default");
    assert!(s.execute("csd.patch.setup", &json!({ "order": "sideways" })).is_err());
    s.execute("csd.patch.setup", &json!({ "addressing": false })).unwrap();
    let off = s.execute("csd.patch", &json!({})).unwrap();
    assert!(off["rows"].as_array().unwrap().is_empty() && off["addressing"] == false);
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.csd_library = s.csd_library.clone();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    let r = t.execute("csd.patch", &json!({})).unwrap();
    assert_eq!((r["order"].clone(), r["addressing"].clone()), (json!("name"), json!(false)));
    s.execute("csd.patch.setup", &json!({ "order": "drawing", "addressing": true })).unwrap();
    assert_eq!(s.doc().unwrap().header.get("CSDPATCHORDER"), None);
}

#[test]
fn huge_quantities_are_clamped_and_never_expanded_past_the_row_cap() {
    let mut s = session();
    let h = place(&mut s, "FIX", [0.0, 0.0], "A");
    let accs: Vec<(&str, u64)> = (0..10).map(|_| ("PLAIN", 100_000)).collect();
    s.doc_mut()
        .unwrap()
        .modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                crate::cmd::csd::put_attr(i, "ACCS", serde_json::to_string(&accs).unwrap_or_default());
            }
        })
        .unwrap();
    let unplaced: Vec<(&str, u64)> = vec![("PLAIN", u64::MAX); 5];
    s.doc_mut().unwrap().header.set_str("CSDUNPLACED", &serde_json::to_string(&unplaced).unwrap_or_default());
    assert!(schedule::read(s.doc().unwrap()).iter().all(|r| r.1 <= schedule::MAX_QTY));
    assert!(entries(&s).unwrap().iter().map(|e| e.accessories.len()).sum::<usize>() <= MAX_ROWS);
    assert!(rows(&mut s).len() <= MAX_ROWS);
}

#[test]
fn unplaced_add_onto_a_saturated_row_does_not_overflow() {
    let mut s = session();
    s.doc_mut().unwrap().header.set_str("CSDUNPLACED", &format!("[[\"PLAIN\",{}]]", u64::MAX));
    s.execute("csd.unplaced.add", &json!({ "product": "PLAIN", "qty": 5 })).unwrap();
    assert_eq!(schedule::read(s.doc().unwrap()), [("PLAIN".to_string(), schedule::MAX_QTY)]);
}

#[test]
fn mode_channels_are_capped_and_csv_quotes_carriage_returns() {
    let p = prod("X", "X", "Big:4000000000", 0, 0);
    assert_eq!(parse_modes(&p), [("Big".to_string(), MAX_MODE_CHANNELS)]);
    assert_eq!(quoted("a\rb".into()), "\"a\rb\"");
}
