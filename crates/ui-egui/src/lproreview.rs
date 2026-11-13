//! Review step of Import Luminary (`lproreview:<path>`): each fixture type with its quantity and
//! the product it will become, changeable or skipped, before `csd.import.lpro` adds anything.

use serde_json::{Map, Value, json};

use crate::CadApp;

/// Opens the review window ([`dialog`]) for the file path that follows.
pub const DIALOG: &str = "lproreview:";

#[derive(Clone, Default)]
struct Review {
    types: Vec<Value>,
    /// Product id per type, "" to skip.
    picks: Vec<String>,
}

/// The `choices` parameter for the rows as reviewed.
fn choices(types: &[Value], picks: &[String]) -> Value {
    let m: Map<String, Value> = types.iter().zip(picks).filter_map(|(t, p)| Some((t["key"].as_str()?.to_string(), json!(p)))).collect();
    Value::Object(m)
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, spec: &str, open: &mut bool) {
    let path = spec.strip_prefix(DIALOG).unwrap_or_default();
    let sid = egui::Id::unique("lproreview").with(path);
    let mut rv: Review = match ctx.data_mut(|d| d.get_temp(sid)) {
        Some(r) => r,
        None => match app.run("csd.import.lpro", json!({ "path": path, "preview": true })) {
            Ok(r) => {
                let types = r["types"].as_array().cloned().unwrap_or_default();
                let picks = types.iter().map(|t| t["product"].as_str().unwrap_or_default().to_string()).collect();
                Review { types, picks }
            }
            Err(e) => {
                app.session.echo(e);
                *open = false;
                return;
            }
        },
    };
    let library = app.session.csd_library.clone();
    let label = |id: &str| match library.iter().find(|p| p.id == id) {
        Some(p) => p.name.clone(),
        None => crate::i18n::t("Skip").to_string(),
    };
    let (mut keep, mut import, mut cancel) = (*open, false, false);
    egui::Window::new(crate::tl!("Import Luminary Project")).id(egui::Id::unique("Import Luminary review")).open(&mut keep).collapsible(false).show(
        ctx,
        |ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("lproreview_grid").num_columns(3).striped(true).show(ui, |ui| {
                    ui.strong(crate::tl!("Fixture type"));
                    ui.strong(crate::tl!("Qty"));
                    ui.strong(crate::tl!("Product"));
                    ui.end_row();
                    for (t, pick) in rv.types.iter().zip(rv.picks.iter_mut()) {
                        let name = t["name"].as_str().filter(|n| !n.is_empty()).or(t["luminaire"].as_str()).unwrap_or_default();
                        ui.label(name);
                        ui.label(t["quantity"].as_u64().unwrap_or_default().to_string());
                        let key = t["key"].as_str().unwrap_or_default();
                        craft_ui::ComboBox::from_id_salt(("lproreview_pick", key)).selected_text(label(pick)).width(240.0).show_ui(ui, |ui| {
                            ui.selectable_value(pick, String::new(), crate::tl!("Skip"));
                            for p in library.iter().filter(|p| !cadcraft_csd::is_builtin(&p.id)) {
                                ui.selectable_value(pick, p.id.clone(), &p.name);
                            }
                        });
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                import = ui.button(crate::tl!("Import")).clicked();
                if ui.button(crate::tl!("Cancel")).clicked() {
                    cancel = true;
                }
            });
        },
    );
    keep &= !cancel;
    if import {
        match app.run("csd.import.lpro", json!({ "path": path, "choices": choices(&rv.types, &rv.picks) })) {
            Ok(_) => keep = false,
            Err(e) => app.session.echo(e),
        }
    }
    *open = keep;
    ctx.data_mut(|d| {
        if keep {
            d.insert_temp(sid, rv);
        } else {
            d.remove_temp::<Review>(sid);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_map_each_type_key_to_its_pick() {
        let types = vec![json!({ "key": "a" }), json!({ "key": "b" })];
        assert_eq!(choices(&types, &["P1".into(), String::new()]), json!({ "a": "P1", "b": "" }));
    }
}
