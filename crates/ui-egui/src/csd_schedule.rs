//! The lighting designer's bottom panel: Placed devices, Unplaced and Bill of materials. Every
//! list is the engine's (`csd.schedule`, `csd.unplaced.list`, `csd.bom`), read once per drawing
//! revision; every button runs a command.

use std::cmp::Ordering;
use std::sync::Arc;

use egui::{Align, Layout, RichText};
use egui_extras::{Column, TableBuilder, TableRow};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{CadApp, Pick};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Tab {
    #[default]
    Placed,
    Unplaced,
    Bom,
    Patch,
}

/// The patch orders `csd.patch` takes, with their labels.
const ORDERS: [(&str, &str); 4] = [("drawing", "Drawing order"), ("name", "Name"), ("product", "Product"), ("position", "Position")];

#[derive(Clone, Debug, Default)]
pub struct State {
    pub open: bool,
    tab: Tab,
    /// Index into [`ORDERS`].
    order: usize,
    /// Placed devices: (column, ascending).
    sort: Option<(usize, bool)>,
    add_query: String,
    msg: Option<(bool, String)>,
    cache: Option<((u64, u64), Arc<Lists>)>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Device {
    handle: String,
    name: String,
    category: String,
    code: String,
    label: String,
    notes: String,
    #[serde(rename = "portsInUse")]
    ports: String,
    connections: u64,
    #[serde(rename = "poeLoad")]
    poe: f64,
    #[serde(rename = "poeWarning", default)]
    poe_warning: Option<String>,
    #[serde(rename = "controlGear")]
    control: bool,
    #[serde(rename = "mountedIn")]
    mounted_in: String,
    #[serde(rename = "supplyByOthers")]
    by_others: bool,
    accessories: Vec<Accessory>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Accessory {
    code: String,
    qty: u64,
}

impl Device {
    fn accessory_count(&self) -> u64 {
        self.accessories.iter().map(|a| a.qty).sum()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Item {
    index: usize,
    product: String,
    qty: u64,
    code: String,
    category: String,
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Line {
    qty: u64,
    product: String,
    code: String,
    category: String,
    name: String,
    labels: Vec<String>,
    notes: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PatchRow {
    handle: String,
    label: String,
    name: String,
    mode: Option<String>,
    #[serde(rename = "assumedMode")]
    assumed: String,
    modes: Vec<String>,
    channels: u64,
    universe: u64,
    address: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Budget {
    used: u64,
    budget: u64,
    over: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Patch {
    rows: Vec<PatchRow>,
    universes: Budget,
}

#[derive(Debug, Default)]
struct Lists {
    devices: Vec<Device>,
    items: Vec<Item>,
    lines: Vec<Line>,
    patch: Patch,
}

enum Act {
    SetMode(String, String),
    SetOrder(usize),
    ExportPatch(&'static str),
    Locate(String),
    DeleteDevice(String),
    Place(usize),
    Remove(usize),
    SetQty(usize, u64),
    Add(String),
    Export(&'static str),
    Close,
}

const MAX_QTY: u64 = 100_000;

pub fn route(app: &mut CadApp, id: &str) -> Option<Result<Value, String>> {
    (id == "ui.csd.schedule").then(|| {
        app.ui.csd_sched.open = !app.ui.csd_sched.open;
        Ok(Value::Null)
    })
}

fn load<T: DeserializeOwned>(app: &mut CadApp, cmd: &str, key: &str) -> Vec<T> {
    let v = app.session.execute(cmd, &json!({})).unwrap_or(Value::Null);
    serde_json::from_value(v.get(key).cloned().unwrap_or(Value::Null)).unwrap_or_default()
}

/// The three lists, worked out once per drawing revision.
fn lists(app: &mut CadApp) -> Arc<Lists> {
    let Some(key) = app.session.state().ok().map(|s| (s.uid, s.revision)) else { return Arc::default() };
    if let Some((k, l)) = &app.ui.csd_sched.cache
        && *k == key
    {
        return l.clone();
    }
    let l = Arc::new(Lists {
        devices: load(app, "csd.schedule", "devices"),
        items: load(app, "csd.unplaced.list", "items"),
        lines: load(app, "csd.bom", "rows"),
        patch: {
            let order = ORDERS.get(app.ui.csd_sched.order).map_or("drawing", |o| o.0);
            let v = app.session.execute("csd.patch", &json!({ "order": order })).unwrap_or(Value::Null);
            serde_json::from_value(v).unwrap_or_default()
        },
    });
    app.ui.csd_sched.cache = Some((key, l.clone()));
    l
}

fn compare(a: &Device, b: &Device, col: usize) -> Ordering {
    let text = |d: &Device| {
        match col {
            1 => &d.name,
            2 => &d.category,
            3 => &d.code,
            4 => &d.label,
            5 => &d.ports,
            8 => &d.notes,
            9 => &d.mounted_in,
            _ => &d.handle,
        }
        .to_lowercase()
    };
    match col {
        6 => a.connections.cmp(&b.connections),
        7 => a.poe.total_cmp(&b.poe),
        10 => a.by_others.cmp(&b.by_others),
        11 => a.accessory_count().cmp(&b.accessory_count()),
        _ => text(a).cmp(&text(b)),
    }
}

fn builder<'a>(ui: &'a mut egui::Ui, widths: &[f32]) -> TableBuilder<'a> {
    let mut t = TableBuilder::new(ui).striped(true).auto_shrink([false, false]).max_scroll_height(f32::INFINITY);
    for (i, w) in widths.iter().enumerate() {
        // The last column takes what is left.
        t = t.column(if i + 1 == widths.len() { Column::remainder().at_least(*w) } else { Column::exact(*w) });
    }
    t
}

fn head(h: &mut TableRow, labels: &[&str]) {
    for l in labels {
        h.col(|ui| {
            ui.label(RichText::new(crate::i18n::t(l)).strong());
        });
    }
}

fn cell(r: &mut TableRow, text: &str) {
    r.col(|ui| {
        ui.add(egui::Label::new(text).truncate());
    });
}

fn placed_tab(ui: &mut egui::Ui, st: &mut State, l: &Lists, act: &mut Option<Act>) {
    if l.devices.is_empty() {
        ui.label(RichText::new(crate::tl!("No devices placed yet.")).weak());
        return;
    }
    // Control gear stays first whatever the sort.
    let mut view: Vec<&Device> = l.devices.iter().collect();
    if let Some((c, asc)) = st.sort {
        view.sort_by(|a, b| (!a.control).cmp(&!b.control).then_with(|| if asc { compare(a, b, c) } else { compare(a, b, c).reverse() }));
    }
    const LABELS: [&str; 13] = [
        "#",
        "Product",
        "Category",
        "Code",
        "Label",
        "Ports in use",
        "Conns",
        "PoE load",
        "Notes",
        "Mounted in",
        "By others",
        "Accessories",
        "Actions",
    ];
    const LAST: usize = LABELS.len() - 1;
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    builder(ui, &[30.0, 200.0, 120.0, 90.0, 110.0, 150.0, 50.0, 70.0, 160.0, 120.0, 70.0, 90.0, 150.0])
        .header(row_h + 4.0, |mut h| {
            for (i, label) in LABELS.iter().enumerate() {
                h.col(|ui| {
                    let arrow = match st.sort {
                        Some((c, true)) if c == i => " ▲",
                        Some((c, false)) if c == i => " ▼",
                        _ => "",
                    };
                    let text = RichText::new(format!("{}{arrow}", crate::i18n::t(label))).strong();
                    if i > 0 && i < LAST && ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                        st.sort = match st.sort {
                            Some((c, true)) if c == i => Some((i, false)),
                            Some((c, false)) if c == i => None,
                            _ => Some((i, true)),
                        };
                    } else if i == 0 || i == LAST {
                        ui.label(RichText::new(crate::i18n::t(label)).strong());
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, view.len(), |mut r| {
                let Some(d) = view.get(r.index()) else { return };
                let n = r.index() + 1;
                cell(&mut r, &n.to_string());
                let poe = if d.poe > 0.0 { format!("{:.0} W", d.poe) } else { String::new() };
                for t in [&d.name, &d.category, &d.code, &d.label, &d.ports, &d.connections.to_string()] {
                    cell(&mut r, t);
                }
                r.col(|ui| {
                    let text = match &d.poe_warning {
                        Some(_) => format!("\u{26a0} {poe}"),
                        None => poe.clone(),
                    };
                    let l = ui.label(text);
                    if let Some(w) = &d.poe_warning {
                        l.on_hover_text(w);
                    }
                });
                cell(&mut r, &d.notes);
                cell(&mut r, &d.mounted_in);
                cell(&mut r, if d.by_others { "✓" } else { "" });
                r.col(|ui| {
                    if d.accessories.is_empty() {
                        return;
                    }
                    let names = d.accessories.iter().map(|a| format!("{} x {}", a.qty, a.code)).collect::<Vec<_>>().join("\n");
                    ui.label(d.accessory_count().to_string()).on_hover_text(names);
                });
                r.col(|ui| {
                    if ui.small_button(crate::tl!("Locate")).clicked() {
                        *act = Some(Act::Locate(d.handle.clone()));
                    }
                    if ui.small_button(crate::tl!("Delete")).clicked() {
                        *act = Some(Act::DeleteDevice(d.handle.clone()));
                    }
                });
            });
        });
}

fn unplaced_tab(ui: &mut egui::Ui, st: &mut State, l: &Lists, library: &[cadcraft_csd::Product], act: &mut Option<Act>) {
    ui.horizontal(|ui| {
        craft_ui::ComboBox::from_id_salt("csd_unplaced_add")
            .selected_text(crate::tl!("Add a product…"))
            .width(260.0)
            .show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut st.add_query).hint_text(crate::tl!("Search products")));
                let needle = st.add_query.trim().to_lowercase();
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for p in
                        library.iter().filter(|p| needle.is_empty() || [&p.id, &p.name, &p.code].iter().any(|s| s.to_lowercase().contains(&needle)))
                    {
                        if ui.selectable_label(false, format!("{} · {}", p.code, p.name)).clicked() {
                            *act = Some(Act::Add(p.id.clone()));
                        }
                    }
                });
            })
            .response
            .accessible_name(crate::tl!("Add a product…"));
    });
    if l.items.is_empty() {
        ui.label(RichText::new(crate::tl!("Nothing unplaced.")).weak());
        return;
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
    builder(ui, &[40.0, 90.0, 150.0, 100.0, 140.0, 200.0, 150.0])
        .header(row_h + 4.0, |mut h| head(&mut h, &["Item", "Qty", "Model ID", "Code", "Category", "Product name", "Actions"]))
        .body(|body| {
            body.rows(row_h, l.items.len(), |mut r| {
                let Some(it) = l.items.get(r.index()) else { return };
                let n = r.index() + 1;
                cell(&mut r, &n.to_string());
                r.col(|ui| qty_cell(ui, it, act));
                for t in [&it.product, &it.code, &it.category, &it.name] {
                    cell(&mut r, t);
                }
                r.col(|ui| {
                    if ui.small_button(crate::tl!("Place")).clicked() {
                        *act = Some(Act::Place(it.index));
                    }
                    if ui.small_button(crate::tl!("Delete")).clicked() {
                        *act = Some(Act::Remove(it.index));
                    }
                });
            });
        });
}

/// A quantity that is committed when the drag or the typing ends, so one edit is one undo step.
fn qty_cell(ui: &mut egui::Ui, it: &Item, act: &mut Option<Act>) {
    let id = ui.scope_id().with(("unplaced_qty", it.index));
    let mut q = ui.data(|d| d.get_temp::<u64>(id)).unwrap_or(it.qty);
    let r = ui.add(egui::DragValue::new(&mut q).range(1..=MAX_QTY)).accessible_name(crate::tl!("Qty"));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(id, q));
    }
    if (r.drag_stopped() || r.lost_focus()) && q != it.qty {
        *act = Some(Act::SetQty(it.index, q));
    }
    if !r.dragged() && !r.has_focus() {
        ui.data_mut(|d| d.remove_temp::<u64>(id));
    }
}

fn bom_tab(ui: &mut egui::Ui, l: &Lists) {
    if l.lines.is_empty() {
        ui.label(RichText::new(crate::tl!("Nothing to list.")).weak());
        return;
    }
    // A heading row (None) before each category.
    let mut rows: Vec<(Option<usize>, &Line)> = Vec::new();
    for (i, line) in l.lines.iter().enumerate() {
        if rows.last().is_none_or(|(_, p)| p.category != line.category) {
            rows.push((None, line));
        }
        rows.push((Some(i), line));
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    builder(ui, &[40.0, 50.0, 150.0, 100.0, 140.0, 200.0, 160.0, 160.0])
        .header(row_h + 4.0, |mut h| head(&mut h, &["Item", "Qty", "Model ID", "Code", "Category", "Product name", "Labels", "Notes"]))
        .body(|body| {
            body.rows(row_h, rows.len(), |mut r| {
                let Some((n, line)) = rows.get(r.index()) else { return };
                let Some(n) = n else {
                    cell(&mut r, "");
                    cell(&mut r, "");
                    r.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&line.category).strong()).truncate());
                    });
                    return;
                };
                for t in [
                    &(n + 1).to_string(),
                    &line.qty.to_string(),
                    &line.product,
                    &line.code,
                    &line.category,
                    &line.name,
                    &line.labels.join(", "),
                    &line.notes,
                ] {
                    cell(&mut r, t);
                }
            });
        });
}

fn patch_tab(ui: &mut egui::Ui, st: &State, l: &Lists, act: &mut Option<Act>) {
    let b = &l.patch.universes;
    ui.horizontal(|ui| {
        let line = if b.budget > 0 {
            crate::tf!("Universes: {used} of {budget}", used = b.used, budget = b.budget)
        } else {
            crate::tf!("Universes: {used}", used = b.used)
        };
        let c = if b.over { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
        ui.label(RichText::new(line).color(c));
        craft_ui::ComboBox::from_id_salt("csd_patch_order")
            .selected_text(ORDERS.get(st.order).map_or("", |o| crate::i18n::t(o.1)))
            .show_ui(ui, |ui| {
                for (i, (_, label)) in ORDERS.iter().enumerate() {
                    if ui.selectable_label(st.order == i, crate::i18n::t(label)).clicked() {
                        *act = Some(Act::SetOrder(i));
                    }
                }
            })
            .response
            .accessible_name(crate::tl!("Patch order"));
    });
    if l.patch.rows.is_empty() {
        ui.label(RichText::new(crate::tl!("Nothing to patch.")).weak());
        return;
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
    builder(ui, &[40.0, 90.0, 70.0, 150.0, 200.0, 180.0, 70.0])
        .header(row_h + 4.0, |mut h| head(&mut h, &["Item", "Universe", "Address", "Label", "Product name", "Mode", "Channels"]))
        .body(|body| {
            body.rows(row_h, l.patch.rows.len(), |mut r| {
                let Some(p) = l.patch.rows.get(r.index()) else { return };
                let n = r.index() + 1;
                for t in [&n.to_string(), &p.universe.to_string(), &p.address.to_string(), &p.label, &p.name] {
                    cell(&mut r, t);
                }
                r.col(|ui| {
                    if p.modes.is_empty() {
                        ui.label(RichText::new(&p.assumed).weak());
                        return;
                    }
                    let shown = p.mode.clone().unwrap_or_else(|| crate::tf!("{mode} (assumed)", mode = p.assumed));
                    craft_ui::ComboBox::from_id_salt(("csd_mode", &p.handle)).selected_text(shown).width(160.0).show_ui(ui, |ui| {
                        if ui.selectable_label(p.mode.is_none(), crate::tl!("Largest mode")).clicked() {
                            *act = Some(Act::SetMode(p.handle.clone(), String::new()));
                        }
                        for m in &p.modes {
                            if ui.selectable_label(p.mode.as_ref() == Some(m), m).clicked() {
                                *act = Some(Act::SetMode(p.handle.clone(), m.clone()));
                            }
                        }
                    });
                });
                cell(&mut r, &p.channels.to_string());
            });
        });
}

pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    if !app.ui.csd_sched.open {
        return;
    }
    let t = Tokens::get();
    let l = lists(app);
    let library = app.session.csd_library.clone();
    let can_pick = !cfg!(target_arch = "wasm32") && app.services.pick_save.is_some();
    let mut act = None;
    let st = &mut app.ui.csd_sched;
    egui::Panel::bottom("csd_schedule").resizable(true).default_size(230.0).min_size(110.0).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let tabs = [
                (Tab::Placed, crate::tf!("Placed devices · {count}", count = l.devices.len())),
                (Tab::Unplaced, crate::tf!("Unplaced · {count}", count = l.items.len())),
                (Tab::Bom, crate::tf!("Bill of materials · {count}", count = l.lines.len())),
                (Tab::Patch, crate::tf!("Patch · {count}", count = l.patch.rows.len())),
            ];
            for (tab, label) in tabs {
                if ui.selectable_label(st.tab == tab, label).clicked() {
                    st.tab = tab;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button(crate::tl!("Close")).clicked() {
                    act = Some(Act::Close);
                }
                if st.tab == Tab::Bom {
                    for (label, fmt) in [(crate::tl!("Export PDF"), "pdf"), (crate::tl!("Export CSV"), "csv")] {
                        if ui.add_enabled(can_pick && !l.lines.is_empty(), egui::Button::new(label)).clicked() {
                            act = Some(Act::Export(fmt));
                        }
                    }
                }
                if st.tab == Tab::Patch {
                    for (label, fmt) in [(crate::tl!("Export MagicQ"), "magicq"), (crate::tl!("Export CSV"), "csv")] {
                        if ui.add_enabled(can_pick && !l.patch.rows.is_empty(), egui::Button::new(label)).clicked() {
                            act = Some(Act::ExportPatch(fmt));
                        }
                    }
                }
                if let Some((ok, m)) = &st.msg {
                    let c = if *ok { ui.visuals().text_color() } else { ui.visuals().error_fg_color };
                    ui.label(RichText::new(m).color(c));
                }
            });
        });
        ui.separator();
        match st.tab {
            Tab::Placed => placed_tab(ui, st, &l, &mut act),
            Tab::Unplaced => unplaced_tab(ui, st, &l, &library, &mut act),
            Tab::Bom => bom_tab(ui, &l),
            Tab::Patch => patch_tab(ui, st, &l, &mut act),
        }
    });
    if let Some(a) = act {
        apply(app, a);
    }
}

fn say(app: &mut CadApp, r: Result<Value, String>) {
    app.ui.csd_sched.msg = r.err().map(|e| (false, e));
}

fn apply(app: &mut CadApp, act: Act) {
    match act {
        Act::Close => app.ui.csd_sched.open = false,
        Act::SetMode(h, mode) => {
            let r = app.run("csd.device.mode", json!({ "handles": [h], "mode": mode }));
            say(app, r);
        }
        Act::SetOrder(i) => {
            app.ui.csd_sched.order = i;
            app.ui.csd_sched.cache = None;
        }
        Act::ExportPatch(format) => {
            let then = move |app: &mut CadApp, path: String| {
                let order = ORDERS.get(app.ui.csd_sched.order).map_or("drawing", |o| o.0);
                app.ui.csd_sched.msg = Some(match app.run("csd.patch.export", json!({ "path": path, "format": format, "order": order })) {
                    Ok(v) => (true, crate::tf!("Exported {rows} rows: {path}", rows = v["rows"], path = path)),
                    Err(e) => (false, e),
                });
                Ok(Value::Null)
            };
            if let Some(Err(e)) = app.pick_then(Pick::Save("dmx-patch.csv".into()), then) {
                say(app, Err(e));
            }
        }
        Act::Locate(h) => {
            let r = app.run("select", json!({ "handles": [h] })).and_then(|_| app.run("zoom", json!({ "mode": "object", "handles": [h] })));
            say(app, r);
        }
        Act::DeleteDevice(h) => {
            let r = app.run("csd.device.delete", json!({ "handles": [h] }));
            say(app, r);
        }
        Act::Place(index) => match app.run("csd.unplaced.place", json!({ "index": index })) {
            Ok(_) => {
                app.ui.csd_sched.msg = None;
                app.cmd.buffer.clear();
                app.start("csd.place");
            }
            Err(e) => say(app, Err(e)),
        },
        Act::Remove(index) => {
            let r = app.run("csd.unplaced.remove", json!({ "index": index }));
            say(app, r);
        }
        Act::SetQty(index, qty) => {
            let r = app.run("csd.unplaced.set", json!({ "index": index, "qty": qty }));
            say(app, r);
        }
        Act::Add(product) => {
            let r = app.run("csd.unplaced.add", json!({ "product": product }));
            say(app, r);
        }
        Act::Export(format) => {
            let then = move |app: &mut CadApp, path: String| {
                app.ui.csd_sched.msg = Some(match app.run("csd.bom.export", json!({ "path": path, "format": format })) {
                    Ok(v) => (true, crate::tf!("Exported {rows} rows: {path}", rows = v["rows"], path = path)),
                    Err(e) => (false, e),
                });
                Ok(Value::Null)
            };
            if let Some(Err(e)) = app.pick_then(Pick::Save(format!("bill-of-materials.{format}")), then) {
                say(app, Err(e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> CadApp {
        CadApp::new(cadcraft_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn placed_rows_carry_mounting_supply_and_accessories() {
        let mut app = app();
        let rack = app.run("csd.place", json!({ "product": "CHAMMQ50", "at": [0, 0] })).unwrap();
        let node = app.run("csd.place", json!({ "product": "epix-flex-drive", "at": [300, 0] })).unwrap();
        app.run("csd.device.set", json!({ "handle": rack["handle"], "label": "Rack" })).unwrap();
        app.run(
            "csd.device.set",
            json!({ "handle": node["handle"], "mount": rack["handle"], "supplyByOthers": true, "accessories": [{ "product": "b565fc", "qty": 2 }] }),
        )
        .unwrap();
        let l = lists(&mut app);
        let d = l.devices.iter().find(|d| d.by_others).expect("the supplied device");
        assert_eq!((d.mounted_in.as_str(), d.accessory_count()), ("Rack", 2));
    }

    #[test]
    fn the_command_toggles_the_panel_and_the_lists_follow_the_drawing() {
        let mut app = app();
        app.run("ui.csd.schedule", Value::Null).unwrap();
        assert!(app.ui.csd_sched.open);
        app.run("csd.place", json!({ "product": "CHAMMQ50", "at": [0, 0] })).unwrap();
        app.run("csd.unplaced.add", json!({ "product": "b565fc", "qty": 2 })).unwrap();
        let l = lists(&mut app);
        assert_eq!((l.devices.len(), l.items.len(), l.lines.len()), (1, 1, 2));
        assert!(Arc::ptr_eq(&l, &lists(&mut app)), "same revision, same lists");
        apply(&mut app, Act::SetQty(0, 5));
        assert_eq!(lists(&mut app).items.first().map(|i| i.qty), Some(5));
        let h = lists(&mut app).devices[0].handle.clone();
        apply(&mut app, Act::Locate(h.clone()));
        assert_eq!(app.session.selection().len(), 1);
        apply(&mut app, Act::DeleteDevice(h));
        assert!(lists(&mut app).devices.is_empty());
        apply(&mut app, Act::Place(0));
        assert_eq!(app.session.csd_armed.as_deref(), Some("b565fc"));
        apply(&mut app, Act::Close);
        assert!(!app.ui.csd_sched.open);
    }

    #[test]
    fn the_patch_tab_lists_devices_and_a_mode_pick_repatches_them() {
        let mut app = app();
        app.session.csd_library = Arc::new(vec![cadcraft_csd::Product { id: "FIX".into(), modes: "Std:300, Ext:200".into(), ..Default::default() }]);
        for x in [0, 300] {
            app.run("csd.place", json!({ "product": "FIX", "at": [x, 0] })).unwrap();
        }
        let l = lists(&mut app);
        assert_eq!(l.patch.rows.iter().map(|r| (r.universe, r.address)).collect::<Vec<_>>(), [(1, 1), (2, 1)]);
        let h = l.patch.rows[0].handle.clone();
        apply(&mut app, Act::SetMode(h, "Ext".into()));
        let l = lists(&mut app);
        assert_eq!(l.patch.rows.iter().map(|r| (r.universe, r.address, r.channels)).collect::<Vec<_>>(), [(1, 1, 200), (1, 201, 300)]);
        apply(&mut app, Act::SetOrder(1));
        assert!(app.ui.csd_sched.cache.is_none());
    }
}
