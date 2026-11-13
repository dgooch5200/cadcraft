//! The Device categories of the Properties palette: shown above the generic ones when exactly one
//! lighting device is selected. The data is the engine's `csd.device.info`; every edit is a command.

use cadcraft_csd::Protocol;
use cadcraft_engine::doc::Handle;
use egui::{Color32, Sense, Stroke, vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{edit_field, group_title, number, prop_row, read_only, report_bad};
use crate::CadApp;
use crate::icons::{self, Icon};
use crate::theme::Tokens;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Product {
    id: String,
    name: String,
    code: String,
    brand: String,
    cat: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Port {
    protocol: String,
    label: String,
    used: u64,
    capacity: u64,
    pooled: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Other {
    label: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Connection {
    wire: String,
    protocol: String,
    dir: String,
    other: Other,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Info {
    known: bool,
    product: Product,
    label: String,
    notes: String,
    ports: Vec<Port>,
    connections: Vec<Connection>,
    #[serde(rename = "sameProduct")]
    same_product: u32,
    at: [f64; 2],
    size: [f64; 2],
    #[serde(rename = "libSize")]
    lib_size: [f64; 2],
}

fn parse_info(v: Value) -> Option<Info> {
    serde_json::from_value::<Info>(v).ok()
}

fn is_full(p: &Port) -> bool {
    p.used >= p.capacity
}

/// "PoE ← SYS_PatchBay" for an incoming wire, "PoE → GN2" for an outgoing one.
fn connection_text(c: &Connection) -> String {
    let proto = Protocol::from_id(&c.protocol).map_or_else(|| c.protocol.clone(), |p| p.label().to_string());
    format!("{proto} {} {}", if c.dir == "in" { "←" } else { "→" }, c.other.label)
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn dot(ui: &mut egui::Ui, protocol: &str) {
    let c = Protocol::from_id(protocol).map_or(Color32::GRAY, |p| rgb(p.color()));
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, c);
}

/// The device's info, worked out once per drawing revision.
fn info(ui: &egui::Ui, app: &mut CadApp, h: Handle) -> Option<Info> {
    let uid_rev = app.session.state().ok().map(|s| (s.uid, s.revision))?;
    let id = egui::Id::unique(("csd_device_info", uid_rev, h));
    if let Some(c) = ui.data(|d| d.get_temp::<Option<Info>>(id)) {
        return c;
    }
    let i = app.session.execute("csd.device.info", &json!({ "handle": h.hex() })).ok().and_then(parse_info);
    ui.data_mut(|d| d.insert_temp(id, i.clone()));
    i
}

fn is_device(app: &CadApp, h: Handle) -> bool {
    app.session
        .doc()
        .ok()
        .and_then(|d| d.entity(h))
        .is_some_and(|e| matches!(&e.kind, cadcraft_doc::EntityKind::Insert(i) if i.block.starts_with("CSD_")))
}

/// A collapsible category; `body` runs while it is open.
fn category(app: &mut CadApp, ui: &mut egui::Ui, name: &str, body: impl FnOnce(&mut CadApp, &mut egui::Ui)) {
    let collapsed = app.ui.collapsed_groups.iter().any(|g| g == name);
    let (_, resp) = group_title(ui, name, collapsed);
    icons::describe_control(ui, &resp, name, Some(!collapsed), false);
    if resp.clicked() {
        match app.ui.collapsed_groups.iter().position(|g| g == name) {
            Some(i) => drop(app.ui.collapsed_groups.remove(i)),
            None => app.ui.collapsed_groups.push(name.to_string()),
        }
    }
    if !collapsed {
        body(app, ui);
    }
}

/// A multiline field that commits on focus loss.
fn notes_field(ui: &mut egui::Ui, id: egui::Id, value: &str) -> Option<String> {
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| value.to_string());
    let w = ui.available_width() - 10.0;
    let resp = ui.add(egui::TextEdit::multiline(&mut buf).id(id.with("te")).desired_rows(3).desired_width(w).font(crate::theme::body()));
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    (resp.lost_focus() && buf != value).then_some(buf)
}

/// The sheet's text fields: label, `csd.project` key.
const SHEET_FIELDS: [(&str, &str); 9] = [
    ("Project", "project"),
    ("Venue", "venue"),
    ("Location", "location"),
    ("Drawn by", "drawnBy"),
    ("Date", "date"),
    ("Sheet", "sheet"),
    ("Rev", "rev"),
    ("Drawing title", "title"),
    ("Meta", "meta"),
];

/// The project's `csd.project` answer, worked out once per drawing revision.
fn project(ui: &egui::Ui, app: &mut CadApp) -> Option<Value> {
    let uid_rev = app.session.state().ok().map(|s| (s.uid, s.revision))?;
    let id = egui::Id::unique(("csd_project", uid_rev));
    if let Some(c) = ui.data(|d| d.get_temp::<Option<Value>>(id)) {
        return c;
    }
    let v = app.session.execute("csd.project", &json!({})).ok();
    ui.data_mut(|d| d.insert_temp(id, v.clone()));
    v
}

/// The Title Block category: for a selected title block, and with nothing selected on a drawing
/// that has lighting devices or a title block.
fn show_title_block(app: &mut CadApp, ui: &mut egui::Ui, sel: &[Handle]) {
    let Some(info) = project(ui, app) else { return };
    let tb = info["handle"].as_str().and_then(Handle::parse_hex);
    let wanted = match *sel {
        [] => tb.is_some() || info["devices"].as_u64().unwrap_or(0) > 0,
        [h] => Some(h) == tb,
        _ => false,
    };
    if !wanted {
        return;
    }
    let sid = ui.scope_id().with("csd_title_block");
    let mut cmd: Option<(&str, Value)> = None;
    category(app, ui, "Title Block", |_, ui| {
        for (label, key) in SHEET_FIELDS {
            let now = info["fields"][key].as_str().unwrap_or_default().to_string();
            prop_row(ui, label, |ui| {
                if let Some(v) = edit_field(ui, sid.with(key), now) {
                    cmd = Some(("csd.project.set", json!({ "fields": { key: v } })));
                }
            });
        }
        if tb.is_none() {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(crate::tl!("Typing here adds a title block to the drawing.")).small().color(Tokens::get().text_dim));
            });
        }
    });
    if let Some((id, params)) = cmd {
        let _ = app.run(id, params);
    }
}

/// Adds the Device categories when `sel` is exactly one lighting device, and the Title Block.
pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, sel: &[Handle]) {
    show_title_block(app, ui, sel);
    let [h] = *sel else { return };
    if !is_device(app, h) {
        return;
    }
    let Some(i) = info(ui, app, h) else { return };
    let t = Tokens::get();
    let hex = h.hex();
    let sid = ui.scope_id().with(("csd_device", h));
    let mut cmd: Option<(&str, Value)> = None;
    let mut bad = None;
    let p = &i.product;

    category(app, ui, "Device", |app, ui| {
        prop_row(ui, "Product", |ui| read_only(ui, &p.name));
        if i.known {
            prop_row(ui, "Brand", |ui| read_only(ui, &format!("{} · {}", p.brand, p.cat)));
            prop_row(ui, "Code", |ui| read_only(ui, &p.code));
        } else {
            prop_row(ui, "Id", |ui| read_only(ui, &p.id));
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(crate::tl!("Not in the library")).small().color(t.text_dim));
            });
        }
        prop_row(ui, "Label", |ui| {
            let r = ui.scope(|ui| edit_field(ui, sid.with("label"), i.label.clone()));
            if let Some(v) = r.inner {
                cmd = Some(("csd.device.set", json!({ "handle": hex, "label": v })));
            }
            if i.label.is_empty() {
                ui.label(egui::RichText::new(&p.code).small().color(t.text_faint));
            }
        });
        prop_row(ui, "Notes", |ui| {
            if let Some(v) = notes_field(ui, sid.with("notes"), &i.notes) {
                cmd = Some(("csd.device.set", json!({ "handle": hex, "notes": v })));
            }
        });
        let all_id = sid.with("all");
        let mut all = ui.data(|d| d.get_temp::<bool>(all_id)).unwrap_or(false);
        prop_row(ui, "Replace with", |ui| {
            craft_ui::ComboBox::from_id_salt(sid.with("replace"))
                .selected_text(crate::tl!("Choose a product"))
                .width(ui.available_width() - 10.0)
                .show_ui(ui, |ui| {
                    for q in app.session.csd_library.iter() {
                        if ui.selectable_label(q.id == p.name, &q.name).clicked() {
                            let mut params = json!({ "product": q.id, "handles": [hex] });
                            if all {
                                params["allOfProduct"] = json!(true);
                            }
                            cmd = Some(("csd.replace", params));
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let label = crate::tf!("All {n} {code} on this drawing", n = i.same_product, code = p.code);
            ui.checkbox(&mut all, label);
        });
        ui.data_mut(|d| d.insert_temp(all_id, all));
        ui.horizontal_wrapped(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(crate::tl!(
                    "Position, mounting and accessories stay as they are. Cables the new product cannot carry are removed — undo puts everything back in one step."
                ))
                .small()
                .color(t.text_dim),
            );
        });
    });

    if !i.ports.is_empty() {
        category(app, ui, "Ports", |_, ui| {
            for port in &i.ports {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    dot(ui, &port.protocol);
                    ui.label(&port.label);
                    if port.pooled {
                        ui.label(egui::RichText::new(crate::tl!("pooled")).small().color(t.text_dim));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let c = if is_full(port) { t.danger } else { t.text_dim };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                port.used,
                                if port.capacity > 1_000_000 { "∞".to_string() } else { port.capacity.to_string() }
                            ))
                            .color(c),
                        );
                    });
                });
            }
        });
    }

    if !i.connections.is_empty() {
        category(app, ui, "Connections", |_, ui| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(i.connections.len().to_string()).small().color(t.text_dim));
            });
            for c in &i.connections {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    dot(ui, &c.protocol);
                    ui.label(connection_text(c));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if icons::button(ui, Icon::Erase, 20.0, "Delete connection", false).clicked() {
                            cmd = Some(("erase", json!({ "handles": [c.wire] })));
                        }
                    });
                });
            }
        });
    }

    category(app, ui, "Position & size", |app, ui| {
        let (lu, lp) = app.session.doc().map(|d| (d.header.i64("LUNITS", 2), d.header.i64("LUPREC", 4))).unwrap_or((2, 4));
        let shown = |v: f64| cadcraft_engine::units::format_distance(v, lu, lp);
        let mut field = |ui: &mut egui::Ui, label: &str, key: &str, v: f64| {
            let mut out = None;
            prop_row(ui, label, |ui| out = number(edit_field(ui, sid.with(key), shown(v)), &mut bad));
            out
        };
        let x = field(ui, "X", "x", i.at[0]);
        let y = field(ui, "Y", "y", i.at[1]);
        let w = field(ui, "W", "w", i.size[0]);
        let hh = field(ui, "H", "h", i.size[1]);
        if x.is_some() || y.is_some() {
            let delta = [x.map_or(0.0, |x| x - i.at[0]), y.map_or(0.0, |y| y - i.at[1])];
            cmd = Some(("move", json!({ "handles": [hex], "delta": delta })));
        }
        if w.is_some() || hh.is_some() {
            let mut params = json!({ "handles": [hex] });
            if let Some(w) = w {
                params["w"] = json!(w);
            }
            if let Some(h) = hh {
                params["h"] = json!(h);
            }
            cmd = Some(("csd.size", params));
        }
        ui.horizontal_wrapped(|ui| {
            ui.add_space(8.0);
            let hint = crate::tf!(
                "The library's size for {name}. Taller sides fan more cables apart.",
                name = format!("{} ({} × {})", p.name, shown(i.lib_size[0]), shown(i.lib_size[1]))
            );
            ui.label(egui::RichText::new(hint).small().color(t.text_dim));
        });
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let w = ui.available_width() - 8.0;
        let b = egui::Button::new(egui::RichText::new(crate::tl!("Delete device")).color(t.danger))
            .stroke(Stroke::new(1.0, t.danger))
            .min_size(vec2(w, 26.0));
        if ui.add_sized(vec2(w, 26.0), b).clicked() {
            cmd = Some(("csd.device.delete", json!({ "handles": [hex] })));
        }
    });
    ui.add_space(6.0);
    let (sr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(sr.x_range().shrink(6.0), sr.center().y, Stroke::new(1.0, t.border));

    report_bad(app, bad);
    if let Some((id, params)) = cmd {
        let _ = app.run(id, params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_and_ignores_unknown() {
        let v = json!({"known": true, "product": {"name": "Pod", "code": "P1"}, "ports": [{"protocol": "dmx", "label": "DMX", "used": 1, "capacity": 1, "pooled": true}],
            "connections": [{"wire": "2A", "protocol": "network", "dir": "in", "other": {"handle": "1", "label": "Bay"}}], "sameProduct": 3, "at": [1, 2], "size": [3, 4], "libSize": [5, 6]});
        let i = parse_info(v).unwrap();
        assert_eq!((i.same_product, i.at, i.lib_size), (3, [1.0, 2.0], [5.0, 6.0]));
        assert!(is_full(&i.ports[0]) && i.ports[0].pooled);
        assert_eq!(connection_text(&i.connections[0]), "PoE ← Bay");
        assert!(!parse_info(json!({"known": false, "product": {"id": "X"}})).unwrap().known);
        assert!(parse_info(json!("nope")).is_none());
    }

    #[test]
    fn full_only_at_capacity() {
        let p = |used| Port { used, capacity: 2, ..Port::default() };
        assert!(!is_full(&p(1)) && is_full(&p(2)) && is_full(&p(3)));
    }
}
