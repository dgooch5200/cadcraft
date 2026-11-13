use cadcraft_csd::{Deco, Protocol, Side, Slot, WireLink, deco_from_xdata, deco_xdata, port_point};
use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{Drawing, EntityKind, Handle, LwPolyline, RawTag};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use crate::cmd::csd::find_product;
use crate::{Input, Session};

const CONSOLE: &str = "CHAMMQ50";
const NODE: &str = "epix-flex-drive";

fn handle(r: &Value) -> Handle {
    Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap()
}

/// Adds a generic PoE switch to `s`'s library: the sample POE-X feeds MR16s only.
pub(super) fn poe_switch(s: &mut Session) -> &'static str {
    let mut lib = (*s.csd_library).clone();
    let poex = lib.iter().find(|p| p.id == "logic-poex").unwrap().clone();
    lib.push(cadcraft_csd::Product { id: "poe-switch".into(), name: "PoE Switch".into(), code: "POE-SW".into(), ..poex });
    s.csd_library = std::sync::Arc::new(lib);
    "poe-switch"
}

/// A session routing wires at right angles (the tests below assume it; the default is smooth curves).
fn ortho_session() -> Session {
    let mut s = Session::new();
    s.execute("ortho", &json!({ "on": true })).unwrap();
    s
}

/// A console at the origin and a node at (200, 0), joined by a wire from the console's right to the node's left.
fn setup() -> (Session, Handle, Handle, Handle) {
    let mut s = ortho_session();
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
    // Each rounded bend is reported as its sharp corner.
    let mut out: Vec<Vec2> = Vec::new();
    let mut skip = false;
    for (i, v) in p.vertices.iter().enumerate() {
        if std::mem::take(&mut skip) {
            continue;
        }
        match (v.bulge != 0.0, out.last(), p.vertices.get(i + 1)) {
            (true, Some(prev), Some(q)) => {
                out.push(if prev.x == v.p.x { Vec2::new(v.p.x, q.p.y) } else { Vec2::new(q.p.x, v.p.y) });
                skip = true;
            }
            _ => out.push(v.p),
        }
    }
    out
}

#[test]
fn wire_bends_are_rounded_arcs_on_the_route() {
    let (s, a, _, w) = setup();
    let Some(EntityKind::LwPolyline(p)) = s.doc().unwrap().entity(w).map(|e| e.kind.clone()) else { panic!("no polyline") };
    let arcs: Vec<_> = p.vertices.iter().filter(|v| v.bulge != 0.0).collect();
    assert!(!arcs.is_empty());
    assert!(arcs.iter().all(|v| (v.bulge.abs() - std::f64::consts::FRAC_PI_8.tan()).abs() < 1e-9));
    assert!(p.vertices.first().unwrap().p.near(port(&s, a, Side::Right), 1e-9));
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
        WireLink::new(ghost, Side::Top, ghost, Side::Left, cadcraft_csd::Protocol::Dmx).to_xdata(),
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
    t.execute("ortho", &json!({ "on": true })).unwrap();
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
    let mut s = ortho_session();
    let sw = poe_switch(&mut s);
    let src = handle(&s.execute("csd.place", &json!({ "product": sw, "at": [0, 0] })).unwrap());
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
    // The dots sit on the ports.
    assert!(at(Deco::Start).near(path[0], 1e-9));
    assert!(at(Deco::End).near(path[path.len() - 1], 1e-9));
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
    super::refresh(&mut s.state_mut().unwrap().doc, true);
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
fn library_csv_round_trips_and_reads_old_headers() {
    let mut s = Session::new();
    let csv = s.execute("csd.library.export", &json!({})).unwrap();
    let before = csv["count"].clone();
    assert!(before.as_u64() > Some(1));
    let r = s.execute("csd.library.import", &json!({ "text": csv["csv"] })).unwrap();
    assert_eq!(r["count"], before);
    assert_eq!(s.execute("csd.library.export", &json!({})).unwrap()["csv"], csv["csv"]);

    let text = "\u{feff}Product Name,Model ID,DMX Ports,Network Ports,Notes\r\n\"Wash, big\",My Wash,2,1,\"say \"\"hi\"\"\"\r\n,No Name,1,1,\r\nSpot,My Wash,9,9,dup\r\n";
    let r = s.execute("csd.library.import", &json!({ "text": text })).unwrap();
    assert_eq!(r["count"], 1);
    let p = &s.csd_library[0];
    assert_eq!((p.id.as_str(), p.code.as_str(), p.name.as_str(), p.notes.as_str()), ("My_Wash", "My_Wash", "Wash, big", "say \"hi\""));
    assert_eq!((p.counts[3].1, p.counts[0].1), (2, 1));
}

#[test]
fn library_csv_import_rejects_hostile_input() {
    let mut s = Session::new();
    let import = |s: &mut Session, t: String| s.execute("csd.library.import", &json!({ "text": t }));
    assert!(import(&mut s, String::new()).is_err());
    assert!(import(&mut s, "a,b\n1,2\n".into()).is_err());
    assert!(import(&mut s, "Model ID,Product Name\n\"open,x\n".into()).is_err());
    let many = format!("Model ID,Product Name\n{}", "a,b\n".repeat(10_001));
    assert!(import(&mut s, many).is_err());
    let wide = format!("Model ID,Product Name{}\nx,y{}\n", ",c".repeat(5000), ",1".repeat(5000));
    assert_eq!(import(&mut s, wide).unwrap()["count"], 1);
    assert!(s.execute("csd.library.import", &json!({})).is_err());
}

#[test]
fn library_list_offers_only_general_and_own_series_accessories() {
    let mut s = Session::new();
    let rec = |id: &str, cat: &str| json!({ "id": id, "name": id, "cat": cat });
    let products = [
        rec("par", "Ovation — PAR"),
        rec("rack", "GeNetix"),
        rec("general", "General Accessory"),
        rec("ovation", "Ovation Series Accessories"),
        rec("genetix", "GeNetix Accessories"),
    ];
    s.execute("csd.library.set", &json!({ "products": products })).unwrap();
    let ids = |s: &mut Session, f: Value| -> Vec<String> {
        let r = s.execute("csd.library.list", &json!({ "accessoriesFor": f })).unwrap();
        r["products"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(ids(&mut s, json!(["par"])), ["general", "ovation"]);
    assert_eq!(ids(&mut s, json!(["par", "rack"])), ["general", "ovation", "genetix"]);
    assert_eq!(ids(&mut s, json!([])), ["general", "ovation", "genetix"]);
    assert_eq!(ids(&mut s, json!(["nope", 7])), ["general", "ovation", "genetix"]);
}

#[test]
fn library_save_then_load() {
    let mut s = Session::new();
    let path = std::env::temp_dir().join(format!("csd-lib-save-{}.json", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let n = s.csd_library.len() - 1;
    assert_eq!(s.execute("csd.library.save", &json!({ "path": path })).unwrap()["count"], n + 1);
    s.execute("csd.library.set", &json!({ "products": [] })).unwrap();
    assert_eq!(s.execute("csd.library.load", &json!({ "path": path })).unwrap()["count"], n);
    assert!(s.execute("csd.library.load", &json!({ "path": "/nonexistent/lib.json" })).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_library_loads_before_any_drawing_is_open() {
    let mut s = Session::empty();
    let path = std::env::temp_dir().join(format!("csd-lib-early-{}.json", std::process::id()));
    std::fs::write(&path, r#"[{"id": "a", "name": "A"}]"#).unwrap();
    let loaded = s.execute("csd.library.load", &json!({ "path": path.to_string_lossy() }));
    let _ = std::fs::remove_file(&path);
    assert_eq!(loaded.unwrap(), json!({ "count": 1 }));
    assert_eq!(s.execute("csd.library.list", &json!({})).unwrap()["products"].as_array().map(Vec::len), Some(1));
}

fn device_count(s: &mut Session) -> usize {
    s.execute("csd.netlist", &json!({})).unwrap()["devices"].as_array().unwrap().len()
}

#[test]
fn armed_product_is_placed_at_each_point_until_enter() {
    let mut s = Session::new();
    s.execute("csd.arm", &json!({ "product": NODE })).unwrap();
    s.start("csd.place").unwrap();
    s.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    s.input(Input::Point(Vec2::new(200.0, 0.0))).unwrap();
    assert!(s.running.is_some());
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert_eq!(device_count(&mut s), 2);
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(device_count(&mut s), 0, "the run is one undo step");
}

#[test]
fn escape_stops_placing_and_keeps_what_is_placed() {
    let mut s = Session::new();
    s.execute("csd.arm", &json!({ "product": NODE })).unwrap();
    s.start("csd.place").unwrap();
    s.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    s.cancel();
    assert!(s.running.is_none());
    assert_eq!(device_count(&mut s), 1);
}

#[test]
fn arming_needs_a_known_product() {
    let mut s = Session::new();
    assert!(s.execute("csd.arm", &json!({ "product": "no-such-product" })).is_err());
}

#[test]
fn placing_with_nothing_armed_asks_for_the_product() {
    let mut s = Session::new();
    s.start("csd.place").unwrap();
    s.input(Input::Text("no-such-product".into())).unwrap();
    assert!(s.running.is_some(), "still asking");
    s.input(Input::Text(NODE.into())).unwrap();
    s.input(Input::Point(Vec2::new(0.0, 0.0))).unwrap();
    s.input(Input::Enter).unwrap();
    assert_eq!(device_count(&mut s), 1);
}

/// A console at the origin and a node at (200, 60), both unwired, with `csd.connect` running.
fn connecting() -> Session {
    let mut s = ortho_session();
    s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 0] })).unwrap();
    s.execute("csd.place", &json!({ "product": NODE, "at": [200, 60] })).unwrap();
    s.start("csd.connect").unwrap();
    s
}

fn wire_links(s: &mut Session) -> Vec<Value> {
    s.execute("csd.netlist", &json!({})).unwrap()["wires"].as_array().unwrap().clone()
}

#[test]
fn clicking_two_ports_wires_them_with_the_chosen_protocol() {
    let mut s = connecting();
    s.input(Input::Keyword("netdata".into())).unwrap();
    s.input(Input::Point(Vec2::new(30.0, 2.0))).unwrap();
    s.input(Input::Point(Vec2::new(172.0, 58.0))).unwrap();
    assert!(s.running.is_some(), "chaining: the command keeps running");
    let w = wire_links(&mut s);
    assert_eq!(w.len(), 1);
    assert_eq!((w[0]["fromSide"].as_str(), w[0]["toSide"].as_str(), w[0]["protocol"].as_str()), (Some("rgt"), Some("lft"), Some("netdata")));
    s.input(Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert_eq!(s.csd_protocol, Protocol::NetData, "the next run starts with it");
}

#[test]
fn a_refused_connection_keeps_the_command_running_and_adds_no_wire() {
    let mut s = connecting();
    s.input(Input::Keyword("midi".into())).unwrap();
    s.input(Input::Point(Vec2::new(30.0, 2.0))).unwrap();
    s.input(Input::Point(Vec2::new(172.0, 58.0))).unwrap();
    assert!(s.running.is_some());
    assert!(wire_links(&mut s).is_empty());
    assert!(s.log.last().is_some_and(|m| m.contains("MIDI")), "{:?}", s.log.last());
}

#[test]
fn clicking_empty_space_adds_nothing() {
    let mut s = connecting();
    s.input(Input::Point(Vec2::new(500.0, 500.0))).unwrap();
    s.input(Input::Point(Vec2::new(1000.0, 500.0))).unwrap();
    assert!(s.running.is_some());
    assert!(wire_links(&mut s).is_empty());
}

/// A drawing whose console card has a stale extra entity, round-tripped through DXF.
fn stale_copy() -> (Drawing, String) {
    let (s, ..) = setup();
    let mut d = s.doc().unwrap().clone();
    let name = cadcraft_csd::block_name(CONSOLE);
    let mut block = (**d.blocks.get(&name).unwrap()).clone();
    let h = d.new_handle();
    block.entities.push(cadcraft_doc::Entity::new(h, EntityKind::Point(cadcraft_doc::Point { p: Default::default(), angle: 0.0 })));
    d.blocks.insert(name.clone(), std::sync::Arc::new(block));
    let text = cadcraft_io::write_dxf(&d);
    (cadcraft_io::read_dxf(text.as_bytes()).unwrap(), name)
}

#[test]
fn opening_a_drawing_brings_its_cards_up_to_date() {
    let (d, name) = stale_copy();
    let stale = d.block(&name).unwrap().entities.len();
    let mut s = Session::new();
    s.open_drawing(d, "Old", None);
    assert_eq!(s.doc().unwrap().block(&name).unwrap().entities.len() + 1, stale);
    assert!(!s.state().unwrap().is_dirty());
    assert!(s.state().unwrap().undo.is_empty());
}

#[test]
fn opening_an_up_to_date_drawing_changes_nothing() {
    let (s, ..) = setup();
    let imported = cadcraft_io::read_dxf(cadcraft_io::write_dxf(s.doc().unwrap()).as_bytes()).unwrap();
    let before = imported.blocks.clone();
    let mut t = Session::new();
    t.open_drawing(imported, "Same", None);
    assert!(t.doc().unwrap().blocks.iter().all(|(n, b)| std::sync::Arc::ptr_eq(b, &before[n])));
}

#[test]
fn a_library_colour_change_recolours_the_placed_cards() {
    let (mut s, ..) = setup();
    let border = |s: &Session| {
        let b = s.doc().unwrap().block(&cadcraft_csd::block_name(CONSOLE)).unwrap().clone();
        b.entities.iter().find(|e| matches!(e.kind, EntityKind::LwPolyline(_))).unwrap().common.color
    };
    let old = border(&s);
    let mut rec = cadcraft_csd::product_record(s.csd_library.iter().find(|p| p.id == CONSOLE).unwrap());
    rec.insert("color".into(), json!("#123456"));
    s.execute("csd.library.set", &json!({ "products": [rec] })).unwrap();
    assert_ne!(border(&s), old);
    assert_eq!(border(&s), Color::True(Rgb::from_u32(0x123456)));
}

#[test]
fn wires_saved_with_an_old_route_are_rerouted_on_open() {
    let (mut s, _, b, w) = setup();
    let want = verts(&s, w);
    let _ = s.doc_mut().unwrap().modify_entity(w, |e| {
        if let EntityKind::LwPolyline(p) = &mut e.kind {
            p.vertices.truncate(2);
        }
    });
    let imported = cadcraft_io::read_dxf(cadcraft_io::write_dxf(s.doc().unwrap()).as_bytes()).unwrap();
    let mut t = Session::empty();
    t.execute("ortho", &json!({ "on": true })).unwrap();
    t.open_drawing(imported, "Old", None);
    assert_eq!(verts(&t, w).len(), want.len());
    assert!(verts(&t, w).last().unwrap().near(port(&t, b, Side::Left), 1e-9));
    assert!(!t.state().unwrap().is_dirty());
}

fn insert(s: &Session, h: Handle) -> cadcraft_doc::Insert {
    let Some(EntityKind::Insert(i)) = s.doc().unwrap().entity(h).map(|e| e.kind.clone()) else { panic!("no insert") };
    i
}

#[test]
fn size_resizes_every_listed_device_and_the_wire_follows() {
    let (mut s, a, b, w) = setup();
    let before = verts(&s, w);
    let r = s.execute("csd.size", &json!({ "handles": [a.hex(), b.hex()], "w": 140 })).unwrap();
    assert_eq!(r["sized"], 2);
    assert_eq!(insert(&s, a).scale.x, 2.0);
    assert_eq!(insert(&s, b).scale.y, 1.0);
    assert_ne!(before, verts(&s, w));
    assert!(verts(&s, w)[0].near(port(&s, a, Side::Right), 1e-9));
    // null resets the axis; hostile values are refused and change nothing.
    s.execute("csd.size", &json!({ "handles": [a.hex()], "w": null })).unwrap();
    assert_eq!(insert(&s, a).scale.x, 1.0);
    assert!(s.execute("csd.size", &json!({ "handles": [a.hex()], "h": "big" })).is_err());
    s.execute("csd.size", &json!({ "handles": [a.hex()], "h": 1e300 })).unwrap();
    assert_eq!(insert(&s, a).scale.y, 1000.0 / cadcraft_csd::CARD_H);
}

#[test]
fn replace_swaps_the_product_on_the_selection_and_skips_the_same_one() {
    let (mut s, a, b, _) = setup();
    s.set_selection(vec![a, b]);
    let r = s.execute("csd.replace", &json!({ "product": NODE })).unwrap();
    assert_eq!(r["replaced"], 1);
    let i = insert(&s, a);
    assert_eq!(i.block, cadcraft_csd::block_name(NODE));
    assert!(i.attribs.iter().any(|t| t.tag == "PRODUCT" && t.text.value == NODE));
    assert!(s.execute("csd.replace", &json!({ "product": "nope" })).is_err());
}

#[test]
fn csd_protocol_sets_the_wire_type_and_rejects_unknown_ids() {
    let mut s = Session::new();
    assert_eq!(s.execute("csd.protocol", &json!({ "protocol": "dmx" })).unwrap()["protocol"], "dmx");
    assert_eq!(s.csd_protocol, Protocol::Dmx);
    assert!(s.execute("csd.protocol", &json!({ "protocol": "nope" })).is_err());
    assert!(s.execute("csd.protocol", &json!({})).is_err());
    assert_eq!(s.csd_protocol, Protocol::Dmx);
}

#[test]
fn product_lookup_ignores_case_and_punctuation() {
    let lib = cadcraft_csd::sample_products();
    let id = |q: &str| find_product(&lib, q).map(|p| p.id.as_str());
    assert_eq!(id("nothing-like-it"), None);
    assert_eq!(id("LOGIC-AR111M"), Some("logic-ar111m"));
    assert_eq!(id("logicar111m"), Some("logic-ar111m"));
    assert_eq!(id("b565fc"), Some("b565fc"));
}

fn polyline(s: &Session, h: Handle) -> Vec<(Vec2, f64)> {
    let Some(EntityKind::LwPolyline(p)) = s.doc().unwrap().entity(h).map(|e| e.kind.clone()) else { panic!("no polyline") };
    p.vertices.iter().map(|v| (v.p, v.bulge)).collect()
}

#[test]
fn smooth_wires_by_default_leave_the_ports_and_follow_the_ortho_toggle() {
    let mut s = Session::new();
    let a = handle(&s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 0] })).unwrap());
    let b = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [200, 60] })).unwrap());
    let w = connect(&mut s, a, b).unwrap();
    let smooth = polyline(&s, w);
    let (p, q) = (port_point(Vec2::ZERO, 0.0, Vec2::new(1.0, 1.0), Side::Right, Slot::SOLO), smooth[smooth.len() - 1].0);
    assert_eq!(smooth[0].0, p);
    assert!(smooth.len() > 10 && smooth.iter().all(|v| v.1 == 0.0));
    assert!(smooth.windows(2).any(|v| v[0].0.x != v[1].0.x && v[0].0.y != v[1].0.y));
    // Leaves the right port along +x and arrives at the left port along +x.
    let (u, v) = (smooth[1].0 - smooth[0].0, q - smooth[smooth.len() - 2].0);
    assert!(u.x > 0.0 && u.y.abs() < 0.2 * u.x && v.x > 0.0 && v.y.abs() < 0.2 * v.x);
    let pill = decos(&s).iter().filter(|d| d.2 == Deco::Pill).count();
    assert_eq!(pill, 1);

    s.execute("ortho", &json!({ "on": true })).unwrap();
    let ortho = polyline(&s, w);
    assert!(ortho.len() < smooth.len() && ortho.iter().any(|v| v.1 != 0.0));
    let doc = s.state_mut().unwrap().doc.clone();
    s.execute("ortho", &json!({ "on": true })).unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &s.state_mut().unwrap().doc), "same style rewrites nothing");

    s.execute("ortho", &json!({ "on": false })).unwrap();
    assert_eq!(polyline(&s, w), smooth);
}

#[test]
fn device_info_reports_ports_connections_and_size() {
    let (mut s, a, b, w) = setup();
    s.execute("csd.size", &json!({ "handles": [a.hex()], "w": 140 })).unwrap();
    let r = s.execute("csd.device.info", &json!({ "handle": a.hex() })).unwrap();
    assert_eq!(r["product"]["id"], CONSOLE);
    assert_eq!(r["known"], true);
    assert_eq!(r["sameProduct"], 1);
    assert_eq!(r["size"][0], 140.0);
    assert_eq!(r["libSize"][0], cadcraft_csd::CARD_W);
    let net = r["ports"].as_array().unwrap().iter().find(|p| p["protocol"] == "netdata").unwrap();
    assert_eq!((net["used"].as_u64(), net["capacity"].as_u64(), net["pooled"].as_bool()), (Some(1), Some(2), Some(true)));
    assert!(r["ports"].as_array().unwrap().iter().all(|p| p["protocol"] != "midi"));
    assert_eq!((r["connections"][0]["wire"].as_str(), r["connections"][0]["dir"].as_str()), (Some(w.hex().as_str()), Some("out")));
    assert_eq!(r["connections"][0]["other"]["handle"], b.hex());
    // The selection stands in for the handle; the far end sees the wire as incoming.
    s.set_selection(vec![b]);
    assert_eq!(s.execute("csd.device.info", &json!({})).unwrap()["connections"][0]["dir"], "in");
    s.set_selection(vec![a, b]);
    assert!(s.execute("csd.device.info", &json!({})).is_err());
}

#[test]
fn device_set_writes_label_and_notes_in_one_undo_step_and_survives_a_library_refresh() {
    let (mut s, a, _, _) = setup();
    s.execute("csd.device.set", &json!({ "handle": a.hex(), "label": "Main desk", "notes": "FOH" })).unwrap();
    let r = s.execute("csd.device.info", &json!({ "handle": a.hex() })).unwrap();
    assert_eq!((r["label"].as_str(), r["notes"].as_str()), (Some("Main desk"), Some("FOH")));
    let attr = |s: &Session, t: &str| insert(s, a).attribs.iter().find(|x| x.tag == t).map(|x| x.text.value.clone());
    assert_eq!(attr(&s, "LABEL").as_deref(), Some("Main desk"));
    let products = s.csd_library.to_vec();
    s.execute("csd.library.set", &json!({ "products": products.iter().map(cadcraft_csd::product_record).collect::<Vec<_>>() })).unwrap();
    assert_eq!(attr(&s, "NOTES").as_deref(), Some("FOH"));
    // An empty label falls back to the product code.
    s.execute("csd.device.set", &json!({ "handle": a.hex(), "label": " " })).unwrap();
    assert_eq!(attr(&s, "NAME").as_deref(), Some("MQ50"));
    assert!(s.execute("csd.device.set", &json!({ "handle": a.hex(), "notes": 3 })).is_err());
    s.undo().unwrap();
    assert_eq!(attr(&s, "NAME").as_deref(), Some("Main desk"));
}

#[test]
fn device_set_adds_notes_to_a_device_that_lacks_them() {
    let (mut s, a, _, _) = setup();
    s.doc_mut()
        .unwrap()
        .modify_entity(a, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.attribs.retain(|x| x.tag != "NOTES");
            }
        })
        .unwrap();
    s.execute("csd.device.set", &json!({ "handle": a.hex(), "notes": "late" })).unwrap();
    let i = insert(&s, a);
    assert!(i.attribs.iter().any(|x| x.tag == "NOTES" && x.text.value == "late" && x.invisible));
}

#[test]
fn device_delete_removes_wires_and_their_decorations_in_one_undo_step() {
    let (mut s, a, b, w) = setup();
    let count = |s: &Session| s.doc().unwrap().space(&s.space()).unwrap().iter().count();
    let before = count(&s);
    assert!(before > 3, "wire decorations exist");
    let r = s.execute("csd.device.delete", &json!({ "handles": [a.hex()] })).unwrap();
    assert_eq!((r["deleted"].as_u64(), r["wires"].as_u64()), (Some(1), Some(1)));
    assert!(s.doc().unwrap().entity(w).is_none());
    assert_eq!(count(&s), 1, "only the other device is left");
    assert!(s.doc().unwrap().entity(b).is_some());
    s.undo().unwrap();
    assert_eq!(count(&s), before);
}

#[test]
fn replace_all_of_product_reaches_devices_outside_the_selection() {
    let (mut s, a, b, _) = setup();
    let c = handle(&s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 200] })).unwrap());
    s.set_selection(vec![a]);
    assert_eq!(s.execute("csd.replace", &json!({ "product": NODE })).unwrap()["replaced"], 1);
    s.set_selection(vec![b]);
    let r = s.execute("csd.replace", &json!({ "product": CONSOLE, "allOfProduct": true })).unwrap();
    assert_eq!(r["replaced"], 2);
    assert_eq!(insert(&s, a).block, cadcraft_csd::block_name(CONSOLE));
    assert_eq!(insert(&s, c).block, cadcraft_csd::block_name(CONSOLE));
}

fn product(id: &str, code: &str) -> cadcraft_csd::Product {
    cadcraft_csd::Product { id: id.into(), name: id.into(), code: code.into(), ..Default::default() }
}

#[test]
fn product_lookup_falls_back_to_a_unique_code() {
    let lib = [product("OVATIONB565FC", "B-565FC"), product("OVATIONB565", "B-565"), product("OTHER1", "B 565fc")];
    assert_eq!(find_product(&lib[..2], "b565fc").map(|p| p.id.as_str()), Some("OVATIONB565FC"));
    // Two products share the code, so there is no match; an exact id still wins.
    assert!(find_product(&lib, "b565fc").is_none());
    assert_eq!(find_product(&lib, "OTHER1").map(|p| p.id.as_str()), Some("OTHER1"));
}

/// A card whose PRODUCT attribute names `id`, whatever the library holds.
fn card_of(s: &mut Session, id: &str, at: [f64; 2]) -> Handle {
    let h = handle(&s.execute("csd.place", &json!({ "product": CONSOLE, "at": at })).unwrap());
    s.doc_mut()
        .unwrap()
        .modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.attribs.iter_mut().filter(|a| a.tag == "PRODUCT").for_each(|a| a.text.value = id.into());
            }
        })
        .unwrap();
    h
}

#[test]
fn a_card_not_in_the_library_is_a_device_that_can_be_replaced_or_deleted() {
    let mut s = ortho_session();
    let ghost = card_of(&mut s, "OVCYC1FCV2", [0.0, 0.0]);
    let node = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [200, 60] })).unwrap());
    let info = s.execute("csd.device.info", &json!({ "handle": ghost.hex() })).unwrap();
    assert_eq!(info["known"], false);
    assert_eq!(info["product"]["id"], "OVCYC1FCV2");
    assert_eq!(info["product"]["name"], "OVCYC1FCV2 (not in library)");
    assert_eq!(info["product"]["code"], "OVCYC1FCV2");
    assert_eq!(info["ports"], json!([]));
    let netlist = s.execute("csd.netlist", &json!({})).unwrap();
    assert_eq!(netlist["devices"].as_array().unwrap().len(), 2);
    let err = connect(&mut s, ghost, node).unwrap_err().to_string();
    assert!(err.contains("not in the library"), "{err}");
    s.execute("csd.size", &json!({ "handles": [ghost.hex()], "w": 100 })).unwrap();
    s.execute("csd.device.set", &json!({ "handle": ghost.hex(), "notes": "n" })).unwrap();
    s.execute("csd.replace", &json!({ "product": CONSOLE, "handles": [ghost.hex()] })).unwrap();
    let info = s.execute("csd.device.info", &json!({ "handle": ghost.hex() })).unwrap();
    assert_eq!((info["known"].as_bool(), info["product"]["id"].as_str(), info["notes"].as_str()), (Some(true), Some(CONSOLE), Some("n")));
    assert!(connect(&mut s, ghost, node).is_ok());
    let gone = card_of(&mut s, "NOPE", [0.0, 300.0]);
    assert_eq!(s.execute("csd.device.delete", &json!({ "handles": [gone.hex()] })).unwrap()["deleted"], 1);
}

fn place_at(s: &mut Session, product: &str, x: f64, y: f64) -> Handle {
    handle(&s.execute("csd.place", &json!({ "product": product, "at": [x, y] })).unwrap())
}

fn at(s: &Session, h: Handle) -> (f64, f64) {
    let Some(EntityKind::Insert(i)) = s.doc().unwrap().entity(h).map(|e| &e.kind) else { panic!("no insert") };
    (i.insert.x, i.insert.y)
}

#[test]
fn select_product_selects_every_device_of_that_product() {
    let mut s = Session::new();
    let a = place_at(&mut s, CONSOLE, 0.0, 0.0);
    let b = place_at(&mut s, CONSOLE, 100.0, 0.0);
    place_at(&mut s, NODE, 200.0, 0.0);
    let r = s.execute("csd.select.product", &json!({ "device": a.hex() })).unwrap();
    assert_eq!(r["count"], 2);
    let mut sel = s.selection();
    sel.sort();
    assert_eq!(sel, vec![a, b]);
    assert!(s.execute("csd.select.product", &json!({ "device": "FFFF" })).is_err());
}

#[test]
fn align_lines_up_card_edges_and_wires_follow() {
    let mut s = ortho_session();
    let a = place_at(&mut s, CONSOLE, 0.0, 0.0);
    let b = place_at(&mut s, NODE, 200.0, 60.0);
    let w = connect(&mut s, a, b).unwrap();
    let before = verts(&s, w);
    s.execute("csd.size", &json!({ "handles": [b.hex()], "w": 140 })).unwrap();
    s.execute("csd.align", &json!({ "mode": "left", "handles": [a.hex(), b.hex()] })).unwrap();
    // Left edges meet at a's -35, so the 140-wide card's centre lands at 35.
    assert_eq!(at(&s, a), (0.0, 0.0));
    assert_eq!(at(&s, b), (35.0, 60.0));
    assert_ne!(verts(&s, w), before);
    s.set_selection(vec![a, b]);
    s.execute("csd.align", &json!({ "mode": "top" })).unwrap();
    assert_eq!(at(&s, a).1, at(&s, b).1);
    assert!(s.execute("csd.align", &json!({ "mode": "middle" })).is_err());
    assert!(s.execute("csd.align", &json!({ "mode": "left", "handles": [a.hex()] })).is_err());
}

#[test]
fn distribute_equalises_gaps_and_keeps_the_ends() {
    let mut s = Session::new();
    let a = place_at(&mut s, CONSOLE, 0.0, 0.0);
    let b = place_at(&mut s, CONSOLE, 50.0, 0.0);
    let c = place_at(&mut s, CONSOLE, 400.0, 0.0);
    let all = json!([a.hex(), b.hex(), c.hex()]);
    s.execute("csd.distribute", &json!({ "axis": "x", "handles": all })).unwrap();
    assert_eq!((at(&s, a).0, at(&s, b).0, at(&s, c).0), (0.0, 200.0, 400.0));
    assert!(s.execute("csd.distribute", &json!({ "axis": "x", "handles": [a.hex(), b.hex()] })).is_err());
}

#[test]
fn device_at_finds_the_card_under_a_point() {
    let mut s = Session::new();
    let a = place_at(&mut s, CONSOLE, 0.0, 0.0);
    let hit = |s: &mut Session, x: f64, y: f64| s.execute("csd.device.at", &json!({ "at": [x, y] })).unwrap()["handle"].clone();
    assert_eq!(hit(&mut s, 30.0, 30.0), json!(a.hex()));
    assert_eq!(hit(&mut s, 90.0, 0.0), json!(null));
}

fn link_of(s: &Session, w: Handle) -> WireLink {
    WireLink::from_xdata(&s.doc().unwrap().entity(w).unwrap().common.xdata).unwrap()
}

fn wire_cmd(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn passes_through(v: &[Vec2], at: Vec2) -> bool {
    v.iter().any(|p| p.near(at, 1e-6))
}

fn xy(v: Vec2) -> [f64; 2] {
    [v.x, v.y]
}

#[test]
fn waypoints_add_move_remove_and_straighten_in_ortho() {
    let (mut s, _, _, w) = setup();
    let straight = verts(&s, w);
    let r = wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    assert_eq!(r["index"], 0);
    assert_eq!(link_of(&s, w).points, [Vec2::new(100.0, 150.0)]);
    assert!(passes_through(&verts(&s, w), Vec2::new(100.0, 150.0)));
    assert_orthogonal(&verts(&s, w));
    assert_eq!(wire_cmd(&mut s, "csd.wire.bent", json!({}))["count"], 1);

    // A second point near the far end lands after the first.
    assert_eq!(wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [180, 150] }))["index"], 1);
    wire_cmd(&mut s, "csd.wire.point.move", json!({ "wire": w.hex(), "index": 0, "to": [90, -80] }));
    assert!(passes_through(&verts(&s, w), Vec2::new(90.0, -80.0)));
    wire_cmd(&mut s, "csd.wire.point.remove", json!({ "wire": w.hex(), "at": [181, 149] }));
    assert_eq!(link_of(&s, w).points, [Vec2::new(90.0, -80.0)]);
    wire_cmd(&mut s, "csd.wire.point.remove", json!({ "wire": w.hex(), "index": 0 }));
    assert_eq!(verts(&s, w), straight);

    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    assert_eq!(wire_cmd(&mut s, "csd.wire.straighten", json!({}))["count"], 1);
    assert_eq!(verts(&s, w), straight);
    assert_eq!(wire_cmd(&mut s, "csd.wire.bent", json!({}))["count"], 0);
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(link_of(&s, w).points.len(), 1, "straighten is one undo step");
}

#[test]
fn waypoint_errors_change_nothing() {
    let (mut s, _, _, w) = setup();
    for (id, p) in [
        ("csd.wire.point.move", json!({ "wire": w.hex(), "index": 0, "to": [1, 1] })),
        ("csd.wire.point.remove", json!({ "wire": w.hex(), "index": 3 })),
        ("csd.wire.point.remove", json!({ "wire": w.hex(), "at": [5, 5] })),
        ("csd.wire.point.add", json!({ "wire": "FFFF", "at": [5, 5] })),
        ("csd.wire.label", json!({ "wire": w.hex(), "mode": "segment" })),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id}");
    }
    assert!(link_of(&s, w).points.is_empty());
}

#[test]
fn smooth_wires_pass_through_their_waypoints() {
    let mut s = Session::new();
    let a = handle(&s.execute("csd.place", &json!({ "product": CONSOLE, "at": [0, 0] })).unwrap());
    let b = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [200, 60] })).unwrap());
    let w = connect(&mut s, a, b).unwrap();
    let plain = polyline(&s, w);
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    let bent = polyline(&s, w);
    assert!(passes_through(&bent.iter().map(|v| v.0).collect::<Vec<_>>(), Vec2::new(100.0, 150.0)));
    assert!(bent.len() > plain.len() && bent.iter().all(|v| v.1 == 0.0));
    assert_eq!(bent[0], plain[0]);
    assert_eq!(bent.last(), plain.last());
}

#[test]
fn waypoints_follow_when_both_ends_move_together_only() {
    let (mut s, a, b, w) = setup();
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    mv(&mut s, b, [0.0, 30.0]);
    assert_eq!(link_of(&s, w).points, [Vec2::new(100.0, 150.0)], "one end moving leaves the point");
    s.execute("move", &json!({ "handles": [a.hex(), b.hex()], "delta": [10, 20] })).unwrap();
    assert_eq!(link_of(&s, w).points, [Vec2::new(110.0, 170.0)]);
    assert!(passes_through(&verts(&s, w), Vec2::new(110.0, 170.0)));
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(link_of(&s, w).points, [Vec2::new(100.0, 150.0)]);
}

#[test]
fn waypoints_snap_to_the_grid_when_snap_is_on() {
    let (mut s, _, _, w) = setup();
    s.settings.snapmode = true;
    s.settings.snapunit = Vec2::new(10.0, 10.0);
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [103, 148] }));
    assert_eq!(link_of(&s, w).points, [Vec2::new(100.0, 150.0)]);
}

#[test]
fn hit_prefers_a_waypoint_over_the_line() {
    let (mut s, _, _, w) = setup();
    let miss = wire_cmd(&mut s, "csd.wire.hit", json!({ "at": [100, 300] }));
    assert_eq!(miss, json!({ "wire": null, "point": null, "points": 0 }));
    let start = verts(&s, w)[0];
    let on_line = wire_cmd(&mut s, "csd.wire.hit", json!({ "at": [start.x + 3.0, start.y + 2.0] }));
    assert_eq!((on_line["wire"].as_str(), on_line["point"].is_null()), (Some(w.hex().as_str()), true));
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    let hit = wire_cmd(&mut s, "csd.wire.hit", json!({ "at": [104, 146] }));
    assert_eq!((hit["wire"].as_str(), hit["point"].as_u64(), hit["points"].as_u64()), (Some(w.hex().as_str()), Some(0), Some(1)));
    assert!(wire_cmd(&mut s, "csd.wire.hit", json!({ "at": [104, 146], "tolerance": 2 }))["point"].is_null());
}

#[test]
fn label_position_rename_and_dxf_round_trip() {
    let (mut s, _, _, w) = setup();
    let pill = |s: &Session| decos(s).into_iter().find(|d| d.2 == Deco::Pill).unwrap().3.insert.xy();
    let default = pill(&s);
    let start = verts(&s, w)[0];
    let at_start = wire_cmd(&mut s, "csd.wire.label", json!({ "wire": w.hex(), "at": xy(start) }));
    assert_eq!(at_start["labelT"], 0.0);
    assert!(pill(&s).near(start, 1e-6));
    let seg = wire_cmd(&mut s, "csd.wire.label", json!({ "wire": w.hex(), "mode": "segment", "at": xy(start) }));
    let t = seg["labelT"].as_f64().unwrap();
    assert!(t > 0.0 && t < 0.5);
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    wire_cmd(&mut s, "csd.wire.set", json!({ "wire": w.hex(), "name": "  Stage left  " }));
    let saved = link_of(&s, w);
    assert_eq!((saved.name.as_deref(), saved.label_t, saved.points.len()), (Some("Stage left"), Some(t), 1));

    let imported = cadcraft_io::read_dxf(cadcraft_io::write_dxf(s.doc().unwrap()).as_bytes()).unwrap();
    let mut u = Session::empty();
    u.execute("ortho", &json!({ "on": true })).unwrap();
    u.open_drawing(imported, "Imported", None);
    assert_eq!(link_of(&u, w), saved);
    assert_eq!(verts(&u, w), verts(&s, w));

    wire_cmd(&mut s, "csd.wire.label", json!({ "wire": w.hex() }));
    wire_cmd(&mut s, "csd.wire.set", json!({ "wire": w.hex(), "name": "" }));
    wire_cmd(&mut s, "csd.wire.straighten", json!({}));
    let cleared = link_of(&s, w);
    assert_eq!((cleared.name, cleared.label_t, cleared.points.len()), (None, None, 0));
    assert!(pill(&s).near(default, 1e-6));
}

fn fields(s: &mut Session) -> Value {
    s.execute("csd.project", &json!({})).unwrap()["fields"].clone()
}

#[test]
fn titleblock_is_inserted_once_and_fields_are_set_and_undone() {
    let mut s = Session::new();
    assert_eq!(fields(&mut s)["project"], "");
    let a = handle(&s.execute("csd.titleblock", &json!({ "at": [10, 20] })).unwrap());
    assert_eq!(handle(&s.execute("csd.titleblock", &json!({})).unwrap()), a);
    assert_eq!(fields(&mut s)["brand"], "CHAUVET Professional");
    s.execute("csd.project.set", &json!({ "fields": { "project": "Arena", "drawnBy": "DG", "rev": "B" } })).unwrap();
    let f = fields(&mut s);
    assert_eq!(
        (f["project"].as_str(), f["drawnBy"].as_str(), f["rev"].as_str(), f["venue"].as_str()),
        (Some("Arena"), Some("DG"), Some("B"), Some(""))
    );
    assert!(s.execute("csd.project.set", &json!({ "fields": { "nope": "x" } })).is_err());
    assert!(s.execute("csd.project.set", &json!({ "fields": { "rev": 3 } })).is_err());
    s.undo().unwrap();
    assert_eq!(fields(&mut s)["project"], "");
}

fn block_right(s: &Session, name: &str) -> f64 {
    let b = s.doc().unwrap().block(name).unwrap();
    b.entities
        .iter()
        .filter_map(|e| if let EntityKind::LwPolyline(p) = &e.kind { Some(p.vertices.iter().map(|v| v.p.x).fold(0.0, f64::max)) } else { None })
        .fold(0.0, f64::max)
}

#[test]
fn paper_sizes_the_sheet_and_legend_and_keeps_fields() {
    let mut s = Session::new();
    assert!(s.execute("csd.paper", &json!({ "paper": "letter" })).is_err(), "needs a title block");
    assert!(s.execute("csd.titleblock", &json!({ "paper": "a4" })).is_err());
    assert_eq!(s.execute("csd.titleblock", &json!({})).unwrap()["paper"], "tabloid");
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1632.0 - 22.0);
    s.execute("csd.project.set", &json!({ "fields": { "project": "Arena" } })).unwrap();
    s.execute("csd.legend", &json!({})).unwrap();
    assert_eq!(block_right(&s, "CSD_LEGEND"), 1632.0 - 44.0);

    assert_eq!(s.execute("csd.paper", &json!({ "paper": "letter" })).unwrap()["paper"], "letter");
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1056.0 - 22.0);
    assert_eq!(block_right(&s, "CSD_LEGEND"), 1056.0 - 44.0);
    assert_eq!(fields(&mut s)["project"], "Arena");
    assert_eq!(s.execute("csd.project", &json!({})).unwrap()["paper"], "letter");
    // An existing title block is resized when the paper is given, and left alone when not.
    s.execute("csd.titleblock", &json!({})).unwrap();
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1056.0 - 22.0);
    s.execute("csd.titleblock", &json!({ "paper": "tabloid" })).unwrap();
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1632.0 - 22.0);
    s.undo().unwrap();
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1056.0 - 22.0);
}

#[test]
fn a_new_letter_titleblock_is_letter_sized() {
    let mut s = Session::new();
    s.execute("csd.titleblock", &json!({ "paper": "letter" })).unwrap();
    assert_eq!(block_right(&s, "CSD_TITLEBLOCK"), 1056.0 - 22.0);
    s.execute("csd.legend", &json!({})).unwrap();
    assert_eq!(block_right(&s, "CSD_LEGEND"), 1056.0 - 44.0);
}

#[test]
fn project_set_inserts_the_titleblock_when_missing_in_one_undo_step() {
    let mut s = Session::new();
    s.execute("csd.project.set", &json!({ "fields": { "venue": "O2" } })).unwrap();
    assert_eq!(s.execute("csd.project", &json!({})).unwrap()["exists"], true);
    s.undo().unwrap();
    assert_eq!(s.execute("csd.project", &json!({})).unwrap()["exists"], false);
}

#[test]
fn project_fields_survive_a_dxf_round_trip() {
    let mut s = Session::new();
    s.execute("csd.project.set", &json!({ "fields": { "project": "Tour", "title": "FOH rack", "meta": "v2" } })).unwrap();
    let before = fields(&mut s);
    let text = cadcraft_io::write_dxf(s.doc().unwrap());
    let mut t = Session::empty();
    t.open_drawing(cadcraft_io::read_dxf(text.as_bytes()).unwrap(), "Imported", None);
    assert_eq!(fields(&mut t), before);
}

fn legend_labels(s: &Session) -> Vec<Protocol> {
    cadcraft_csd::legend_protocols(s.doc().unwrap().block("CSD_LEGEND").unwrap())
}

fn legend_text(s: &Session) -> String {
    let b = s.doc().unwrap().block("CSD_LEGEND").unwrap();
    b.entities.iter().filter_map(|e| if let EntityKind::Text(t) = &e.kind { Some(t.value.clone()) } else { None }).collect::<Vec<_>>().join("|")
}

#[test]
fn legend_lists_only_used_protocols_follows_the_wires_and_adds_the_power_note() {
    let (mut s, a, b, _) = setup();
    s.execute("csd.legend", &json!({})).unwrap();
    assert_eq!(legend_labels(&s), vec![Protocol::NetData]);
    assert!(!legend_text(&s).contains("Warning"));
    let p = json!({ "from": a.hex(), "fromSide": "bot", "to": b.hex(), "toSide": "top", "protocol": "power" });
    s.execute("csd.connect", &p).unwrap();
    assert_eq!(legend_labels(&s), vec![Protocol::NetData, Protocol::Power]);
    assert!(legend_text(&s).contains("Warning: Power connection lines"));
    let h = handle(&s.execute("csd.legend", &json!({})).unwrap());
    assert_eq!(handle(&s.execute("csd.legend", &json!({})).unwrap()), h);
    s.undo().unwrap();
    assert_eq!(legend_labels(&s), vec![Protocol::NetData]);
}

#[test]
fn wiring_alone_creates_no_legend() {
    let (s, ..) = setup();
    assert!(s.doc().unwrap().block("CSD_LEGEND").is_none());
}

#[test]
fn bom_footer_defaults_to_the_title_block_project_fields() {
    let mut s = Session::new();
    assert!(super::sheet::project_footer(&s).is_empty());
    s.execute("csd.project.set", &json!({ "fields": { "project": "Demo", "rev": "B" } })).unwrap();
    assert_eq!(super::sheet::project_footer(&s), vec!["Project: Demo   Rev: B".to_string()]);
}

#[test]
fn devices_label_numbers_in_reading_order_and_is_one_undo() {
    let mut s = Session::new();
    let mut at = |x: i32, y: i32| handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [x, y] })).unwrap());
    let (c, a, b) = (at(0, -200), at(300, 0), at(0, 10));
    let hs = json!([c.hex(), a.hex(), b.hex()]);
    let r = s.execute("csd.devices.label", &json!({ "handles": hs, "base": "N", "start": 8, "pad": 2 })).unwrap();
    let got: Vec<_> = r["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["handle"].as_str().unwrap().to_string(), l["label"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(got, vec![(b.hex(), "N08".into()), (a.hex(), "N09".into()), (c.hex(), "N10".into())]);
    let name = |s: &Session, h| insert(s, h).attribs.iter().find(|x| x.tag == "NAME").unwrap().text.value.clone();
    assert_eq!(name(&s, a), "N09");
    // Without `start` every device gets the base.
    s.execute("csd.devices.label", &json!({ "handles": hs, "base": "Rack" })).unwrap();
    assert_eq!((name(&s, a), name(&s, c)), ("Rack".into(), "Rack".into()));
    assert!(s.execute("csd.devices.label", &json!({ "handles": hs, "base": "x", "start": "1" })).is_err());
    assert!(s.execute("csd.devices.label", &json!({ "handles": hs, "base": "x", "start": i64::MAX })).is_err());
    assert_eq!(name(&s, a), "Rack");
    s.undo().unwrap();
    assert_eq!(name(&s, a), "N09");
}

#[test]
fn devices_info_summarises_products_and_common_size() {
    let (mut s, a, b, _) = setup();
    let hs = json!([a.hex(), b.hex()]);
    let r = s.execute("csd.devices.info", &json!({ "handles": hs })).unwrap();
    assert_eq!(r["count"], 2);
    assert_eq!(r["products"].as_array().unwrap().len(), 2);
    assert_eq!(r["size"]["w"], cadcraft_csd::CARD_W);
    s.execute("csd.size", &json!({ "handles": [a.hex()], "w": 140 })).unwrap();
    let r = s.execute("csd.devices.info", &json!({ "handles": hs })).unwrap();
    assert!(r["size"]["w"].is_null() && r["size"]["h"].is_number());
}

#[test]
fn place_grid_lays_out_rows_down_and_columns_across_in_one_undo() {
    let mut s = ortho_session();
    let r = s.execute("csd.place.grid", &json!({ "product": NODE, "at": [0, 0], "rows": 2, "cols": 3, "dx": 100, "dy": 120 })).unwrap();
    let at: Vec<(f64, f64)> = r["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| match &s.doc().unwrap().entity(Handle::parse_hex(h.as_str().unwrap()).unwrap()).unwrap().kind {
            EntityKind::Insert(i) => (i.insert.x, i.insert.y),
            _ => panic!("not an insert"),
        })
        .collect();
    assert_eq!(at, [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (0.0, -120.0), (100.0, -120.0), (200.0, -120.0)]);
    s.undo().unwrap();
    assert_eq!(device_count(&mut s), 0);
}

#[test]
fn place_grid_refuses_hostile_sizes() {
    let mut s = ortho_session();
    for p in [
        json!({ "rows": 0, "cols": 1 }),
        json!({ "rows": 21, "cols": 20 }),
        json!({ "rows": -1, "cols": 2 }),
        json!({ "rows": u64::MAX, "cols": u64::MAX }),
        json!({ "rows": 1, "cols": 1, "dx": "x" }),
    ] {
        let mut p = p;
        p["product"] = json!(NODE);
        p["at"] = json!([0, 0]);
        assert!(s.execute("csd.place.grid", &p).is_err(), "{p}");
    }
    assert_eq!(device_count(&mut s), 0);
}

#[test]
fn clear_removes_devices_wires_and_decorations_but_keeps_the_sheet_furniture() {
    let (mut s, ..) = setup();
    s.execute("csd.project.set", &json!({ "fields": { "project": "Demo" } })).unwrap();
    s.execute("csd.unplaced.add", &json!({ "product": NODE, "qty": 2 })).unwrap();
    let r = s.execute("csd.clear", &json!({})).unwrap();
    assert_eq!((r["devices"].as_u64(), r["wires"].as_u64()), (Some(2), Some(1)));
    assert_eq!(device_count(&mut s), 0);
    assert!(s.doc().unwrap().model.iter().all(|e| !matches!(e.kind, EntityKind::LwPolyline(_)) && deco_from_xdata(&e.common.xdata).is_none()));
    assert_eq!(s.execute("csd.project", &json!({})).unwrap()["fields"]["project"], "Demo");
    assert_eq!(s.execute("csd.unplaced.list", &json!({})).unwrap()["items"].as_array().map(Vec::len), Some(1));
    s.undo().unwrap();
    assert_eq!(device_count(&mut s), 2);
}

fn label_hidden(s: &Session, h: Handle) -> bool {
    let Some(EntityKind::Insert(i)) = s.doc().unwrap().entity(h).map(|e| e.kind.clone()) else { panic!("no insert") };
    i.attribs.iter().find(|a| a.tag == "LABEL").unwrap().invisible
}

#[test]
fn codes_only_hides_labels_on_every_card_old_and_new_and_survives_a_save() {
    let mut s = ortho_session();
    let a = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [0, 0] })).unwrap());
    assert_eq!(s.execute("csd.display", &json!({})).unwrap()["codesOnly"], false);
    assert_eq!(s.execute("csd.display", &json!({ "codesOnly": true })).unwrap()["codesOnly"], true);
    let b = handle(&s.execute("csd.place", &json!({ "product": NODE, "at": [200, 0] })).unwrap());
    assert!(label_hidden(&s, a) && label_hidden(&s, b));
    let mut t = Session::empty();
    t.open_drawing(cadcraft_io::read_dxf(cadcraft_io::write_dxf(s.doc().unwrap()).as_bytes()).unwrap(), "Imported", None);
    assert_eq!(t.execute("csd.display", &json!({})).unwrap()["codesOnly"], true);
    s.execute("csd.display", &json!({ "codesOnly": false })).unwrap();
    assert!(!label_hidden(&s, a) && !label_hidden(&s, b));
    s.undo().unwrap();
    assert!(label_hidden(&s, a));
}

#[test]
fn route_preview_matches_the_move_and_changes_nothing() {
    let (mut s, _, _, w) = setup();
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    let before = verts(&s, w);
    let r = wire_cmd(&mut s, "csd.wire.route", json!({ "wire": w.hex(), "index": 0, "to": [90, -80] }));
    let path: Vec<Vec2> = r["path"].as_array().unwrap().iter().map(|p| Vec2::new(p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
    assert_eq!(verts(&s, w), before);
    wire_cmd(&mut s, "csd.wire.point.move", json!({ "wire": w.hex(), "index": 0, "to": [90, -80] }));
    assert_eq!(verts(&s, w), path);
    assert!(s.execute("csd.wire.route", &json!({ "wire": w.hex(), "index": 5, "to": [0, 0] })).is_err());
}

#[test]
fn moving_a_waypoint_snaps_to_the_grid_when_snap_is_on() {
    let (mut s, _, _, w) = setup();
    wire_cmd(&mut s, "csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] }));
    s.settings.snapmode = true;
    s.settings.snapunit = Vec2::new(10.0, 10.0);
    wire_cmd(&mut s, "csd.wire.point.move", json!({ "wire": w.hex(), "index": 0, "to": [93, 148] }));
    assert_eq!(link_of(&s, w).points, [Vec2::new(90.0, 150.0)]);
}
