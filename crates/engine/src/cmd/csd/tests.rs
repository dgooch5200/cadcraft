use cadcraft_csd::{Deco, Protocol, Side, Slot, WireLink, deco_from_xdata, deco_xdata, port_point};
use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{Drawing, EntityKind, Handle, LwPolyline, RawTag};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use crate::Session;

const CONSOLE: &str = "CHAMMQ50";
const NODE: &str = "epix-flex-drive";

fn handle(r: &Value) -> Handle {
    Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap()
}

/// A console at the origin and a node at (200, 0), joined by a wire from the console's right to the node's left.
fn setup() -> (Session, Handle, Handle, Handle) {
    let mut s = Session::new();
    let a = handle(&s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 0] })).unwrap());
    let b = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [200, 60] })).unwrap());
    let w = connect(&mut s, a, b).unwrap();
    (s, a, b, w)
}

fn connect(s: &mut Session, a: Handle, b: Handle) -> crate::Result<Handle> {
    let p = json!({ "from": a.hex(), "fromSide": "rgt", "to": b.hex(), "toSide": "lft", "protocol": "netdata" });
    s.execute("csd.connect", &p).map(|r| handle(&r))
}

fn verts(s: &Session, h: Handle) -> Vec<Vec2> {
    let Some(EntityKind::LwPolyline(p)) = s.doc().unwrap().entity(h).map(|e| e.kind.clone()) else { panic!("no polyline") };
    p.vertices.iter().map(|v| v.p).collect()
}

fn port(s: &Session, dev: Handle, side: Side) -> Vec2 {
    let Some(EntityKind::Insert(i)) = s.doc().unwrap().entity(dev).map(|e| e.kind.clone()) else { panic!("no insert") };
    port_point(i.insert.xy(), i.rotation, Vec2::new(i.scale.x, i.scale.y), side, Slot::SOLO)
}

fn assert_orthogonal(v: &[Vec2]) {
    assert!(v.windows(2).all(|w| w[0].x == w[1].x || w[0].y == w[1].y), "{v:?}");
}

fn mv(s: &mut Session, h: Handle, delta: [f64; 2]) {
    s.execute("move", &json!({ "handles": [h.hex()], "delta": delta })).unwrap();
}

#[test]
fn wire_follows_a_move_and_undo_is_one_step() {
    let (mut s, a, b, w) = setup();
    let before = verts(&s, w);
    let a_port = before[0];
    assert!(a_port.near(port(&s, a, Side::Right), 1e-9));
    mv(&mut s, b, [30.0, 40.0]);
    let after = verts(&s, w);
    assert_ne!(before, after);
    assert!(after[0].near(a_port, 1e-9));
    assert!(after.last().unwrap().near(port(&s, b, Side::Left), 1e-9));
    assert!(after.last().unwrap().near(Vec2::new(195.0, 100.0), 1e-9));
    assert_orthogonal(&after);

    s.execute("undo", &json!({})).unwrap();
    assert_eq!(verts(&s, w), before);
    assert!(port(&s, b, Side::Left).near(*before.last().unwrap(), 1e-9));
}

#[test]
fn refused_connections_add_nothing() {
    let (mut s, a, b, _) = setup();
    let count = |s: &Session| s.doc().unwrap().model.len();
    let n = count(&s);
    let own = json!({ "from": a.hex(), "fromSide": "rgt", "to": a.hex(), "toSide": "lft", "protocol": "netdata" });
    assert_eq!(s.execute("csd.connect", &own).unwrap_err().to_string(), "A device cannot connect to itself");
    let none = json!({ "from": a.hex(), "fromSide": "rgt", "to": b.hex(), "toSide": "lft", "protocol": "midi" });
    assert_eq!(s.execute("csd.connect", &none).unwrap_err().to_string(), "MQ50 has no MIDI ports — check the active wire type");
    assert_eq!(count(&s), n);
}

#[test]
fn erased_device_leaves_the_wire_and_undo_relinks_it() {
    let (mut s, a, b, w) = setup();
    let before = verts(&s, w);
    s.execute("erase", &json!({ "handles": [b.hex()] })).unwrap();
    assert_eq!(verts(&s, w), before);
    s.execute("undo", &json!({})).unwrap();
    mv(&mut s, b, [0.0, 50.0]);
    assert_ne!(verts(&s, w), before);
    assert!(verts(&s, w).last().unwrap().near(port(&s, b, Side::Left), 1e-9));
    let _ = a;
}

#[test]
fn copied_wire_snaps_back_to_the_original_devices() {
    let (mut s, a, b, w) = setup();
    s.execute("copy", &json!({ "handles": [b.hex(), w.hex()], "delta": [0, 100] })).unwrap();
    let net = s.execute("csd.netlist", &json!({})).unwrap();
    assert_eq!(net["devices"].as_array().unwrap().len(), 3);
    let wires = net["wires"].as_array().unwrap();
    assert_eq!(wires.len(), 2);
    let mut starts = Vec::new();
    for wire in wires {
        let h = Handle::parse_hex(wire["handle"].as_str().unwrap()).unwrap();
        starts.push(verts(&s, h)[0]);
        assert_eq!((wire["from"].as_str().unwrap(), wire["to"].as_str().unwrap()), (a.hex().as_str(), b.hex().as_str()));
    }
    // Two wires now share a side, so they fan out instead of stacking.
    assert!(!starts[0].near(starts[1], 1e-6));
}

#[test]
fn wire_end_follows_a_rotated_device() {
    let (mut s, _, b, w) = setup();
    s.execute("rotate", &json!({ "handles": [b.hex()], "base": [200, 60], "angle": 90 })).unwrap();
    let v = verts(&s, w);
    assert!(v.last().unwrap().near(port(&s, b, Side::Left), 1e-9));
    // The node's left edge now faces down, so the final stub arrives from below.
    assert!(v[v.len() - 2].y < v[v.len() - 1].y);
    assert_orthogonal(&v);
}

#[test]
fn hostile_xdata_is_left_alone() {
    let (mut s, a, ..) = setup();
    let ghost = Handle(0xFFFF_FFFF);
    let tag = |code, v: &str| RawTag { code, value: v.into() };
    let bad_sets = [
        vec![tag(1001, "CSD"), tag(1000, "zz"), tag(1000, "garbage"), tag(1000, "")],
        WireLink { from: ghost, from_side: Side::Top, to: ghost, to_side: Side::Left, protocol: cadcraft_csd::Protocol::Dmx }.to_xdata(),
        vec![tag(1001, "CSD")],
    ];
    for xdata in bad_sets {
        let h = s
            .add_entity(EntityKind::LwPolyline(LwPolyline { vertices: vec![], closed: false, const_width: 0.0, elevation: 0.0, plinegen: false }))
            .unwrap();
        let line = s.add_entity(EntityKind::Line(cadcraft_doc::Line { a: Vec2::ZERO.to3(0.0), b: Vec2::new(1.0, 1.0).to3(0.0) })).unwrap();
        s.doc_mut().unwrap().modify_entity(h, |e| e.common.xdata = xdata.clone()).unwrap();
        mv(&mut s, line, [5.0, 5.0]);
        assert!(verts(&s, h).is_empty());
        assert_eq!(s.doc().unwrap().entity(h).unwrap().common.xdata, xdata);
    }
    let _ = a;
}

#[test]
fn links_survive_a_dxf_round_trip() {
    let (mut s, a, b, w) = setup();
    let net = s.execute("csd.netlist", &json!({})).unwrap();
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let imported = cadcraft_io::read_dxf(text.as_bytes()).unwrap();
    let mut t = Session::empty();
    t.open_drawing(imported, "Imported", None);
    assert_eq!(t.execute("csd.netlist", &json!({})).unwrap(), net);
    let kept = |s: &Session| match &s.doc().unwrap().entity(w).unwrap().kind {
        EntityKind::LwPolyline(p) => {
            (p.vertices.iter().map(|v| (v.start_width, v.end_width)).collect::<Vec<_>>(), s.doc().unwrap().entity(w).unwrap().common.layer.clone())
        }
        _ => panic!("no polyline"),
    };
    assert_eq!(kept(&t), kept(&s));
    assert_eq!(t.doc().unwrap().layer("CSD_netdata").unwrap().color, Color::True(Rgb::from_u32(0x3377cc)));
    mv(&mut t, b, [0.0, 70.0]);
    assert!(verts(&t, w).last().unwrap().near(port(&t, b, Side::Left), 1e-9));
    let _ = a;
}

#[test]
fn three_wires_off_one_side_fan_out_in_far_end_order() {
    let mut s = Session::new();
    let src = handle(&s.execute("csd.place", &json!({ "product": "logic-poex", "at": [0, 0] })).unwrap());
    let mut wires = Vec::new();
    // Placed right, left, middle: the slots must still follow the far ends' x.
    for x in [150, -150, 0] {
        let d = handle(&s.execute("csd.place", &json!({ "product": "logic-ar111m", "at": [x, -150] })).unwrap());
        let p = json!({ "from": src.hex(), "fromSide": "bot", "to": d.hex(), "toSide": "top", "protocol": "network" });
        wires.push((x, handle(&s.execute("csd.connect", &p).unwrap())));
    }
    wires.sort_by_key(|(x, _)| *x);
    let starts: Vec<Vec2> = wires.iter().map(|(_, w)| verts(&s, *w)[0]).collect();
    assert!(starts.windows(2).all(|w| w[0].x < w[1].x && w[0].y == w[1].y));
    assert!((starts[1].x - starts[0].x - (starts[2].x - starts[1].x)).abs() < 1e-9);
    // Evenly spread along the 70 wide edge, and the first legs end apart.
    assert!((starts[2].x - starts[0].x - 35.0).abs() < 1e-9);
    let stub_ends: Vec<Vec2> = wires.iter().map(|(_, w)| verts(&s, *w)[2]).collect();
    assert!(stub_ends.windows(2).all(|w| !w[0].near(w[1], 1e-6)));
    for (_, w) in &wires {
        assert_orthogonal(&verts(&s, *w));
    }
}

#[test]
fn wires_sit_on_their_protocol_layer() {
    let (s, _, _, w) = setup();
    let d = s.doc().unwrap();
    let e = d.entity(w).unwrap();
    assert_eq!((e.common.layer.as_str(), e.common.color), ("CSD_netdata", Color::ByLayer));
    let l = d.layer("CSD_netdata").unwrap();
    assert_eq!((l.color, l.linetype.as_str()), (Color::True(Rgb::from_u32(0x3377cc)), "Continuous"));

    let mut d = Drawing::new_metric();
    let (name, scale) = super::wire_layer(&mut d, Protocol::Scene10);
    let l = d.layer(&name).unwrap();
    assert_eq!((l.color, l.linetype.as_str()), (Color::True(Rgb::from_u32(0x6dbd45)), "DASHED2"));
    assert!(d.linetype("DASHED2").is_some() && scale > 0.0 && scale != 1.0);
}

/// Every decoration on the drawing: (handle, wire, role, insert).
fn decos(s: &Session) -> Vec<(Handle, Handle, Deco, cadcraft_doc::Insert)> {
    s.doc()
        .unwrap()
        .model
        .iter()
        .filter_map(|e| {
            let (w, d) = deco_from_xdata(&e.common.xdata)?;
            let EntityKind::Insert(i) = &e.kind else { panic!("decoration is not an insert") };
            Some((e.handle, w, d, i.clone()))
        })
        .collect()
}

fn model(s: &Session) -> Vec<(Handle, EntityKind)> {
    s.doc().unwrap().model.iter().map(|e| (e.handle, e.kind.clone())).collect()
}

#[test]
fn connect_adds_two_dots_and_a_pill_on_the_wire() {
    let (s, _, _, w) = setup();
    let d = decos(&s);
    assert_eq!(d.len(), 3);
    let path = verts(&s, w);
    let at = |role| d.iter().find(|x| x.2 == role).unwrap().3.insert.xy();
    // The dots rest against the cards, 4.5 out along the first and last legs.
    assert!(at(Deco::Start).near(path[0] + Vec2::new(4.5, 0.0), 1e-9));
    assert!(at(Deco::End).near(path[path.len() - 1] - Vec2::new(4.5, 0.0), 1e-9));
    // The pill sits on the longest leg.
    let p = at(Deco::Pill);
    assert!(path.windows(2).any(|w| (w[0].x == w[1].x && p.x == w[0].x || w[0].y == w[1].y && p.y == w[0].y) && w[0].dist(w[1]) >= 40.0));
    let doc = s.doc().unwrap();
    for (h, wire, deco, ins) in &d {
        assert_eq!(*wire, w);
        let e = doc.entity(*h).unwrap();
        assert_eq!(e.common.layer, "CSD_netdata");
        let expect = if *deco == Deco::Pill { "CSDPILL_netdata" } else { "CSDDOT_netdata" };
        assert_eq!(ins.block, expect);
        assert!(doc.block(expect).is_some());
    }
    // Decorations are not devices or wires.
    let mut s = s;
    let net = s.execute("csd.netlist", &json!({})).unwrap();
    assert_eq!((net["devices"].as_array().unwrap().len(), net["wires"].as_array().unwrap().len()), (2, 1));
}

#[test]
fn decorations_follow_a_move_and_one_undo_restores_everything() {
    let (mut s, _, b, _) = setup();
    let before = model(&s);
    let dots = decos(&s);
    mv(&mut s, b, [30.0, 40.0]);
    let moved = decos(&s);
    assert_eq!(moved.len(), 3);
    assert_ne!(
        moved.iter().map(|d| d.3.insert.xy().x.to_bits() ^ d.3.insert.xy().y.to_bits()).collect::<Vec<_>>(),
        dots.iter().map(|d| d.3.insert.xy().x.to_bits() ^ d.3.insert.xy().y.to_bits()).collect::<Vec<_>>()
    );
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(model(&s), before);
}

#[test]
fn erasing_a_wire_removes_its_decorations_and_erasing_one_decoration_restores_it() {
    let (mut s, _, _, w) = setup();
    let first = decos(&s)[0].clone();
    s.execute("erase", &json!({ "handles": [first.0.hex()] })).unwrap();
    let d = decos(&s);
    assert_eq!(d.len(), 3);
    assert!(d.iter().any(|x| x.2 == first.2 && x.3 == first.3));
    s.execute("erase", &json!({ "handles": [w.hex()] })).unwrap();
    assert!(decos(&s).is_empty());
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(decos(&s).len(), 3);
}

#[test]
fn refreshing_twice_changes_nothing() {
    let (mut s, ..) = setup();
    let doc = s.state_mut().unwrap().doc.clone();
    super::refresh(&mut s.state_mut().unwrap().doc);
    assert!(std::sync::Arc::ptr_eq(&doc, &s.state_mut().unwrap().doc));
}

#[test]
fn hostile_decorations_are_erased_or_ignored_and_never_panic() {
    let (mut s, a, _, w) = setup();
    let add = |s: &mut Session, xdata: Vec<RawTag>| {
        let h = s.add_entity(EntityKind::Line(cadcraft_doc::Line { a: Vec2::ZERO.to3(0.0), b: Vec2::new(1.0, 1.0).to3(0.0) })).unwrap();
        s.doc_mut().unwrap().modify_entity(h, |e| e.common.xdata = xdata).unwrap();
        h
    };
    let tag = |code, v: &str| RawTag { code, value: v.into() };
    // Names a device, not a wire; names a handle that is not there; a copy of a real role.
    let stale =
        [add(&mut s, deco_xdata(a, Deco::Pill)), add(&mut s, deco_xdata(Handle(0xFFFF_FFFF), Deco::Start)), add(&mut s, deco_xdata(w, Deco::End))];
    // Garbage that does not parse is not a decoration: it stays as it is.
    let junk = add(&mut s, vec![tag(1001, "CSDX"), tag(1000, "zz"), tag(1000, "")]);
    mv(&mut s, a, [0.0, 1.0]);
    assert!(stale.iter().all(|h| s.doc().unwrap().entity(*h).is_none()));
    assert!(s.doc().unwrap().entity(junk).is_some());
    assert_eq!(decos(&s).len(), 3);
}

#[test]
fn decorations_are_not_duplicated_by_a_reload_and_a_move() {
    let (s, _, b, _) = setup();
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    assert_eq!(decos(&t).len(), 3);
    mv(&mut t, b, [0.0, 70.0]);
    assert_eq!(decos(&t).len(), 3);
}

#[test]
fn decorations_stay_above_later_entities() {
    let (mut s, ..) = setup();
    s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 300] })).unwrap();
    s.execute("line", &json!({ "points": [[0, 0], [5, 5]] })).unwrap();
    let order: Vec<bool> = s.doc().unwrap().model.iter().map(|e| deco_from_xdata(&e.common.xdata).is_some()).collect();
    assert_eq!(order, [vec![false; order.len() - 3], vec![true; 3]].concat());
}

#[test]
fn library_set_list_and_place() {
    let mut s = Session::new();
    let rec = |id: &str, cat: &str| json!({ "id": id, "name": format!("Fixture {id}"), "code": id, "cat": cat, "dmxIn": 1, "img": "data:x" });
    let r = s.execute("csd.library.set", &json!({ "products": [rec("A1", "Wash"), rec("B2", "Spot")] })).unwrap();
    assert_eq!(r["count"], 2);
    s.execute("csd.place", &json!({ "product": "B2", "at": [0, 0] })).unwrap();
    assert!(s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 0] })).is_err());

    let r = s.execute("csd.library.list", &json!({ "query": "b2" })).unwrap();
    assert_eq!(r["products"].as_array().unwrap().len(), 1);
    assert_eq!(r["products"][0]["id"], "B2");
    assert!(r["products"][0].get("img").is_none());
    assert_eq!(r["categories"], json!(["Wash", "Spot"]));
    assert_eq!(s.execute("csd.library.list", &json!({ "category": "Wash" })).unwrap()["products"].as_array().unwrap().len(), 1);

    s.execute("csd.library.set", &json!({ "products": [{ "id": "A1", "name": "Renamed" }] })).unwrap();
    let img = s.csd_library.first().and_then(|p| p.extra.get("img").cloned());
    assert_eq!(img, Some(json!("data:x")));
    assert_eq!(s.csd_library.first().map(|p| p.name.as_str()), Some("Renamed"));
}

#[test]
fn library_save_then_load() {
    let mut s = Session::new();
    let path = std::env::temp_dir().join(format!("csd-lib-{}.json", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let n = s.csd_library.len() - 1;
    assert_eq!(s.execute("csd.library.save", &json!({ "path": path })).unwrap()["count"], n + 1);
    s.execute("csd.library.set", &json!({ "products": [] })).unwrap();
    assert_eq!(s.execute("csd.library.load", &json!({ "path": path })).unwrap()["count"], n);
    assert!(s.execute("csd.library.load", &json!({ "path": "/nonexistent/lib.json" })).is_err());
    let _ = std::fs::remove_file(path);
}
