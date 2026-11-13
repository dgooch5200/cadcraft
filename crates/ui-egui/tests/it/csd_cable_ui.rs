//! Lighting cables: the right-click menu and dragging, double-clicking waypoints.

use cadcraft_csd::WireLink;
use cadcraft_doc::Handle;
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
    let mut app = CadApp::new(crate::px_session(), Services::default());
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
        "Straighten cable (1 point)",
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

    let straighten = id_of(&app, "Straighten cable (1 point)");
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

fn device_count(app: &mut CadApp) -> usize {
    app.run("csd.netlist", json!({})).unwrap()["devices"].as_array().unwrap().len()
}

#[test]
fn canvas_menu_places_a_grid_pastes_at_the_cursor_and_clears_the_drawing() {
    let (mut app, _) = app_with_cable();
    app.session.set_selection(Vec::new());
    app.canvas.menu_world = Some(Vec2::new(500.0, 0.0));
    assert!(!has(&app, "Place grid here..."), "nothing armed");
    app.run("csd.arm", json!({ "product": "epix-flex-drive" })).unwrap();
    assert!(has(&app, "Place one here") && has(&app, "Place grid here..."));
    let grid = id_of(&app, "Place grid here...");
    context_menu::choose(&mut app, &grid);
    assert_eq!(app.ui.dialog.as_deref(), Some("placegrid:500,0"));

    // Enter in the window places the default 2 x 2 block at the click.
    let ctx = egui::Context::default();
    let mut open = true;
    let enter = Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![enter], ..Default::default() }, |ui| {
        cadcraft_ui_egui::placegrid::dialog(&mut app, ui.ctx(), "placegrid:500,0", &mut open);
    });
    out.textures_delta.clear();
    assert!(!open);
    assert_eq!(device_count(&mut app), 6);

    // Paste puts the copy under the cursor, not on the original.
    let a = app.run("csd.device.at", json!({ "at": [0, 0] })).unwrap()["handle"].as_str().unwrap().to_string();
    app.run("copyclip", json!({ "handles": [a] })).unwrap();
    app.canvas.menu_world = Some(Vec2::new(900.0, 0.0));
    let paste = id_of(&app, "Paste");
    assert!(paste.starts_with("run:pasteclip:") && paste.contains("900"), "{paste}");
    context_menu::choose(&mut app, &paste);
    assert_eq!(device_count(&mut app), 7);
    assert!(app.run("csd.device.at", json!({ "at": [935, 36] })).unwrap()["handle"].is_string());

    app.session.set_selection(Vec::new());
    let clear = id_of(&app, "Clear drawing");
    context_menu::choose(&mut app, &clear);
    assert_eq!(device_count(&mut app), 0);
    app.session.undo().unwrap();
    assert_eq!(device_count(&mut app), 7);
}

fn harness(app: CadApp) -> Harness<'static, CadApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h
}

fn drag(h: &mut Harness<'static, CadApp>, from: Vec2, to: Vec2) {
    let xf = h.state().canvas.xf.unwrap();
    let (from, to) = (xf.to_screen(from), xf.to_screen(to));
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
}

#[test]
fn dragging_a_waypoint_moves_it_and_double_clicking_removes_it() {
    let (mut app, w) = app_with_cable();
    app.session.set_selection(vec![w]);
    let mut h = harness(app);
    drag(&mut h, Vec2::new(100.0, 150.0), Vec2::new(60.0, 200.0));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
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

#[test]
fn rename_cable_dialog_focuses_its_text_field() {
    let (mut app, w) = app_with_cable();
    let ctx = egui::Context::default();
    let mut open = true;
    let spec = format!("{}{}", context_menu::WIRE_NAME, w.hex());
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| cadcraft_ui_egui::wirename::dialog(&mut app, ui.ctx(), &spec, &mut open));
    out.textures_delta.clear();
    assert!(ctx.memory(|m| m.focused()).is_some());
}

#[test]
fn waypoints_of_every_selected_cable_can_be_dragged() {
    let (mut app, w) = app_with_cable();
    let a = handle(&app.run("csd.device.at", json!({ "at": [0, 0] })).unwrap());
    let c = handle(&app.run("csd.place", json!({ "product": "epix-flex-drive", "at": [200, -60] })).unwrap());
    let p = json!({ "from": a.hex(), "fromSide": "bot", "to": c.hex(), "toSide": "top", "protocol": "netdata" });
    let w2 = handle(&app.run("csd.connect", p).unwrap());
    app.run("csd.wire.point.add", json!({ "wire": w2.hex(), "at": [100, -80] })).unwrap();
    app.session.set_selection(vec![w, w2]);
    let mut h = harness(app);
    drag(&mut h, Vec2::new(100.0, -80.0), Vec2::new(60.0, -100.0));
    let pts = points(h.state(), w2);
    assert!(pts[0].near(Vec2::new(60.0, -100.0), 2.0), "{:?}", pts[0]);
}
