//! The designer's three lists: placed devices, products not yet drawn (kept in the drawing's
//! header, so they save with it) and the bill of materials over both.

use std::collections::{BTreeMap, HashSet};

use cadcraft_csd::{Product, Protocol, WireLink, is_accessory, is_builtin, port_overload};
use cadcraft_doc::{Common, Drawing, EntityKind, HAlign, HVal, Handle, Space, Text, VAlign};
use cadcraft_geom::Vec3;
use serde_json::{Value, json};

use super::{accessories_of, attr, bad, devices, find_product, mount_of, port_usage, product_attr, resolve, str_param, supplied_by_others, wires};
use crate::cmd::file::io;
use crate::{EngineError, Result, Session};

#[cfg(test)]
mod tests;

/// Header variable holding the unplaced list as JSON `[[product id, qty], ...]`.
const VAR: &str = "CSDUNPLACED";
pub(super) const MAX_ROWS: usize = 1000;
pub(super) const MAX_QTY: u64 = 100_000;

/// What the lists show of a product.
struct Prod {
    id: String,
    code: String,
    cat: String,
    name: String,
    brand: String,
    notes: String,
    control: bool,
    poe_watts: f64,
}

/// Control gear patches no DMX channels but is wired into the control system (a console, gateway,
/// splitter or drive), as the web app decides it. ponytail: channels come from the library's
/// `channels` field, not the parsed modes.
fn is_control_gear(p: &Product) -> bool {
    if p.missing {
        return false;
    }
    if p.universes > 0 {
        return true;
    }
    if p.channels > 0 {
        return false;
    }
    if p.counts.iter().all(|&(i, o)| i == 0 && o == 0) {
        return !is_accessory(p);
    }
    true
}

impl Prod {
    fn of(p: &Product) -> Prod {
        Prod {
            id: p.id.clone(),
            code: p.code.clone(),
            cat: p.cat.clone(),
            name: p.name.clone(),
            brand: p.brand.clone(),
            notes: p.notes.clone(),
            control: is_control_gear(p),
            poe_watts: p.poe_watts,
        }
    }
}

struct Placed {
    handle: Handle,
    prod: Prod,
    label: String,
    notes: String,
    mount: String,
    accessories: Vec<(Prod, u64)>,
    by_others: bool,
    ports: Vec<(Protocol, usize, u64)>,
    conns: usize,
    poe: f64,
    poe_warning: Option<String>,
}

/// The devices on the drawing, control gear first.
fn placed(s: &Session) -> Result<Vec<Placed>> {
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
    let links = wires(store);
    let only: Vec<WireLink> = links.iter().map(|(_, l)| l.clone()).collect();
    let mut out: Vec<Placed> = store
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(i) = &e.kind else { return None };
            let product = resolve(&s.csd_library, product_attr(i)?);
            // The built-in blocks are drawing notes: on no list.
            if is_builtin(&product.id) {
                return None;
            }
            let ports = port_usage(&product, &only, e.handle);
            let conns = only.iter().filter(|l| l.from == e.handle || l.to == e.handle).count();
            Some(Placed {
                handle: e.handle,
                prod: Prod::of(&product),
                label: attr(i, "NAME").to_string(),
                notes: attr(i, "NOTES").to_string(),
                mount: mount_of(store, i)
                    .and_then(|m| match store.get(m).map(|e| &e.kind) {
                        Some(EntityKind::Insert(p)) => Some(attr(p, "NAME").to_string()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                accessories: accessories_of(i).into_iter().map(|(id, q)| (Prod::of(&resolve(&s.csd_library, &id)), q)).collect(),
                by_others: supplied_by_others(i),
                ports,
                conns,
                poe: 0.0,
                poe_warning: None,
            })
        })
        .collect();
    // A device's PoE load is the draw of what its PoE outlets feed.
    let draw: BTreeMap<Handle, f64> = out.iter().map(|p| (p.handle, p.prod.poe_watts)).collect();
    for p in &mut out {
        p.poe = only.iter().filter(|l| l.from == p.handle && l.protocol == Protocol::Network).filter_map(|l| draw.get(&l.to)).sum();
    }
    let devs = devices(store, &s.csd_library);
    let list: Vec<(Handle, &Product)> = devs.iter().map(|x| (x.handle, &*x.product)).collect();
    for p in &mut out {
        p.poe_warning = port_overload(&list, &only, p.handle);
    }
    out.sort_by_key(|p| !p.prod.control);
    Ok(out)
}

fn ports_text(ports: &[(Protocol, usize, u64)]) -> String {
    let one =
        |&(pr, n, cap): &(Protocol, usize, u64)| if cap == u64::MAX { format!("{} {n}", pr.label()) } else { format!("{} {n}/{cap}", pr.label()) };
    ports.iter().filter(|p| p.1 > 0).map(one).collect::<Vec<_>>().join(", ")
}

pub(super) fn run_schedule(s: &mut Session, _p: &Value) -> Result<Value> {
    let devices: Vec<Value> = placed(s)?
        .iter()
        .map(|d| {
            json!({
                "handle": d.handle.hex(), "product": d.prod.id, "name": d.prod.name, "category": d.prod.cat, "code": d.prod.code, "label": d.label, "notes": d.notes,
                "mountedIn": d.mount, "supplyByOthers": d.by_others,
                "accessories": d.accessories.iter().map(|(a, q)| json!({ "product": a.id, "code": a.code, "name": a.name, "qty": q })).collect::<Vec<_>>(),
                "portsInUse": ports_text(&d.ports), "connections": d.conns, "poeLoad": d.poe, "poeWarning": d.poe_warning, "controlGear": d.prod.control,
            })
        })
        .collect();
    Ok(json!({ "devices": devices }))
}

pub(super) fn read(d: &Drawing) -> Vec<(String, u64)> {
    let rows: Vec<(String, u64)> = d.header.get(VAR).and_then(HVal::as_str).and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    rows.into_iter().take(MAX_ROWS).map(|(id, q)| (id, q.min(MAX_QTY))).collect()
}

fn write(d: &mut Drawing, rows: &[(String, u64)]) {
    if rows.is_empty() {
        d.header.vars.remove(VAR);
    } else {
        d.header.set_str(VAR, &serde_json::to_string(rows).unwrap_or_default());
    }
}

/// Takes one off `product`'s unplaced row: placing it on the drawing is what the row was for.
pub(super) fn consume(d: &mut Drawing, product: &str) {
    let mut rows = read(d);
    let Some(i) = rows.iter().position(|(p, _)| p == product) else { return };
    match rows.get_mut(i) {
        Some(r) if r.1 > 1 => r.1 -= 1,
        _ => drop(rows.remove(i)),
    }
    write(d, &rows);
}

fn index_param(cmd: &str, p: &Value, len: usize) -> Result<usize> {
    let i = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).filter(|&i| i < len);
    i.ok_or_else(|| bad(cmd, format!("`index` must be 0..{len}")))
}

fn qty_param(cmd: &str, p: &Value, default: Option<u64>) -> Result<u64> {
    match (p.get("qty"), default) {
        (None, Some(d)) => Ok(d),
        (Some(v), _) => v.as_u64().filter(|&q| q >= 1).map(|q| q.min(MAX_QTY)).ok_or_else(|| bad(cmd, "`qty` must be a whole number, at least 1")),
        (None, None) => Err(bad(cmd, "`qty` is required")),
    }
}

pub(super) fn run_unplaced_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let items: Vec<Value> = read(s.doc()?)
        .iter()
        .enumerate()
        .map(|(index, (id, qty))| {
            let p = Prod::of(&resolve(&s.csd_library, id));
            json!({ "index": index, "product": p.id, "qty": qty, "code": p.code, "category": p.cat, "name": p.name })
        })
        .collect();
    Ok(json!({ "items": items }))
}

pub(super) fn run_unplaced_add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.unplaced.add";
    let id = str_param(p, "product").ok_or_else(|| bad(cmd, "`product` is required"))?;
    let qty = qty_param(cmd, p, Some(1))?;
    let id = find_product(&s.csd_library, id).map(|p| p.id.clone()).ok_or_else(|| bad(cmd, format!("unknown product \"{id}\"")))?;
    if is_builtin(&id) {
        return Err(bad(cmd, "built-in blocks are drawing notes, not products to order"));
    }
    let d = s.doc_mut()?;
    let mut rows = read(d);
    let index = match rows.iter().position(|(r, _)| *r == id) {
        Some(i) => {
            if let Some(r) = rows.get_mut(i) {
                r.1 = r.1.saturating_add(qty).min(MAX_QTY);
            }
            i
        }
        None if rows.len() >= MAX_ROWS => return Err(bad(cmd, format!("at most {MAX_ROWS} unplaced items"))),
        None => {
            rows.push((id, qty));
            rows.len() - 1
        }
    };
    write(d, &rows);
    Ok(json!({ "index": index }))
}

pub(super) fn run_unplaced_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.unplaced.set";
    let d = s.doc_mut()?;
    let mut rows = read(d);
    let i = index_param(cmd, p, rows.len())?;
    let qty = qty_param(cmd, p, None)?;
    if let Some(r) = rows.get_mut(i) {
        r.1 = qty;
    }
    write(d, &rows);
    Ok(json!({ "index": i }))
}

pub(super) fn run_unplaced_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.doc_mut()?;
    let mut rows = read(d);
    let i = index_param("csd.unplaced.remove", p, rows.len())?;
    rows.remove(i);
    write(d, &rows);
    Ok(json!({ "removed": i }))
}

pub(super) fn run_unplaced_place(s: &mut Session, p: &Value) -> Result<Value> {
    let rows = read(s.doc()?);
    let i = index_param("csd.unplaced.place", p, rows.len())?;
    let id = rows.get(i).map(|r| r.0.clone()).unwrap_or_default();
    super::place::run_arm(s, &json!({ "product": id }))
}

struct Line {
    prod: Prod,
    qty: u64,
    labels: Vec<String>,
}

/// Placed and unplaced products totalled per product: control-gear categories first, then by
/// category, and by model id within one. Accessories are totalled under their own product; a
/// device supplied by others is left out with its accessories.
fn bom(s: &Session) -> Result<Vec<Line>> {
    bom_on(s, None)
}

/// [`bom`] limited to the devices on sheet `page` (0-based); unplaced products are on no sheet and left out.
fn bom_on(s: &Session, page: Option<usize>) -> Result<Vec<Line>> {
    let on_page: Option<HashSet<Handle>> = match page {
        Some(n) => {
            let d = s.doc()?;
            let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
            let ps = super::page::pages_in(store);
            Some(
                store
                    .iter()
                    .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if super::page::page_of(d, &ps, i.insert.xy()) == Some(n)))
                    .map(|e| e.handle)
                    .collect(),
            )
        }
        None => None,
    };
    let mut by_id: BTreeMap<String, Line> = BTreeMap::new();
    let mut add = |prod: Prod, qty: u64, label: &str| {
        let line = by_id.entry(prod.id.clone()).or_insert_with(|| Line { prod, qty: 0, labels: Vec::new() });
        line.qty = line.qty.saturating_add(qty);
        let label = label.trim();
        if !label.is_empty() && !line.labels.iter().any(|l| l == label) {
            line.labels.push(label.to_string());
        }
    };
    for p in placed(s)? {
        if p.by_others || on_page.as_ref().is_some_and(|h| !h.contains(&p.handle)) {
            continue;
        }
        add(p.prod, 1, &p.label);
        for (a, q) in p.accessories {
            add(a, q, "");
        }
    }
    for (id, qty) in read(s.doc()?).into_iter().filter(|(id, _)| page.is_none() && !is_builtin(id)) {
        add(Prod::of(&resolve(&s.csd_library, &id)), qty, "");
    }
    let mut cats: BTreeMap<String, Vec<Line>> = BTreeMap::new();
    for line in by_id.into_values() {
        cats.entry(line.prod.cat.clone()).or_default().push(line);
    }
    let mut cats: Vec<Vec<Line>> = cats.into_values().collect();
    cats.sort_by_key(|c| !c.iter().all(|l| l.prod.control));
    Ok(cats.into_iter().flatten().collect())
}

pub(super) fn run_bom(s: &mut Session, _p: &Value) -> Result<Value> {
    let rows: Vec<Value> = bom(s)?
        .iter()
        .map(|l| {
            let p = &l.prod;
            json!({ "qty": l.qty, "product": p.id, "code": p.code, "category": p.cat, "name": p.name, "labels": l.labels, "notes": p.notes, "brand": p.brand })
        })
        .collect();
    Ok(json!({ "rows": rows }))
}

/// A CSV field; text a spreadsheet would run as a formula is kept as text.
fn field(t: &str) -> String {
    let t = if t.starts_with(['=', '+', '-', '@']) { format!("'{t}") } else { t.to_string() };
    if t.contains([',', '"', '\n', '\r']) { format!("\"{}\"", t.replace('"', "\"\"")) } else { t }
}

fn csv(lines: &[Line]) -> String {
    let mut out = String::from("Qty,Model ID,Labels,Category,Product,Code,Brand,Notes\r\n");
    for l in lines {
        let p = &l.prod;
        let cells = [
            l.qty.to_string(),
            field(&p.id),
            field(&l.labels.join("; ")),
            field(&p.cat),
            field(&p.name),
            field(&p.code),
            field(&p.brand),
            field(&p.notes),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// A titled table and footer as text on a scratch drawing, plotted by the PDF writer onto one
/// landscape A4 page. Columns are (heading, width). A one-cell row is a full-width section row.
/// ponytail: one page, so a very long list shrinks; paginate when that matters.
pub(super) fn table_pdf(cmd: &str, title: &str, cols: &[(&str, f64)], rows: &[Vec<String>], footer: &[String]) -> Result<Vec<u8>> {
    const H: f64 = 8.0;
    const PITCH: f64 = 14.0;
    let mut d = Drawing::default();
    let mut put = |x: f64, y: f64, h: f64, w: f64, t: &str| -> Result<()> {
        let value: String = t.chars().take((w / (h * 0.75)) as usize).collect();
        let text = Text {
            insert: Vec3::new(x, y, 0.0),
            align_pt: None,
            height: h,
            value,
            rotation: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            style: "Standard".into(),
            halign: HAlign::Left,
            valign: VAlign::Baseline,
        };
        d.add(&Space::Model, Common::default(), EntityKind::Text(text))?;
        Ok(())
    };
    put(0.0, 0.0, 14.0, 400.0, title)?;
    let mut y = -28.0;
    let mut row = |cells: &[String], y: f64| -> Result<()> {
        if let [only] = cells {
            return put(0.0, y, H, cols.iter().map(|c| c.1).sum(), only);
        }
        let mut x = 0.0;
        for (cell, (_, w)) in cells.iter().zip(cols) {
            put(x, y, H, w - 6.0, cell)?;
            x += w;
        }
        Ok(())
    };
    row(&cols.iter().map(|c| c.0.to_string()).collect::<Vec<_>>(), y)?;
    for r in rows {
        y -= PITCH;
        row(r, y)?;
    }
    for line in footer {
        y -= PITCH;
        put(0.0, y, H, cols.iter().map(|c| c.1).sum(), line)?;
    }
    let plot = io().and_then(|h| h.plot).ok_or_else(|| bad(cmd, "PDF output is not available in this build"))?;
    plot(&d, &Space::Model, &json!({ "paper": "A4", "landscape": true, "fit": true, "title": title })).map_err(|e| bad(cmd, e))
}

fn pdf(lines: &[Line], footer: &[String]) -> Result<Vec<u8>> {
    const COLS: [(&str, f64); 6] = [("Qty", 30.0), ("Model ID", 150.0), ("Labels", 230.0), ("Category", 130.0), ("Product", 230.0), ("Code", 90.0)];
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|l| {
            let p = &l.prod;
            vec![l.qty.to_string(), p.id.clone(), l.labels.join(", "), p.cat.clone(), p.name.clone(), p.code.clone()]
        })
        .collect();
    table_pdf("csd.bom.export", "Bill of materials", &COLS, &rows, footer)
}

/// The BOM page, grouped by category in the BOM's own order.
/// With a section title (per page), each list follows its heading; an empty page keeps its heading.
fn html(sections: &[(Option<String>, Vec<Line>)], footer: &[String]) -> String {
    use super::html::{group_row, page, row};
    let mut body = String::new();
    for (title, lines) in sections {
        let mut cat: Option<&str> = None;
        if let Some(t) = title {
            body.push_str(&group_row("group", 6, t, ""));
        }
        for l in lines {
            let p = &l.prod;
            if cat != Some(p.cat.as_str()) {
                cat = Some(p.cat.as_str());
                body.push_str(&group_row("group", 6, &p.cat, ""));
            }
            let name = if p.notes.is_empty() { p.name.clone() } else { format!("{} ({})", p.name, p.notes) };
            body.push_str(&row("", &[l.qty.to_string(), p.id.clone(), l.labels.join(", "), p.brand.clone(), name, p.code.clone()]));
        }
    }
    let all = sections.iter().flat_map(|(_, l)| l);
    let info = [("Items", all.clone().map(|l| l.qty).sum::<u64>().to_string()), ("Products", all.count().to_string())];
    page("Bill of materials", &info, &["Qty", "Model ID", "Labels", "Brand", "Product", "Code"], &body, footer)
}

pub(super) fn run_bom_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.bom.export";
    let path = str_param(p, "path").ok_or_else(|| bad(cmd, "`path` is required"))?;
    let lines = bom(s)?;
    let footer = super::html::footer_lines(s, p);
    let bytes = match str_param(p, "format") {
        Some("csv") => csv(&lines).into_bytes(),
        Some("pdf") => pdf(&lines, &footer)?,
        Some("html") => {
            let per_page = p.get("perPage").and_then(Value::as_bool) == Some(true);
            let ps = super::page::pages(s);
            let sections = if per_page && ps.len() > 1 {
                ps.iter().enumerate().map(|(n, g)| Ok((Some(g.name(n)), bom_on(s, Some(n))?))).collect::<Result<Vec<_>>>()?
            } else {
                vec![(None, bom(s)?)]
            };
            html(&sections, &footer).into_bytes()
        }
        _ => return Err(bad(cmd, "`format` must be csv, pdf or html")),
    };
    super::write_library(path, &bytes).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok(json!({ "path": path, "rows": lines.len() }))
}
