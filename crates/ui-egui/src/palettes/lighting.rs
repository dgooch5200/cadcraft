//! The Lighting group of the Drafting tool set: the product library as a searchable list. Clicking
//! a product arms it and starts the interactive `csd.place`, which keeps placing until Enter or Esc.

use std::collections::BTreeSet;

use cadcraft_csd::{Product, Protocol, builtin_products, product_key, split_category};
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::CadApp;
use crate::icons::Icon;
use crate::theme::Tokens;

const ROW_H: f32 = 46.0;
const LIST_H: f32 = 180.0;

#[derive(Clone, Default)]
struct Filter {
    query: String,
}

fn matches(p: &Product, needle: &str) -> bool {
    needle.is_empty() || [&p.id, &p.name, &p.code].iter().any(|s| s.to_lowercase().contains(needle))
}

type Folders<'a> = Vec<(String, Vec<(String, Vec<&'a Product>)>)>;

/// Family folders in first-seen order, each with its sub-category folders (the family's own products
/// under an empty name), from the products' `Family — Category` text.
fn folders<'a>(products: &[&'a Product]) -> Folders<'a> {
    let mut out: Folders<'a> = Vec::new();
    for p in products {
        let (family, sub) = split_category(&p.cat);
        let fi = out.iter().position(|f| f.0 == family).unwrap_or_else(|| {
            out.push((family, Vec::new()));
            out.len() - 1
        });
        let Some(subs) = out.get_mut(fi).map(|f| &mut f.1) else { continue };
        let si = subs.iter().position(|s| s.0 == sub).unwrap_or_else(|| {
            subs.push((sub, Vec::new()));
            subs.len() - 1
        });
        if let Some(s) = subs.get_mut(si) {
            s.1.push(p);
        }
    }
    out
}

/// One product row; true when clicked.
fn product_row(ui: &mut egui::Ui, t: &Tokens, p: &Product, active: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
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
    let clicked = resp.clicked();
    let tip = format!("{}\n{}", if p.brand.is_empty() { &p.name } else { &p.brand }, port_summary(p));
    resp.on_hover_text(tip.trim_end());
    clicked
}

/// The products the palette offers: the library, then the built-ins (in no library), minus the hidden.
fn listed<'a>(library: &'a [Product], hidden: &BTreeSet<String>) -> Vec<&'a Product> {
    library.iter().chain(builtin_products()).filter(|p| !hidden.contains(&product_key(&p.id))).collect()
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
    let all = listed(&library, &app.ui.csd_hidden);
    toolbar(app, ui, &t);
    ui.add_space(4.0);
    // The file actions share the search row: a row of their own pushes the Tool Sets below down.
    let mut file = None;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut f.query).hint_text(crate::tl!("Search products")).desired_width(ui.available_width() - 72.0))
            .accessible_name(crate::tl!("Search products"));
        ui.menu_button(crate::tl!("Project"), |ui| {
            for (label, cmd) in [
                ("Import .cpsd…", "csd.import.cpsd"),
                ("Import Luminary…", "csd.import.lpro"),
                ("Export .cpsd…", "csd.export.cpsd"),
                ("Print sheet…", "csd.print"),
            ] {
                if ui.button(crate::i18n::t(label)).clicked() {
                    file = Some(cmd);
                    ui.close();
                }
            }
        });
    });
    if let Some(cmd) = file {
        let _ = app.run(cmd, serde_json::Value::Null);
    }
    ui.add_space(4.0);
    let needle = f.query.trim().to_lowercase();
    let hits: Vec<&Product> = all.iter().copied().filter(|p| matches(p, &needle)).collect();
    let searching = !needle.is_empty();
    let placing = app.session.running.as_ref().is_some_and(|r| r.id == "csd.place");
    let armed = if placing { app.session.csd_armed.clone() } else { None };
    let mut picked = None;
    egui::ScrollArea::vertical().id_salt(id.with("list")).max_height(LIST_H).auto_shrink([false, true]).show(ui, |ui| {
        // A search opens every folder, so a hit is never inside a closed one.
        let open = searching.then_some(true);
        let mut rows = |ui: &mut egui::Ui, products: &[&Product]| {
            for p in products {
                if product_row(ui, &t, p, armed.as_deref() == Some(p.id.as_str())) {
                    picked = Some(p.id.clone());
                }
            }
        };
        for (family, subs) in folders(&hits) {
            egui::CollapsingHeader::new(&family).id_salt(id.with(("family", &family))).open(open).show(ui, |ui| {
                for (sub, products) in &subs {
                    if sub.is_empty() {
                        rows(ui, products);
                    } else {
                        egui::CollapsingHeader::new(sub).id_salt(id.with(("sub", &family, sub))).open(open).show(ui, |ui| rows(ui, products));
                    }
                }
            });
        }
    });
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
    fn search_is_case_insensitive_on_id_name_and_code() {
        let lib = cadcraft_csd::sample_products();
        let hit = |query: &str| lib.iter().filter(|p| matches(p, &query.to_lowercase())).count();
        assert_eq!(hit(""), lib.len());
        let first = &lib[0];
        assert!(hit(&first.code.to_uppercase()) >= 1);
        assert!(hit(&first.name.to_lowercase()) >= 1);
        assert_eq!(hit("zzzz-nothing"), 0);
    }

    #[test]
    fn products_group_into_family_then_category_folders() {
        let make = |cat: &str| Product { id: cat.into(), cat: cat.into(), ..Product::default() };
        let (a, b, c, d) = (make("Rogue — Wash"), make("Rogue — Spot"), make("Rogue"), make("Rogue — Wash"));
        let f = folders(&[&a, &b, &c, &d]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0, "Rogue");
        let subs: Vec<(&str, usize)> = f[0].1.iter().map(|s| (s.0.as_str(), s.1.len())).collect();
        assert_eq!(subs, [("Wash", 2), ("Spot", 1), ("", 1)]);
    }

    #[test]
    fn hidden_products_are_not_listed() {
        let lib = cadcraft_csd::sample_products();
        let all = listed(&lib, &BTreeSet::new()).len();
        let hidden = BTreeSet::from([product_key(&lib[0].id)]);
        let shown = listed(&lib, &hidden);
        assert_eq!(shown.len(), all - 1);
        assert!(shown.iter().all(|p| p.id != lib[0].id));
    }

    #[test]
    fn hidden_products_survive_a_restart() {
        let mut ui = crate::UiState::default();
        ui.csd_hidden.insert("rogue-r1".into());
        let back: crate::UiState = serde_json::from_str(&serde_json::to_string(&ui).unwrap()).unwrap();
        assert!(back.csd_hidden.contains("rogue-r1"));
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
