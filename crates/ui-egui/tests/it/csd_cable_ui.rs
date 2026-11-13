//! Lighting cables: the right-click menu and dragging, double-clicking waypoints.

use cadcraft_csd::WireLink;
use cadcraft_doc::Handle;
use cadcraft_engine::Session;
use cadcraft_engine::geom::Vec2;
use cadcraft_ui_egui::menus::Entry;
use cadcraft_ui_egui::{CadApp, Services, context_menu};
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn handle(r: &serde_json::Value) -> Handle {
    Handle::parse_hex(r["handle"].as_str().unwrap()).unwrap()
}

/// A wired console and node with one waypoint at (100, 150).
fn app_with_cable() -> (CadApp, Handle) {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.run("ortho", json!({ "on": true })).unwrap();
    let a = handle(&app.run("csd.place", json!({ "product": "CHAMMQ50", "at": [0, 0] })).unwrap());
    let b = handle(&app.run("csd.place", json!({ "product": "epix-flex-drive", "at": [200, 60] })).unwrap());
    let p = json!({ "from": a.hex(), "fromSide": "rgt", "to": b.hex(), "toSide": "lft", "protocol": "netdata" });
    let w = handle(&app.run("csd.connect", p).unwrap());
    app.run("csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] })).unwrap();
    app.session.execute("view.set", &json!({ "center": [100, 70], "height": 400 })).unwrap();
    app.canvas.zoom_pending = false;
    (app, w)
}

/// (label, id) of every item of the canvas menu.
fn items(app: &CadApp) -> Vec<(String, String)> {
    fn walk(es: &[Entry], out: &mut Vec<(String, String)>) {
        for e in es {
            match e {
                Entry::Item { label, id, .. } => out.push((label.clone(), id.clone())),
                Entry::Sub { children, .. } => walk(children, out),
            }
        }
    }
    let mut out = Vec::new();
    context_menu::entries(app).iter().for_each(|g| walk(g, &mut out));
    out
}

fn has(app: &CadApp, label: &str) -> bool {
    items(app).iter().any(|(l, _)| l == label)
}

fn id_of(app: &CadApp, label: &str) -> String {
    items(app).into_iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{label} missing")).1
}

fn points(app: &CadApp, w: Handle) -> Vec<Vec2> {
    WireLink::from_xdata(&app.session.doc().unwrap().entity(w).unwrap().common.xdata).unwrap().points
}

#[test]
fn cable_menu_offers_point_label_name_and_delete_items() {
    let (mut app, w) = app_with_cable();
    // Right-click on the waypoint selects the cable; the menu offers removing that point.
    let click = Vec2::new(101.0, 149.0);
    app.canvas.menu_world = Some(click);
    context_menu::select_device_at(&mut app, Some(click));
    assert_eq!(app.session.selection(), vec![w]);
    for want in [
        "Remove point",
        "Straighten cable (1 points)",
        "Move label here",
        "Align label to middle of segment",
        "Centre label",
        "Rename cable...",
        "Delete cable",
        "Deselect all",
    ] {
        assert!(has(&app, want), "{want} missing from {:?}", items(&app));
    }
    // Away from a waypoint it adds one instead.
    app.canvas.menu_world = Some(Vec2::new(300.0, 300.0));
    assert!(has(&app, "Add point here") && !has(&app, "Remove point"));

    let straighten = id_of(&app, "Straighten cable (1 points)");
    context_menu::choose(&mut app, &straighten);
    assert!(points(&app, w).is_empty());

    // Nothing selected: the canvas menu offers straightening only while a cable is bent.
    assert!(!has(&app, "Straighten cable (0 points)"));
    app.run("csd.wire.point.add", json!({ "wire": w.hex(), "at": [100, 150] })).unwrap();
    app.session.set_selection(Vec::new());
    assert!(has(&app, "Straighten 1 bent cables"));

    app.session.set_selection(vec![w]);
    let rename = id_of(&app, "Rename cable...");
    context_menu::choose(&mut app, &rename);
    assert_eq!(app.ui.dialog, Some(rename));
    let delete = id_of(&app, "Delete cable");
    context_menu::choose(&mut app, &delete);
    assert!(app.session.doc().unwrap().entity(w).is_none());
}

#[test]
fn dragging_a_waypoint_moves_it_and_double_clicking_removes_it() {
    let (mut app, w) = app_with_cable();
    app.session.set_selection(vec![w]);
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    let xf = h.state().canvas.xf.unwrap();
    let (from, to) = (xf.to_screen(Vec2::new(100.0, 150.0)), xf.to_screen(Vec2::new(60.0, 200.0)));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(Event::PointerMoved(from));
    h.run_steps(2);
    h.event(button(from, true));
    h.run_steps(1);
    for i in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    h.event(button(to, false));
    h.run_steps(3);
    let pts = points(h.state(), w);
    assert_eq!(pts.len(), 1);
    assert!(pts[0].near(Vec2::new(60.0, 200.0), 2.0), "{:?}", pts[0]);

    let at = h.state().canvas.xf.unwrap().to_screen(pts[0]);
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(button(at, pressed));
            h.run_steps(1);
        }
    }
    h.run_steps(2);
    assert!(points(h.state(), w).is_empty(), "double-click removed the waypoint");
}
