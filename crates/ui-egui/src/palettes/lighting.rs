//! The Lighting group of the Drafting tool set: the product library as a searchable list. Clicking
//! a product arms it and starts the interactive `csd.place`, which keeps placing until Enter or Esc.

use cadcraft_csd::{Product, Protocol, builtin_products};
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::icons::Icon;
use crate::theme::Tokens;
use crate::{CadApp, Pick};

const ROW_H: f32 = 46.0;
const LIST_H: f32 = 180.0;

#[derive(Clone, Default)]
struct Filter {
    query: String,
    /// Empty is "All".
    category: String,
}

fn matches(p: &Product, f: &Filter, needle: &str) -> bool {
    (f.category.is_empty() || p.cat == f.category)
        && (needle.is_empty() || [&p.id, &p.name, &p.code].iter().any(|s| s.to_lowercase().contains(needle)))
}

/// "DMX 1 in / 1 out, PoE 2 out".
fn port_summary(p: &Product) -> String {
    let n = |count: u32, word: &str| (count > 0).then(|| format!("{count} {word}"));
    Protocol::ALL
        .iter()
        .filter_map(|pr| {
            let (i, o) = p.ports(*pr);
            let ends = [n(i, "in"), n(o, "out")].into_iter().flatten().collect::<Vec<_>>().join(" / ");
            (!ends.is_empty()).then(|| format!("{} {ends}", pr.label()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn section(ui: &mut egui::Ui, t: &Tokens, name: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(crate::i18n::t(name)).small().color(t.text_dim));
    });
}

/// Signal protocols, the Connect command and the drafting toggles, after the web app's toolbar.
fn toolbar(app: &mut CadApp, ui: &mut egui::Ui, t: &Tokens) {
    let gap = 4.0;
    section(ui, t, "Signal");
    let active = app.session.csd_protocol;
    let mut set = None;
    for row in Protocol::ALL.chunks(2) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.x = gap;
            let w = (ui.available_width() - 8.0 - gap) / 2.0;
            for pr in row {
                let c = rgb(pr.color());
                let (mut fill, mut text) = (Color32::TRANSPARENT, c);
                if *pr == active {
                    (fill, text) = (c, if c.r() as u32 * 3 + c.g() as u32 * 6 + c.b() as u32 > 1280 { Color32::BLACK } else { Color32::WHITE });
                }
                let b = egui::Button::new(egui::RichText::new(crate::i18n::t(pr.label())).color(text))
                    .fill(fill)
                    .stroke(Stroke::new(1.5, c))
                    .min_size(vec2(w, 20.0));
                let resp = ui.add_sized(vec2(w, 20.0), b);
                crate::icons::describe_control(ui, &resp, pr.label(), Some(*pr == active), false);
                if resp.clicked() {
                    set = Some(pr.id());
                }
            }
        });
    }
    if let Some(id) = set {
        let _ = app.run("csd.protocol", json!({ "protocol": id }));
    }
    let connecting = app.session.running.as_ref().is_some_and(|r| r.id == "csd.connect");
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = gap;
        let total = ui.available_width() - 8.0;
        let w = (total - gap) * 0.58;
        let b = egui::Button::new(egui::RichText::new(crate::tl!("Connect")).strong().size(15.0)).selected(connecting).min_size(vec2(w, 30.0));
        let resp = ui.add_sized(vec2(w, 30.0), b);
        crate::icons::describe_control(ui, &resp, "Connect", Some(connecting), false);
        if resp.clicked() {
            app.cmd.buffer.clear();
            app.start("csd.connect");
        }
        let open = app.ui.csd_sched.open;
        let w = total - gap - w;
        let resp = ui.add_sized(vec2(w, 30.0), egui::Button::new(crate::tl!("Schedules")).selected(open));
        crate::icons::describe_control(ui, &resp, "Schedules", Some(open), false);
        if resp.clicked() {
            let _ = app.run("ui.csd.schedule", serde_json::Value::Null);
        }
    });
    ui.add_space(4.0);
    let st = app.session.settings.clone();
    let mut toggle = None;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = gap;
        let w = (ui.available_width() - 8.0 - gap) / 2.0;
        for (icon, on, cmd, label) in [(Icon::Ortho, st.orthomode, "ortho", "Ortho"), (Icon::Snap, st.snapmode, "snap", "Snap")] {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
            let p = ui.painter();
            if on {
                p.rect_filled(r, 3.0, t.toggle_on.gamma_multiply(0.35));
            } else if resp.hovered() {
                p.rect_filled(r, 3.0, t.control_hover.gamma_multiply(0.5));
            }
            p.rect_stroke(r, 3.0, Stroke::new(1.0, if on { t.toggle_on } else { t.border }), egui::StrokeKind::Inside);
            crate::icons::paint(p, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(16.0, 16.0)), icon, false);
            p.text(pos2(r.left() + 28.0, r.center().y), egui::Align2::LEFT_CENTER, crate::i18n::t(label), egui::FontId::proportional(13.0), t.text);
            crate::icons::describe_control(ui, &resp, label, Some(on), false);
            if resp.clicked() {
                toggle = Some(cmd);
            }
        }
    });
    if let Some(c) = toggle {
        crate::menus::run_transparent(app, c);
    }
    grid_size(app, ui, &st);
}

/// The snap grid in drawing units, as the web app's grid-size select (greyed while Snap is off), with the
/// sheet's title block (selected once placed, so its fields show in Properties) and line key beside it.
fn grid_size(app: &mut CadApp, ui: &mut egui::Ui, st: &cadcraft_engine::Settings) {
    const CHOICES: [f64; 4] = [4.0, 8.0, 12.0, 24.0];
    let unit = app.session.drafting_settings().snapunit.x;
    let mut pick = None;
    let mut sheet = None;
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add_enabled_ui(st.snapmode, |ui| {
            craft_ui::ComboBox::from_id_salt(ui.scope_id().with("csd_grid"))
                .selected_text(format!("{} px", unit))
                .width(52.0)
                .show_ui(ui, |ui| {
                    for c in CHOICES {
                        if ui.selectable_label((unit - c).abs() < 1e-9, format!("{c} px")).clicked() {
                            pick = Some(c);
                        }
                    }
                })
                .response
                .accessible_name(crate::tl!("Snap grid size"));
        });
        let w = (ui.available_width() - 8.0 - 8.0) / 3.0;
        for (label, cmd) in [("Title block", "csd.titleblock"), ("Legend", "csd.legend")] {
            if ui.add_sized(vec2(w, 20.0), egui::Button::new(crate::i18n::t(label)).truncate()).clicked() {
                sheet = Some(cmd);
            }
        }
        let codes = app.session.doc().is_ok_and(|d| d.header.i64("CSDCODESONLY", 0) != 0);
        let resp = ui.add_sized(vec2(w, 20.0), egui::Button::new(crate::tl!("Codes only")).selected(codes).truncate());
        crate::icons::describe_control(ui, &resp, "Codes only", Some(codes), false);
        if resp.clicked() {
            let _ = app.run("csd.display", json!({ "codesOnly": !codes }));
        }
    });
    if let Some(h) =
        sheet.and_then(|cmd| app.run(cmd, json!({})).ok()).and_then(|v| v["handle"].as_str().and_then(cadcraft_engine::doc::Handle::parse_hex))
    {
        app.session.set_selection(vec![h]);
    }
    if let Some(c) = pick {
        for var in ["SNAPUNIT", "GRIDUNIT"] {
            let _ = app.run("setvar", json!({ "name": var, "value": [c, c] }));
        }
    }
}

/// The group's body. The header is the caller's.
pub(super) fn body(app: &mut CadApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let id = ui.scope_id().with("csd_lighting");
    let mut f = ui.ctx().data_mut(|d| d.get_temp::<Filter>(id)).unwrap_or_default();
    let library = app.session.csd_library.clone();
    // The built-ins are in no library, so they are listed after it.
    let all: Vec<&Product> = library.iter().chain(builtin_products()).collect();
    let mut cats: Vec<&str> = Vec::new();
    for p in &all {
        if !cats.contains(&p.cat.as_str()) {
            cats.push(&p.cat);
        }
    }
    toolbar(app, ui, &t);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let w = (ui.available_width() - 12.0) / 2.0;
        if ui.add_sized(vec2(w, 20.0), egui::Button::new(crate::tl!("Import .cpsd…"))).clicked() {
            let _ = app.run("csd.import.cpsd", serde_json::Value::Null);
        }
        if ui.add_sized(vec2(w, 20.0), egui::Button::new(crate::tl!("Export .cpsd…"))).clicked() {
            let save = |app: &mut CadApp, path: String| app.run("csd.export.cpsd", json!({ "path": path }));
            if let Some(Err(e)) = app.pick_then(Pick::Save("drawing.cpsd".into()), save) {
                app.session.echo(e);
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut f.query).hint_text(crate::tl!("Search products")).desired_width(ui.available_width() - 8.0))
            .accessible_name(crate::tl!("Search products"));
    });
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let shown = if f.category.is_empty() { crate::tl!("All").to_string() } else { f.category.clone() };
        craft_ui::ComboBox::from_id_salt(id.with("cat"))
            .selected_text(shown)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.category, String::new(), crate::tl!("All"));
                for c in &cats {
                    ui.selectable_value(&mut f.category, c.to_string(), *c);
                }
            })
            .response
            .accessible_name(crate::tl!("Category"));
    });
    ui.add_space(4.0);
    let needle = f.query.trim().to_lowercase();
    let hits: Vec<&Product> = all.iter().copied().filter(|p| matches(p, &f, &needle)).collect();
    let placing = app.session.running.as_ref().is_some_and(|r| r.id == "csd.place");
    let mut picked = None;
    egui::ScrollArea::vertical().id_salt(id.with("list")).max_height(LIST_H).auto_shrink([false, true]).show_rows(
        ui,
        ROW_H,
        hits.len(),
        |ui, range| {
            for p in hits.get(range).unwrap_or_default() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let active = placing && app.session.csd_armed.as_deref() == Some(p.id.as_str());
                let pt = ui.painter();
                if active {
                    pt.rect_filled(r, 0.0, t.selection);
                } else if resp.hovered() {
                    pt.rect_filled(r, 0.0, t.hover);
                }
                let sw = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 11.0), vec2(30.0, 22.0));
                pt.rect_filled(sw, 2.0, rgb(p.color));
                pt.rect_stroke(sw, 2.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let x = sw.right() + 8.0;
                pt.text(pos2(x, r.center().y - 8.0), egui::Align2::LEFT_CENTER, &p.code, egui::FontId::proportional(14.0), t.text);
                let name = pt.layout_no_wrap(p.name.clone(), egui::FontId::proportional(12.5), t.text_dim);
                let clip = Rect::from_min_max(pos2(x, r.top()), pos2(r.right() - 8.0, r.bottom()));
                pt.with_clip_rect(clip).galley(pos2(x, r.center().y + 2.0), name, t.text_dim);
                if resp.clicked() {
                    picked = Some(p.id.clone());
                }
                let tip = format!("{}\n{}", if p.brand.is_empty() { &p.name } else { &p.brand }, port_summary(p));
                resp.on_hover_text(tip.trim_end());
            }
        },
    );
    ui.ctx().data_mut(|d| d.insert_temp(id, f));
    if let Some(id) = picked {
        let was_active = placing && app.session.csd_armed.as_deref() == Some(id.as_str());
        if was_active {
            app.session.cancel();
        } else if app.run("csd.arm", json!({ "product": id })).is_ok() {
            app.cmd.buffer.clear();
            app.start("csd.place");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_case_insensitive_on_id_name_and_code_and_the_category_narrows() {
        let lib = cadcraft_csd::sample_products();
        let hit = |query: &str, category: &str| {
            let f = Filter { query: query.into(), category: category.into() };
            lib.iter().filter(|p| matches(p, &f, &f.query.to_lowercase())).count()
        };
        assert_eq!(hit("", ""), lib.len());
        let first = &lib[0];
        assert!(hit(&first.code.to_uppercase(), "") >= 1);
        assert!(hit(&first.name.to_lowercase(), "") >= 1);
        assert!(hit(&first.id, &first.cat) >= 1);
        assert_eq!(hit(&first.id, "no such category"), 0);
        assert_eq!(hit("zzzz-nothing", ""), 0);
    }

    #[test]
    fn port_summary_lists_each_protocol_with_ports() {
        let mut p = Product::default();
        assert_eq!(port_summary(&p), "");
        p.counts[Protocol::Dmx as usize] = (1, 1);
        p.counts[Protocol::Network as usize] = (0, 2);
        assert_eq!(port_summary(&p), "PoE 2 out, DMX 1 in / 1 out");
    }
}
