use cadcraft_doc::Handle;
use serde_json::json;

use crate::Session;

const CONSOLE: &str = "CHAMMQ50";
const NODE: &str = "epix-flex-drive";

fn place(s: &mut Session, product: &str, at: [f64; 2]) -> Handle {
    let r = s.execute("csd.place", &json!({ "product": product, "at": at })).unwrap();
    Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap()
}

fn unplaced(s: &mut Session) -> Vec<(String, u64)> {
    let r = s.execute("csd.unplaced.list", &json!({})).unwrap();
    r["items"].as_array().unwrap().iter().map(|i| (i["product"].as_str().unwrap().to_string(), i["qty"].as_u64().unwrap())).collect()
}

#[test]
fn unplaced_items_add_merge_set_remove_and_undo() {
    let mut s = Session::new();
    s.execute("csd.unplaced.add", &json!({ "product": NODE, "qty": 2 })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": "b565fc" })).unwrap();
    assert_eq!(s.execute("csd.unplaced.add", &json!({ "product": NODE })).unwrap()["index"], 0);
    assert_eq!(unplaced(&mut s), [(NODE.to_string(), 3), ("b565fc".to_string(), 1)]);
    let first = s.execute("csd.unplaced.list", &json!({})).unwrap()["items"][0].clone();
    assert_eq!((first["code"].as_str(), first["category"].as_str()), (Some("EPIX FLEX DRIVE"), Some("Sample")));
    s.execute("csd.unplaced.set", &json!({ "index": 1, "qty": 7 })).unwrap();
    assert_eq!(unplaced(&mut s)[1].1, 7);
    for bad in [json!({ "index": 1, "qty": 0 }), json!({ "index": 5, "qty": 1 }), json!({ "index": 0 })] {
        assert!(s.execute("csd.unplaced.set", &bad).is_err());
    }
    assert!(s.execute("csd.unplaced.add", &json!({ "product": "nope" })).is_err());
    s.execute("csd.unplaced.remove", &json!({ "index": 0 })).unwrap();
    assert_eq!(unplaced(&mut s), [("b565fc".to_string(), 7)]);
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(unplaced(&mut s).len(), 2);
}

#[test]
fn unplaced_items_survive_a_dxf_save_and_reopen() {
    let mut s = Session::new();
    s.execute("csd.unplaced.add", &json!({ "product": NODE, "qty": 4 })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": "b565fc", "qty": 2 })).unwrap();
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    assert_eq!(unplaced(&mut t), [(NODE.to_string(), 4), ("b565fc".to_string(), 2)]);
}

#[test]
fn placing_a_product_takes_one_off_its_unplaced_row_and_place_arms_it() {
    let mut s = Session::new();
    s.execute("csd.unplaced.add", &json!({ "product": NODE, "qty": 2 })).unwrap();
    assert_eq!(s.execute("csd.unplaced.place", &json!({ "index": 0 })).unwrap()["product"], NODE);
    assert_eq!(s.csd_armed.as_deref(), Some(NODE));
    place(&mut s, NODE, [0.0, 0.0]);
    assert_eq!(unplaced(&mut s), [(NODE.to_string(), 1)]);
    place(&mut s, NODE, [300.0, 0.0]);
    assert!(unplaced(&mut s).is_empty());
    assert!(s.doc().unwrap().header.get("CSDUNPLACED").is_none());
    assert!(s.execute("csd.unplaced.place", &json!({ "index": 0 })).is_err());
}

#[test]
fn schedule_lists_control_gear_first_with_ports_connections_and_poe() {
    let mut s = Session::new();
    let missing = place(&mut s, "OVCYC1FCV2", [0.0, 200.0]);
    let poe = place(&mut s, "logic-poex", [0.0, 0.0]);
    let lamp = place(&mut s, "logic-ar111m", [200.0, 0.0]);
    let p = json!({ "from": poe.hex(), "fromSide": "rgt", "to": lamp.hex(), "toSide": "lft", "protocol": "network" });
    s.execute("csd.connect", &p).unwrap();
    s.execute("csd.device.set", &json!({ "handle": poe.hex(), "label": "PX1", "notes": "rack" })).unwrap();
    let r = s.execute("csd.schedule", &json!({})).unwrap();
    let d = r["devices"].as_array().unwrap();
    assert_eq!(d.len(), 3);
    assert_eq!((d[2]["handle"].as_str(), d[2]["controlGear"].as_bool()), (Some(missing.hex().as_str()), Some(false)));
    let x = d.iter().find(|x| x["handle"] == poe.hex()).unwrap();
    assert_eq!(
        (x["label"].as_str(), x["notes"].as_str(), x["portsInUse"].as_str(), x["connections"].as_u64(), x["poeLoad"].as_f64()),
        (Some("PX1"), Some("rack"), Some("PoE 1/3, Net 1/3"), Some(1), Some(45.0))
    );
    assert_eq!(x["controlGear"], true);
    assert_eq!(d.iter().find(|x| x["handle"] == lamp.hex()).unwrap()["poeLoad"], 0.0);
}

#[test]
fn bom_totals_placed_and_unplaced_by_product_and_exports_csv() {
    let mut s = Session::new();
    for (i, label) in ["N1", "N2"].iter().enumerate() {
        s.execute("csd.place", &json!({ "product": NODE, "at": [i as f64 * 300.0, 0.0], "label": label })).unwrap();
    }
    s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 200], "label": "=Desk, \"A\"" })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": NODE, "qty": 3 })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": "b565fc" })).unwrap();
    let rows = s.execute("csd.bom", &json!({})).unwrap()["rows"].clone();
    let rows = rows.as_array().unwrap();
    let got: Vec<(&str, u64)> = rows.iter().map(|r| (r["product"].as_str().unwrap(), r["qty"].as_u64().unwrap())).collect();
    assert_eq!(got, [("CHAMMQ50", 1), ("b565fc", 1), ("epix-flex-drive", 5)]);
    assert_eq!(rows[2]["labels"], json!(["N1", "N2"]));

    let path = std::env::temp_dir().join(format!("csd-bom-{}.csv", std::process::id()));
    s.execute("csd.bom.export", &json!({ "path": path, "format": "csv" })).unwrap();
    let csv = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("Qty,Model ID,Labels,Category,Product,Code,Brand,Notes"));
    assert_eq!(lines.next(), Some("1,CHAMMQ50,\"'=Desk, \"\"A\"\"\",Sample,MagicQ MagicQ50 Compact Console,MQ50,,"));
    assert_eq!(lines.nth(1), Some("5,epix-flex-drive,N1; N2,Sample,EPIX Flex Drive,EPIX FLEX DRIVE,,"));
    assert!(s.execute("csd.bom.export", &json!({ "path": "x", "format": "xls" })).is_err());
}
