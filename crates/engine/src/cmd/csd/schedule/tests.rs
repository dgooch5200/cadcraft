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
    let sw = super::super::tests::poe_switch(&mut s);
    let poe = place(&mut s, sw, [0.0, 0.0]);
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

fn info(s: &mut Session, h: Handle) -> serde_json::Value {
    s.execute("csd.device.info", &json!({ "handle": h.hex() })).unwrap()
}

fn set(s: &mut Session, h: Handle, mut p: serde_json::Value) -> crate::Result<serde_json::Value> {
    p["handle"] = json!(h.hex());
    s.execute("csd.device.set", &p)
}

#[test]
fn mounting_accessories_and_supply_by_others_round_trip_through_a_dxf() {
    let mut s = Session::new();
    let rack = place(&mut s, CONSOLE, [0.0, 0.0]);
    let node = place(&mut s, NODE, [300.0, 0.0]);
    let before = info(&mut s, node);
    assert!(before["mount"].is_null() && before["accessories"] == json!([]) && before["supplyByOthers"] == json!(false));
    let accs = json!([{ "product": "b565fc", "qty": 2 }, { "product": "b565fc", "qty": 1 }, { "product": "x", "qty": 0 }]);
    set(&mut s, node, json!({ "mount": rack.hex(), "accessories": accs, "supplyByOthers": true })).unwrap();
    let i = info(&mut s, node);
    assert_eq!(i["mount"]["handle"], json!(rack.hex()));
    assert_eq!((i["accessories"].as_array().unwrap().len(), i["accessories"][0]["qty"].clone()), (1, json!(3)));
    assert_eq!(i["supplyByOthers"], json!(true));

    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    assert_eq!(info(&mut t, node), i);

    set(&mut s, node, json!({ "mount": "", "accessories": [], "supplyByOthers": false })).unwrap();
    assert_eq!(info(&mut s, node), before);
}

#[test]
fn mounting_refuses_itself_cycles_and_bad_input() {
    let mut s = Session::new();
    let (a, b) = (place(&mut s, CONSOLE, [0.0, 0.0]), place(&mut s, NODE, [300.0, 0.0]));
    assert!(set(&mut s, a, json!({ "mount": a.hex() })).is_err());
    set(&mut s, b, json!({ "mount": a.hex() })).unwrap();
    assert!(set(&mut s, a, json!({ "mount": b.hex() })).is_err());
    assert!(set(&mut s, a, json!({ "mount": "FFFFF" })).is_err());
    assert!(set(&mut s, a, json!({ "accessories": [{ "product": "p", "qty": -1 }] })).is_err());
    assert!(set(&mut s, a, json!({ "supplyByOthers": "yes" })).is_err());
}

#[test]
fn bom_lists_accessories_and_leaves_out_supply_by_others() {
    let mut s = Session::new();
    let a = place(&mut s, NODE, [0.0, 0.0]);
    let b = place(&mut s, NODE, [300.0, 0.0]);
    set(&mut s, a, json!({ "accessories": [{ "product": "b565fc", "qty": 2 }] })).unwrap();
    set(&mut s, b, json!({ "accessories": [{ "product": "b565fc", "qty": 5 }] })).unwrap();
    let rows = |s: &mut Session| -> Vec<(String, u64)> {
        let r = s.execute("csd.bom", &json!({})).unwrap();
        r["rows"].as_array().unwrap().iter().map(|r| (r["product"].as_str().unwrap().into(), r["qty"].as_u64().unwrap())).collect()
    };
    let all = rows(&mut s);
    assert!(all.contains(&("b565fc".into(), 7)) && all.contains(&(NODE.into(), 2)));
    set(&mut s, b, json!({ "supplyByOthers": true })).unwrap();
    let left = rows(&mut s);
    assert!(left.contains(&("b565fc".into(), 2)) && left.contains(&(NODE.into(), 1)));
    let sched = s.execute("csd.schedule", &json!({})).unwrap();
    assert!(sched["devices"].as_array().unwrap().iter().any(|d| d["supplyByOthers"] == json!(true)));
}

#[test]
fn moving_a_rack_carries_what_is_mounted_in_it_once() {
    let mut s = Session::new();
    let rack = place(&mut s, CONSOLE, [0.0, 0.0]);
    let node = place(&mut s, NODE, [300.0, 0.0]);
    let inner = place(&mut s, NODE, [600.0, 0.0]);
    set(&mut s, node, json!({ "mount": rack.hex() })).unwrap();
    set(&mut s, inner, json!({ "mount": node.hex() })).unwrap();
    let x = |s: &mut Session, h: Handle| info(s, h)["at"][0].as_f64().unwrap();
    s.execute("move", &json!({ "handles": [rack.hex()], "from": [0, 0], "to": [10, 0] })).unwrap();
    assert_eq!((x(&mut s, rack), x(&mut s, node), x(&mut s, inner)), (10.0, 310.0, 610.0));
    // Moved together, the mounted device is not carried a second time.
    s.execute("move", &json!({ "handles": [rack.hex(), node.hex()], "from": [0, 0], "to": [5, 0] })).unwrap();
    assert_eq!((x(&mut s, rack), x(&mut s, node), x(&mut s, inner)), (15.0, 315.0, 615.0));
    s.undo().unwrap();
    assert_eq!(x(&mut s, node), 310.0);
}

#[test]
fn moving_a_device_wholly_into_a_card_mounts_it_and_out_unmounts() {
    let mut s = Session::new();
    let rack = place(&mut s, CONSOLE, [0.0, 0.0]);
    let inner = place(&mut s, NODE, [600.0, 0.0]);
    s.execute("csd.size", &json!({ "handles": [rack.hex()], "w": 1000, "h": 1000 })).unwrap();
    let mount = |s: &mut Session| info(s, inner)["mount"]["handle"].as_str().map(str::to_string);
    // Clipping the edge is only passing over.
    s.execute("move", &json!({ "handles": [inner.hex()], "from": [600, 0], "to": [480, 0] })).unwrap();
    assert_eq!(mount(&mut s), None);
    s.execute("move", &json!({ "handles": [inner.hex()], "from": [480, 0], "to": [10, 0] })).unwrap();
    assert_eq!(mount(&mut s), Some(rack.hex()));
    s.undo().unwrap();
    assert_eq!(mount(&mut s), None);
    s.redo().unwrap();
    s.execute("move", &json!({ "handles": [inner.hex()], "from": [10, 0], "to": [900, 0] })).unwrap();
    assert_eq!(mount(&mut s), None);
}

#[test]
fn a_mounted_device_is_painted_after_its_host() {
    let mut s = Session::new();
    let inner = place(&mut s, NODE, [600.0, 0.0]);
    let rack = place(&mut s, CONSOLE, [0.0, 0.0]);
    let order = |s: &Session| s.doc().unwrap().space(&s.space()).unwrap().handles();
    let at = |s: &Session, h| order(s).iter().position(|x| *x == h).unwrap();
    assert!(at(&s, inner) < at(&s, rack));
    set(&mut s, inner, json!({ "mount": rack.hex() })).unwrap();
    assert!(at(&s, inner) > at(&s, rack));
}

#[test]
fn a_cable_to_a_mounted_device_is_lifted_over_the_cards_and_dropped_on_unmount() {
    let mut s = Session::new();
    let rack = place(&mut s, CONSOLE, [0.0, 0.0]);
    let inner = place(&mut s, NODE, [600.0, 0.0]);
    let other = place(&mut s, NODE, [1200.0, 0.0]);
    let wire = json!({ "from": inner.hex(), "fromSide": "rgt", "to": other.hex(), "toSide": "lft", "protocol": "dmx" });
    let w = Handle::parse_hex(s.execute("csd.connect", &wire).unwrap()["handle"].as_str().unwrap()).unwrap();
    let at = |s: &Session, h| s.doc().unwrap().space(&s.space()).unwrap().handles().iter().position(|x| *x == h).unwrap();
    let over = |s: &Session| [rack, inner, other].iter().all(|c| at(s, w) > at(s, *c));
    assert!(!over(&s));
    set(&mut s, inner, json!({ "mount": rack.hex() })).unwrap();
    assert!(over(&s));
    set(&mut s, inner, json!({ "mount": "" })).unwrap();
    assert!(!over(&s));
}

#[test]
fn builtin_blocks_place_but_are_on_no_list() {
    let mut s = Session::new();
    let node = place(&mut s, NODE, [0.0, 0.0]);
    let by_others = place(&mut s, cadcraft_csd::BY_OTHERS_ID, [300.0, 0.0]);
    place(&mut s, cadcraft_csd::OFF_PAGE_ID, [600.0, 0.0]);
    let rows = s.execute("csd.bom", &json!({})).unwrap()["rows"].as_array().unwrap().clone();
    assert_eq!(rows.iter().map(|r| r["product"].as_str().unwrap()).collect::<Vec<_>>(), [NODE]);
    let sched = s.execute("csd.schedule", &json!({})).unwrap();
    assert_eq!(sched["devices"].as_array().unwrap().len(), 1);
    let wire = json!({ "from": node.hex(), "fromSide": "rgt", "to": by_others.hex(), "toSide": "lft", "protocol": "dmx" });
    s.execute("csd.connect", &wire).unwrap();
    let patch = s.execute("csd.patch", &json!({})).unwrap();
    assert!(!patch.to_string().contains("__by-others"));
}

#[test]
fn builtins_stay_off_the_unplaced_list() {
    let mut s = Session::new();
    for id in ["__by-others", "__off-page"] {
        assert!(s.execute("csd.unplaced.add", &json!({ "product": id })).is_err());
    }
    assert!(unplaced(&mut s).is_empty());
}

#[test]
fn poe_over_the_port_budget_warns_on_connect_and_on_the_schedule() {
    let mut s = Session::new();
    let sw = super::super::tests::poe_switch(&mut s);
    let poe = place(&mut s, sw, [0.0, 0.0]);
    let a = place(&mut s, "logic-ar111m", [200.0, 0.0]);
    let b = place(&mut s, "logic-ar111m", [400.0, 0.0]);
    let net = |f: Handle, t: Handle| json!({ "from": f.hex(), "fromSide": "rgt", "to": t.hex(), "toSide": "lft", "protocol": "network" });
    assert!(s.execute("csd.connect", &net(poe, a)).unwrap().get("warning").is_none());
    let r = s.execute("csd.connect", &net(a, b)).unwrap();
    assert!(r["warning"].as_str().unwrap().contains("90.0W"), "{r}");
    let d = s.execute("csd.schedule", &json!({})).unwrap();
    let x = d["devices"].as_array().unwrap().iter().find(|x| x["handle"] == poe.hex()).unwrap();
    assert!(x["poeWarning"].as_str().unwrap().contains("max 75W"));
    assert!(d["devices"].as_array().unwrap().iter().find(|x| x["handle"] == a.hex()).unwrap()["poeWarning"].is_null());
}

#[test]
fn interactive_connect_echoes_the_poe_warning() {
    let mut s = Session::new();
    let sw = super::super::tests::poe_switch(&mut s);
    let poe = place(&mut s, sw, [0.0, 0.0]);
    let a = place(&mut s, "logic-ar111m", [200.0, 0.0]);
    let b = place(&mut s, "logic-ar111m", [400.0, 0.0]);
    let net = |f: Handle, t: Handle| json!({ "from": f.hex(), "fromSide": "rgt", "to": t.hex(), "toSide": "lft", "protocol": "network" });
    s.execute("csd.connect", &net(poe, a)).unwrap();
    s.start("csd.connect").unwrap();
    s.input(crate::Input::Keyword("network".into())).unwrap();
    let _ = (a, b);
    let half = cadcraft_csd::CARD_W / 2.0;
    s.input(crate::Input::Point(cadcraft_geom::Vec2::new(200.0 + half, 0.0))).unwrap();
    s.input(crate::Input::Point(cadcraft_geom::Vec2::new(400.0 - half, 0.0))).unwrap();
    assert!(s.log.last().is_some_and(|m| m.contains("90.0W")), "{:?}", s.log.last());
}
