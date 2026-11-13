//! The Devices category of the Properties palette: shown when two or more lighting devices are
//! selected. The data is the engine's `csd.devices.info`; every edit is a command.

use cadcraft_engine::doc::Handle;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{edit_field, group_title, number, prop_row, report_bad};
use crate::CadApp;
use crate::theme::Tokens;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Product {
    name: String,
    code: String,
    count: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Size {
    w: Option<f64>,
    h: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Info {
    count: u32,
    products: Vec<Product>,
    size: Size,
}

/// "2 × Pod, 1 × Node".
fn summary(i: &Info) -> String {
    i.products.iter().map(|p| format!("{} × {}", p.count, if p.code.is_empty() { &p.name } else { &p.code })).collect::<Vec<_>>().join(", ")
}

/// The selection's summary, worked out once per drawing revision.
fn info(ui: &egui::Ui, app: &mut CadApp, hexes: &[String]) -> Option<Info> {
    let uid_rev = app.session.state().ok().map(|s| (s.uid, s.revision))?;
    let id = egui::Id::unique(("csd_devices_info", uid_rev, hexes));
    if let Some(c) = ui.data(|d| d.get_temp::<Option<Info>>(id)) {
        return c;
    }
    let i = app.session.execute("csd.devices.info", &json!({ "handles": hexes })).ok().and_then(|v| serde_json::from_value::<Info>(v).ok());
    ui.data_mut(|d| d.insert_temp(id, i.clone()));
    i
}

pub(super) fn show(app: &mut CadApp, ui: &mut egui::Ui, sel: &[Handle]) {
    if sel.len() < 2 {
        return;
    }
    let hexes: Vec<String> = sel.iter().map(|h| h.hex()).collect();
    let Some(i) = info(ui, app, &hexes) else { return };
    if i.count < 2 {
        return;
    }
    let t = Tokens::get();
    let sid = ui.scope_id().with("csd_devices");
    let mut cmd: Option<(&str, Value)> = None;
    let mut bad = None;
    group_title(ui, "Devices", false);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(crate::tf!("{n} devices: {summary}", n = i.count, summary = summary(&i))).small().color(t.text_dim));
    });

    let (base_id, number_id, start_id) = (sid.with("base"), sid.with("number"), sid.with("start"));
    let mut base = ui.data(|d| d.get_temp::<String>(base_id)).unwrap_or_default();
    let mut numbered = ui.data(|d| d.get_temp::<bool>(number_id)).unwrap_or(false);
    let mut start = ui.data(|d| d.get_temp::<i64>(start_id)).unwrap_or(1);
    prop_row(ui, "Label", |ui| {
        let w = ui.available_width() - 10.0;
        ui.add_sized([w, 20.0], egui::TextEdit::singleline(&mut base).font(crate::theme::body()));
    });
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.checkbox(&mut numbered, crate::tl!("Number from"));
        ui.add_enabled(numbered, egui::DragValue::new(&mut start));
        if ui.button(crate::tl!("Apply")).clicked() {
            let mut params = json!({ "handles": hexes, "base": base });
            if numbered {
                params["start"] = json!(start);
            }
            cmd = Some(("csd.devices.label", params));
        }
    });
    ui.data_mut(|d| {
        d.insert_temp(base_id, base);
        d.insert_temp(number_id, numbered);
        d.insert_temp(start_id, start);
    });

    let (lu, lp) = app.session.doc().map(|d| (d.header.i64("LUNITS", 2), d.header.i64("LUPREC", 4))).unwrap_or((2, 4));
    let shown = |v: Option<f64>| v.map_or_else(String::new, |v| cadcraft_engine::units::format_distance(v, lu, lp));
    let w = number(prop_field(ui, "W", sid.with("w"), shown(i.size.w)), &mut bad);
    let h = number(prop_field(ui, "H", sid.with("h"), shown(i.size.h)), &mut bad);
    if w.is_some() || h.is_some() {
        let mut params = json!({ "handles": hexes });
        if let Some(w) = w {
            params["w"] = json!(w);
        }
        if let Some(h) = h {
            params["h"] = json!(h);
        }
        cmd = Some(("csd.size", params));
    }

    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        for (label, mode) in [("Align left", "left"), ("Align top", "top"), ("Align right", "right"), ("Align bottom", "bottom")] {
            if ui.button(crate::i18n::t(label).to_string()).clicked() {
                cmd = Some(("csd.align", json!({ "mode": mode, "handles": hexes })));
            }
        }
        for (label, axis) in [("Distribute horizontally", "x"), ("Distribute vertically", "y")] {
            if ui.add_enabled(i.count >= 3, egui::Button::new(crate::i18n::t(label).to_string())).clicked() {
                cmd = Some(("csd.distribute", json!({ "axis": axis, "handles": hexes })));
            }
        }
    });

    prop_row(ui, "Replace all with", |ui| {
        craft_ui::ComboBox::from_id_salt(sid.with("replace"))
            .selected_text(crate::tl!("Choose a product"))
            .width(ui.available_width() - 10.0)
            .show_ui(ui, |ui| {
                for q in app.session.csd_library.iter() {
                    if ui.selectable_label(false, &q.name).clicked() {
                        cmd = Some(("csd.replace", json!({ "product": q.id, "handles": hexes })));
                    }
                }
            });
    });
    ui.add_space(6.0);

    report_bad(app, bad);
    if let Some((id, params)) = cmd {
        let _ = app.run(id, params);
    }
}

/// A labelled text field; blank where the devices differ.
fn prop_field(ui: &mut egui::Ui, label: &str, id: egui::Id, value: String) -> Option<String> {
    let mut out = None;
    prop_row(ui, label, |ui| out = edit_field(ui, id, value));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_and_summarises() {
        let v = json!({"count": 3, "products": [{"id": "a", "name": "Pod", "code": "P1", "count": 2}, {"name": "Node", "code": "", "count": 1}], "size": {"w": 120.0, "h": null}});
        let i: Info = serde_json::from_value(v).unwrap();
        assert_eq!((i.count, i.size.w, i.size.h), (3, Some(120.0), None));
        assert_eq!(summary(&i), "2 × P1, 1 × Node");
    }
}
