//! The Lighting group of the Drafting tool set: the product library as a searchable list. Clicking
//! a product arms it and starts the interactive `csd.place`, which keeps placing until Enter or Esc.

use cadcraft_csd::{Product, Protocol};
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;

const ROW_H: f32 = 36.0;
const LIST_H: f32 = 252.0;

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

/// The group's body. The header is the caller's.
pub(super) fn body(app: &mut CadApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let id = ui.scope_id().with("csd_lighting");
    let mut f = ui.ctx().data_mut(|d| d.get_temp::<Filter>(id)).unwrap_or_default();
    let library = app.session.csd_library.clone();
    let mut cats: Vec<&str> = Vec::new();
    for p in library.iter() {
        if !cats.contains(&p.cat.as_str()) {
            cats.push(&p.cat);
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        if ui.button(crate::tl!("Connect")).clicked() {
            app.cmd.buffer.clear();
            app.start("csd.connect");
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut f.query).hint_text(crate::tl!("Search products")).desired_width(ui.available_width() - 8.0));
    });
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let shown = if f.category.is_empty() { crate::tl!("All").to_string() } else { f.category.clone() };
        craft_ui::ComboBox::from_id_salt(id.with("cat")).selected_text(shown).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut f.category, String::new(), crate::tl!("All"));
            for c in &cats {
                ui.selectable_value(&mut f.category, c.to_string(), *c);
            }
        });
    });
    ui.add_space(4.0);
    let needle = f.query.trim().to_lowercase();
    let hits: Vec<&Product> = library.iter().filter(|p| matches(p, &f, &needle)).collect();
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
                let sw = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 9.0), vec2(26.0, 18.0));
                pt.rect_filled(sw, 2.0, rgb(p.color));
                pt.rect_stroke(sw, 2.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let x = sw.right() + 8.0;
                pt.text(pos2(x, r.center().y - 7.0), egui::Align2::LEFT_CENTER, &p.code, egui::FontId::proportional(12.5), t.text);
                let name = pt.layout_no_wrap(p.name.clone(), egui::FontId::proportional(11.0), t.text_dim);
                let clip = Rect::from_min_max(pos2(x, r.top()), pos2(r.right() - 8.0, r.bottom()));
                pt.with_clip_rect(clip).galley(pos2(x, r.center().y + 1.0), name, t.text_dim);
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
