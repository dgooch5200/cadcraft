//! The product-library worksheet (CSD): an editable table over a working copy of the session's
//! library. Apply sends the copy through `csd.library.set`; Open and Save as go through the host's
//! file dialogs and `csd.library.load` / `csd.library.save`.

use std::cmp::Ordering;
use std::collections::HashSet;

use cadcraft_csd::{Product, product_key, product_record};
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};
use serde_json::{Value, json};

use crate::{CadApp, Pick};

#[derive(Default, Clone, Debug)]
pub struct State {
    pub open: bool,
    rows: Vec<Product>,
    loaded: bool,
    dirty: bool,
    query: String,
    cat: String,
    rename: String,
    sel: Option<usize>,
    sort: Option<(usize, bool)>,
    msg: Option<(bool, String)>,
    /// Products this person hides from the Lighting palette, by `product_key`. A view preference, not
    /// a product field: it is never applied, saved or exported, and placing by command ignores it.
    pub(crate) hidden: HashSet<String>,
    /// (drawings open, active drawing): a change means a drawing tab took over.
    seen: (usize, usize),
}

#[derive(Clone, Copy, PartialEq)]
enum Col {
    Show,
    Id,
    Name,
    Code,
    Cat,
    Brand,
    Color,
    PoeW,
    PoePortW,
    Port(usize, bool),
    Modes,
    Channels,
    Universes,
    Notes,
}

const PROTOCOLS: [&str; 10] = ["PoE", "Net", "10S", "DMX", "0-10V", "Cont", "PWR", "MIDI", "EPIX3", "EPIX4"];

fn columns() -> Vec<(String, f32, Col)> {
    let mut c: Vec<(String, f32, Col)> = vec![
        ("Hidden".into(), 56.0, Col::Show),
        ("Model ID".into(), 110.0, Col::Id),
        ("Product name".into(), 190.0, Col::Name),
        ("Code".into(), 80.0, Col::Code),
        ("Category".into(), 130.0, Col::Cat),
        ("Brand".into(), 100.0, Col::Brand),
        ("Color".into(), 100.0, Col::Color),
        ("PoE W draw".into(), 70.0, Col::PoeW),
        ("PoE W/port".into(), 70.0, Col::PoePortW),
    ];
    for (i, p) in PROTOCOLS.iter().enumerate() {
        c.push((format!("{p} in"), 52.0, Col::Port(i, false)));
        c.push((format!("{p} out"), 52.0, Col::Port(i, true)));
    }
    c.extend([
        ("Modes".into(), 170.0, Col::Modes),
        ("Channels".into(), 70.0, Col::Channels),
        ("Universes".into(), 70.0, Col::Universes),
        ("Notes".into(), 220.0, Col::Notes),
    ]);
    c
}

fn compare(a: &Product, b: &Product, col: Col) -> Ordering {
    let text = |p: &Product| match col {
        Col::Id => p.id.to_lowercase(),
        Col::Name => p.name.to_lowercase(),
        Col::Code => p.code.to_lowercase(),
        Col::Cat => p.cat.to_lowercase(),
        Col::Brand => p.brand.to_lowercase(),
        Col::Modes => p.modes.to_lowercase(),
        Col::Notes => p.notes.to_lowercase(),
        _ => String::new(),
    };
    let num = |p: &Product| match col {
        Col::Color => f64::from(p.color),
        Col::PoeW => p.poe_watts,
        Col::PoePortW => p.poe_port_watts,
        Col::Port(i, out) => p.counts.get(i).map_or(0.0, |c| f64::from(if out { c.1 } else { c.0 })),
        Col::Channels => f64::from(p.channels),
        Col::Universes => f64::from(p.universes),
        _ => 0.0,
    };
    text(a).cmp(&text(b)).then(num(a).total_cmp(&num(b)))
}

/// Problems that block Apply, by row index.
fn problems(rows: &[Product]) -> Vec<(usize, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (i, p) in rows.iter().enumerate() {
        let id = p.id.trim();
        if id.is_empty() {
            out.push((i, format!("row {}: id is empty", i + 1)));
        } else if !seen.insert(id) {
            out.push((i, format!("row {}: id \"{id}\" is used twice", i + 1)));
        }
        if p.name.trim().is_empty() {
            out.push((i, format!("row {}: name is empty", i + 1)));
        }
    }
    out
}

/// A new product with an id no row has.
fn new_product(rows: &[Product]) -> Product {
    let free = |id: &str| !rows.iter().any(|p| p.id == id);
    let id = std::iter::once("new-product".to_string()).chain((2..).map(|n| format!("new-product-{n}"))).find(|id| free(id)).unwrap_or_default();
    Product { name: "New product".into(), code: id.clone(), cat: "Uncategorized".into(), color: 0x555555, id, ..Product::default() }
}

/// A copy of `p` under a free id, ready to edit.
fn duplicate(rows: &[Product], p: &Product) -> Product {
    let free = |id: &str| !rows.iter().any(|r| r.id == id);
    let id = std::iter::once(format!("{}-copy", p.id)).chain((2..).map(|n| format!("{}-copy-{n}", p.id))).find(|id| free(id)).unwrap_or_default();
    Product { name: format!("{} copy", p.name), code: id.clone(), id, ..p.clone() }
}

/// The working copy as `csd.library.set` records; `img` and the other extra keys stay out (the
/// command keeps the old product's).
fn records(rows: &[Product]) -> Vec<Value> {
    rows.iter().map(|p| Value::Object(product_record(&Product { extra: Default::default(), ..p.clone() }))).collect()
}

fn reload(app: &mut CadApp) {
    let st = &mut app.ui.csd_lib;
    st.rows = app.session.csd_library.to_vec();
    st.dirty = false;
    st.sel = None;
    st.loaded = true;
}

pub fn route(app: &mut CadApp, id: &str) -> Option<Result<Value, String>> {
    (id == "ui.csd.library").then(|| {
        app.ui.csd_lib.open = true;
        app.ui.csd_lib.seen = (app.session.docs.len(), app.session.active);
        Ok(Value::Null)
    })
}

/// Whether the worksheet replaces the central area. A drawing opened or switched to closes it.
pub fn active(app: &mut CadApp) -> bool {
    let now = (app.session.docs.len(), app.session.active);
    let st = &mut app.ui.csd_lib;
    if st.seen != now {
        st.seen = now;
        st.open = false;
    }
    st.open
}

fn apply(app: &mut CadApp) {
    let bad = problems(&app.ui.csd_lib.rows);
    if !bad.is_empty() {
        app.ui.csd_lib.msg = Some((false, "Not applied: fix the highlighted rows.".into()));
        return;
    }
    let params = json!({ "products": records(&app.ui.csd_lib.rows) });
    match app.run("csd.library.set", params) {
        Ok(v) => {
            reload(app);
            app.ui.csd_lib.msg = Some((true, format!("Applied {} products. Cards already placed keep their old look.", v["count"])));
        }
        Err(e) => app.ui.csd_lib.msg = Some((false, e)),
    }
}

/// A library file dialog: `cmd` is the engine command run on the chosen path.
fn pick(app: &mut CadApp, cmd: &'static str, save: bool, file: &str) {
    let then = move |app: &mut CadApp, path: String| {
        let done = if save { "Saved" } else { "Loaded" };
        if save && app.ui.csd_lib.dirty {
            app.ui.csd_lib.msg = Some((false, "Apply or Revert first: the file holds the applied library.".into()));
            return Ok(Value::Null);
        }
        match app.run(cmd, json!({ "path": path })) {
            Ok(v) => {
                if !save {
                    reload(app);
                }
                app.ui.csd_lib.msg = Some((true, format!("{done} {} products: {path}", v["count"])));
            }
            Err(e) => app.ui.csd_lib.msg = Some((false, e)),
        }
        Ok(Value::Null)
    };
    let kind = if save { Pick::Save(file.into()) } else { Pick::Open };
    if let Some(Err(e)) = app.pick_then(kind, then) {
        app.ui.csd_lib.msg = Some((false, e));
    }
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    if !app.ui.csd_lib.loaded {
        reload(app);
    }
    let can_pick = !cfg!(target_arch = "wasm32") && app.services.pick_open.is_some();
    let bad = problems(&app.ui.csd_lib.rows);
    let bad_rows: HashSet<usize> = bad.iter().map(|b| b.0).collect();
    let (mut do_apply, mut do_revert, mut do_open, mut do_save) = (false, false, false, false);
    let (mut do_import, mut do_export) = (false, false);
    let st = &mut app.ui.csd_lib;
    let mut cats: Vec<String> = st.rows.iter().map(|p| p.cat.clone()).collect::<HashSet<_>>().into_iter().collect();
    cats.sort();

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Filter products…").desired_width(200.0)).accessible_name("Filter products");
        craft_ui::ComboBox::from_id_salt("csd_lib_cat")
            .selected_text(if st.cat.is_empty() { "All categories" } else { &st.cat })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut st.cat, String::new(), "All categories");
                for c in &cats {
                    ui.selectable_value(&mut st.cat, c.clone(), c);
                }
            })
            .response
            .accessible_name("Category");
        if ui.button("Add product").clicked() {
            let p = new_product(&st.rows);
            st.rows.push(p);
            st.sel = Some(st.rows.len() - 1);
            st.dirty = true;
            st.query.clear();
            st.cat.clear();
        }
        if ui.add_enabled(st.sel.is_some(), egui::Button::new("Duplicate selected")).clicked()
            && let Some(p) = st.sel.and_then(|i| st.rows.get(i)).map(|p| duplicate(&st.rows, p))
        {
            st.rows.push(p);
            st.sel = Some(st.rows.len() - 1);
            st.dirty = true;
            st.query.clear();
            st.cat.clear();
        }
        if ui.add_enabled(st.sel.is_some(), egui::Button::new("Delete selected")).clicked()
            && let Some(i) = st.sel.take().filter(|i| *i < st.rows.len())
        {
            st.rows.remove(i);
            st.dirty = true;
        }
        ui.separator();
        do_apply = ui.add_enabled(st.dirty, egui::Button::new("Apply")).clicked();
        do_revert = ui.add_enabled(st.dirty, egui::Button::new("Revert")).clicked();
        if st.dirty {
            ui.label(RichText::new("modified").italics().color(Color32::from_rgb(230, 170, 60)));
        }
        ui.separator();
        do_open =
            ui.add_enabled(can_pick, egui::Button::new("Open…")).on_disabled_hover_text("Needs a host file dialog that returns a path").clicked();
        do_save = ui.add_enabled(can_pick, egui::Button::new("Save as…")).clicked();
        do_import = ui
            .add_enabled(can_pick, egui::Button::new("Import CSV…"))
            .on_disabled_hover_text("Needs a host file dialog that returns a path")
            .clicked();
        do_export = ui.add_enabled(can_pick, egui::Button::new("Export CSV…")).clicked();
        ui.label(RichText::new(format!("{} products", st.rows.len())).weak());
    });
    if !st.cat.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label("Rename this category to");
            ui.add(egui::TextEdit::singleline(&mut st.rename).desired_width(180.0)).accessible_name("New category name");
            if ui.add_enabled(!st.rename.trim().is_empty(), egui::Button::new("Rename")).clicked() {
                let to = st.rename.trim().to_string();
                for p in st.rows.iter_mut().filter(|p| p.cat == st.cat) {
                    p.cat = to.clone();
                }
                st.cat = to;
                st.rename.clear();
                st.dirty = true;
            }
        });
    }
    if let Some((ok, m)) = &st.msg {
        let c = if *ok { ui.visuals().text_color() } else { ui.visuals().error_fg_color };
        ui.label(RichText::new(m).color(c));
    }
    for (_, m) in bad.iter().take(3) {
        ui.label(RichText::new(m).color(ui.visuals().error_fg_color));
    }

    let q = st.query.to_lowercase();
    let mut view: Vec<usize> = (0..st.rows.len())
        .filter(|i| {
            let p = &st.rows[*i];
            (st.cat.is_empty() || p.cat == st.cat) && (q.is_empty() || [&p.id, &p.name, &p.code].iter().any(|s| s.to_lowercase().contains(&q)))
        })
        .collect();
    let cols = columns();
    if let Some((c, asc)) = st.sort
        && let Some((_, _, col)) = cols.get(c)
    {
        view.sort_by(|a, b| {
            let o = compare(&st.rows[*a], &st.rows[*b], *col);
            if asc { o } else { o.reverse() }
        });
    }

    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        let mut t = TableBuilder::new(ui).striped(true).auto_shrink([false, false]).max_scroll_height(f32::INFINITY).column(Column::exact(28.0));
        for (_, w, _) in &cols {
            t = t.column(Column::exact(*w));
        }
        t.header(row_h + 4.0, |mut h| {
            h.col(|_| {});
            for (i, (label, _, _)) in cols.iter().enumerate() {
                h.col(|ui| {
                    let arrow = match st.sort {
                        Some((c, true)) if c == i => " ▲",
                        Some((c, false)) if c == i => " ▼",
                        _ => "",
                    };
                    if ui.add(egui::Label::new(RichText::new(format!("{label}{arrow}")).strong()).sense(egui::Sense::click())).clicked() {
                        st.sort = match st.sort {
                            Some((c, true)) if c == i => Some((i, false)),
                            Some((c, false)) if c == i => None,
                            _ => Some((i, true)),
                        };
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, view.len(), |mut r| {
                let Some(&ri) = view.get(r.index()) else { return };
                r.col(|ui| {
                    if ui.selectable_label(st.sel == Some(ri), "▸").clicked() {
                        st.sel = Some(ri);
                    }
                });
                for (_, _, col) in &cols {
                    r.col(|ui| {
                        let Some(p) = st.rows.get_mut(ri) else { return };
                        if *col == Col::Show {
                            let mut hide = st.hidden.contains(&product_key(&p.id));
                            if ui.checkbox(&mut hide, "").on_hover_text("Hide from the Lighting palette").changed() {
                                let key = product_key(&p.id);
                                if hide {
                                    st.hidden.insert(key)
                                } else {
                                    st.hidden.remove(&key)
                                };
                            }
                            return;
                        }
                        if cell(ui, p, *col, ri, bad_rows.contains(&ri)) {
                            st.dirty = true;
                        }
                    });
                }
            });
        });
    });

    if do_apply {
        apply(app);
    }
    if do_revert {
        reload(app);
        app.ui.csd_lib.msg = Some((true, "Reverted to the applied library.".into()));
    }
    if do_open {
        pick(app, "csd.library.load", false, "");
    }
    if do_save {
        pick(app, "csd.library.save", true, "library.json");
    }
    if do_import {
        pick(app, "csd.library.import", false, "");
    }
    if do_export {
        pick(app, "csd.library.export", true, "library.csv");
    }
}

/// One editable cell; true when the value changed.
fn cell(ui: &mut egui::Ui, p: &mut Product, col: Col, row: usize, flagged: bool) -> bool {
    let w = ui.available_width();
    let text = |ui: &mut egui::Ui, s: &mut String, warn: bool| {
        let mut e = egui::TextEdit::singleline(s).desired_width(w);
        if warn {
            e = e.text_color(ui.visuals().error_fg_color);
        }
        ui.add(e).changed()
    };
    match col {
        Col::Show => false,
        Col::Id => text(ui, &mut p.id, flagged),
        Col::Name => {
            let empty = p.name.trim().is_empty();
            text(ui, &mut p.name, empty)
        }
        Col::Code => text(ui, &mut p.code, false),
        Col::Cat => text(ui, &mut p.cat, false),
        Col::Brand => text(ui, &mut p.brand, false),
        Col::Modes => text(ui, &mut p.modes, false),
        Col::Notes => text(ui, &mut p.notes, false),
        Col::Color => color_cell(ui, p, row),
        Col::PoeW => ui.add(egui::DragValue::new(&mut p.poe_watts).range(0.0..=1e6).speed(0.5)).changed(),
        Col::PoePortW => ui.add(egui::DragValue::new(&mut p.poe_port_watts).range(0.0..=1e6).speed(0.5)).changed(),
        Col::Channels => ui.add(egui::DragValue::new(&mut p.channels).range(0..=1_000_000)).changed(),
        Col::Universes => ui.add(egui::DragValue::new(&mut p.universes).range(0..=1_000_000)).changed(),
        Col::Port(i, out) => match p.counts.get_mut(i) {
            Some(c) => ui.add(egui::DragValue::new(if out { &mut c.1 } else { &mut c.0 }).range(0..=1000)).changed(),
            None => false,
        },
    }
}

/// Swatch plus hex text; the text keeps what is typed until focus leaves, so half a value survives.
fn color_cell(ui: &mut egui::Ui, p: &mut Product, row: usize) -> bool {
    let id = ui.scope_id().with(("csd_hex", row));
    let (r, g, b) = ((p.color >> 16) as u8, (p.color >> 8) as u8, p.color as u8);
    let mut changed = false;
    ui.horizontal(|ui| {
        let (sw, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(sw, 2.0, Color32::from_rgb(r, g, b));
        let mut s = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| format!("{:06X}", p.color));
        let resp = ui.add(egui::TextEdit::singleline(&mut s).desired_width(ui.available_width()).char_limit(7));
        if resp.changed() {
            if let Ok(v) = u32::from_str_radix(s.trim().trim_start_matches('#'), 16)
                && s.trim().trim_start_matches('#').len() == 6
            {
                p.color = v;
                changed = true;
            }
            ui.data_mut(|d| d.insert_temp(id, s));
        } else if !resp.has_focus() {
            ui.data_mut(|d| d.remove_temp::<String>(id));
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> CadApp {
        CadApp::new(cadcraft_engine::Session::empty(), crate::Services::default())
    }

    #[test]
    fn duplicate_ids_and_empty_names_block_apply() {
        let mut app = app();
        reload(&mut app);
        let dup = app.ui.csd_lib.rows[0].clone();
        app.ui.csd_lib.rows.push(dup);
        app.ui.csd_lib.rows[1].name.clear();
        let before = app.session.csd_library.len();
        apply(&mut app);
        assert_eq!(app.session.csd_library.len(), before);
        assert_eq!(app.ui.csd_lib.msg.as_ref().map(|m| m.0), Some(false));
        assert_eq!(problems(&app.ui.csd_lib.rows).len(), 2);
    }

    #[test]
    fn duplicate_gets_a_free_id() {
        let mut app = app();
        reload(&mut app);
        let rows = &mut app.ui.csd_lib.rows;
        let a = duplicate(rows, &rows[0]);
        assert_ne!(a.id, rows[0].id);
        rows.push(a.clone());
        assert_ne!(duplicate(rows, &rows[0]).id, a.id);
        assert!(problems(rows).is_empty());
    }

    #[test]
    fn apply_replaces_the_session_library_and_the_command_opens_the_tab() {
        let mut app = app();
        app.run("ui.csd.library", Value::Null).unwrap();
        assert!(active(&mut app));
        reload(&mut app);
        let p = new_product(&app.ui.csd_lib.rows);
        let id = p.id.clone();
        app.ui.csd_lib.rows.push(p);
        app.ui.csd_lib.dirty = true;
        apply(&mut app);
        assert!(app.session.csd_library.iter().any(|p| p.id == id), "{:?}", app.ui.csd_lib.msg);
        assert!(!app.ui.csd_lib.dirty);
    }
}
