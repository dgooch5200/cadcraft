//! `csd.import.cpsd`: builds a ChamSys Designer project file's page into the drawing. The file is
//! hostile input: read through a size cap, walked leniently as JSON, every count and number bounded.

use std::collections::{BTreeMap, HashMap};

use cadcraft_csd::{MAX_COORD, MAX_WIRE_POINTS, OFF_PAGE_ID, Paper, Protocol, Side, WireLink, card_size, is_builtin};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::patch::MODE_TAG;
use super::{
    MAX_SIZE, MIN_SIZE, accessory_param, add_wire, bad, mount_cycles, page, place_card, put_attr, resolve, schedule, sheet, str_param, twoline,
};
use crate::{Result, Session};
use cadcraft_doc::{EntityKind, Handle};

const CMD: &str = "csd.import.cpsd";
#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: usize = 50 * 1024 * 1024;
const MAX_DEVICES: usize = 20_000;
const MAX_WIRES: usize = 50_000;
const MAX_PAGES: usize = 1_000;
/// ChamSys keeps the page's top-left corner at (30, 30) of its y-down canvas.
const PAGE_ORIGIN: f64 = 30.0;
const UNPLACED_PAGE: &str = "unplaced";

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> std::io::Result<Vec<u8>> {
    crate::capped_read::read_capped(std::path::Path::new(path), MAX_BYTES)
}

#[cfg(target_arch = "wasm32")]
fn read(_: &str) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "files are not available on the web"))
}

pub(super) fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub(super) fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn list<'a>(v: &'a Value, key: &str, cap: usize) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().take(cap)
}

/// ChamSys canvas (y down, page corner at 30,30) to the drawing (y up, page corner at the origin).
pub(super) fn to_drawing(x: f64, y: f64) -> Option<Vec2> {
    let p = Vec2::new(x - PAGE_ORIGIN, PAGE_ORIGIN + Paper::Tabloid.size().1 - y);
    (p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD).then_some(p)
}

/// The drawing (y up, page corner at the origin) to ChamSys canvas coordinates.
pub(super) fn from_drawing(p: Vec2) -> (f64, f64) {
    (p.x + PAGE_ORIGIN, PAGE_ORIGIN + Paper::Tabloid.size().1 - p.y)
}

/// Title block fields as (ChamSys key, `csd.project` key).
pub(super) const TITLE_MAP: [(&str, &str); 7] = [
    ("project", "project"),
    ("venue", "venue"),
    ("location", "location"),
    ("designer", "drawnBy"),
    ("date", "date"),
    ("rev", "rev"),
    ("system", "title"),
];

fn side(v: &Value, key: &str, fallback: Side) -> Side {
    match text(v, key).as_str() {
        "bottom" => Side::Bottom,
        "left" => Side::Left,
        "right" => Side::Right,
        id => Side::from_id(id).unwrap_or(fallback),
    }
}

/// Counts reasons so a file of thousands of bad rows reports a handful of lines.
#[derive(Default)]
struct Skips(BTreeMap<String, usize>);

impl Skips {
    fn add(&mut self, reason: impl Into<String>) {
        *self.0.entry(reason.into()).or_default() += 1;
    }
}

/// A failed import leaves the routing mode as it was; the wires it drew need the file's mode while it runs.
pub(super) fn run_import(s: &mut Session, p: &Value) -> Result<Value> {
    let ortho = s.settings.orthomode;
    let r = import(s, p);
    if r.is_err() {
        s.settings.orthomode = ortho;
    }
    r
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let bytes = read(path).map_err(|e| bad(CMD, format!("{path}: {e}")))?;
    let file: Value = serde_json::from_slice(&bytes).map_err(|e| bad(CMD, format!("{path}: not a ChamSys project file ({e})")))?;
    if !file.is_object() {
        return Err(bad(CMD, format!("{path}: not a ChamSys project file (expected a JSON object)")));
    }

    // Page ids in file order; none means the one unnamed first page.
    let mut pages: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for e in file.get("pages").and_then(Value::as_array).into_iter().flatten().take(MAX_PAGES) {
        let id = if e.is_object() { text(e, "id") } else { e.as_str().unwrap_or_default().to_string() };
        if !id.trim().is_empty() && !pages.contains(&id) {
            names.push(if e.is_object() { text(e, "name") } else { String::new() });
            pages.push(id);
        }
    }
    if pages.is_empty() {
        pages.push(String::new());
    }
    let all = p.get("page").and_then(Value::as_str) == Some("all") && !pages.iter().any(|x| x == "all");
    let chosen = match p.get("page") {
        None | Some(Value::Null) => Some(0),
        Some(Value::String(_)) if all => Some(0),
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)).filter(|i| *i < pages.len()),
        Some(Value::String(id)) => {
            pages.iter().position(|x| x == id).or_else(|| id.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).filter(|i| *i < pages.len()))
        }
        Some(_) => None,
    }
    .ok_or_else(|| bad(CMD, format!("`page` must be a number from 1 to {} or a page id", pages.len())))?;
    // The index of the page an item sits on (an unknown page is the first), if the import takes it.
    let page_idx = |d: &Value| {
        let i = pages.iter().position(|x| *x == text(d, "page")).unwrap_or(0);
        (all || i == chosen).then_some(i)
    };
    // `page: "all"` lays page i on sheet i; the offsets are the sheets' distances from the first.
    let mut offsets = vec![0.0; pages.len()];
    if all {
        sheet::run_titleblock(s, &json!({}))?;
        while page::pages(s).len() < pages.len() {
            page::add_page(s)?;
        }
        let at: Vec<f64> = page::pages(s).iter().map(|g| g.at.x).collect();
        for (o, x) in offsets.iter_mut().zip(&at) {
            *o = x - at.first().copied().unwrap_or(0.0);
        }
        for (i, n) in names.iter().enumerate().filter(|(_, n)| !n.trim().is_empty()) {
            page::name_page(s, i, n)?;
        }
    }

    s.settings.orthomode = file.get("ortho").and_then(Value::as_bool) == Some(true);

    let library = s.csd_library.clone();
    let mut skips = Skips::default();
    let mut placed: HashMap<String, (Handle, usize)> = HashMap::new();
    let mut mounts: Vec<(Handle, String)> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();
    let mut stand_ins = 0;

    for d in list(&file, "nodes", MAX_DEVICES) {
        let pid = [text(d, "prodId"), text(d, "prod_id"), text(d, "productId")].into_iter().find(|t| !t.is_empty());
        let Some(pid) = pid else {
            skips.add("device without a product");
            continue;
        };
        if text(d, "page") == UNPLACED_PAGE {
            unplaced.push(pid);
            continue;
        }
        let Some(page) = page_idx(d) else {
            skips.add("device on another page");
            continue;
        };
        let id = text(d, "id");
        if placed.contains_key(&id) {
            skips.add("device with a repeated id");
            continue;
        }
        let size = |k| number(d, k).filter(|n| *n > 0.0);
        let (w, h) = (size("w").map(|w| w.clamp(MIN_SIZE, MAX_SIZE)), size("h").map(|h| h.clamp(MIN_SIZE, MAX_SIZE)));
        let (x, y) = (number(d, "x").unwrap_or(100.0), number(d, "y").unwrap_or(100.0));
        let product = resolve(&library, &pid).into_owned();
        let two_line = !is_builtin(&product.id)
            && (d.get("twoLine").and_then(Value::as_bool) == Some(true) || d.get("showNote").and_then(Value::as_bool) == Some(true));
        let card = card_size(&product, two_line);
        // x, y is the card's top-left; cards are inserted at their centre.
        let Some(at) = to_drawing(x + w.unwrap_or(card.x) / 2.0 + offsets[page], y + h.unwrap_or(card.y) / 2.0) else {
            skips.add("device outside the drawing limits");
            continue;
        };
        stand_ins += usize::from(product.missing);
        let handle = place_card(s, &product, &text(d, "label"), at)?;
        if two_line {
            twoline::set(s, &[handle], true)?;
        }
        let notes = text(d, "notes");
        let mount = text(d, "parentId");
        let accessories = accessories(d);
        let accs = accessory_param(&accessories).map_err(|_| skips.add("device with unusable accessories")).ok().filter(|a| !a.is_empty());
        let mode = text(d, "mode");
        // ChamSys names an unnamed page "Drawing N".
        let link_page = (product.id == OFF_PAGE_ID).then(|| text(d, "linkPage")).and_then(|id| {
            let n = pages.iter().position(|p| *p == id)?;
            Some(names.get(n).filter(|t| !t.trim().is_empty()).cloned().unwrap_or_else(|| format!("Drawing {}", n + 1)))
        });
        s.doc_mut()?.modify_entity(handle, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            if w.is_some() || h.is_some() {
                i.scale.x = w.map_or(1.0, |w| w / card.x);
                i.scale.y = h.map_or(1.0, |h| h / card.y);
            }
            if !notes.is_empty() {
                put_attr(i, "NOTES", notes);
            }
            if let Some(l) = link_page {
                put_attr(i, "LINKPAGE", l);
            }
            if !mode.is_empty() {
                put_attr(i, MODE_TAG, mode);
            }
            if let Some(a) = accs {
                put_attr(i, "ACCS", serde_json::to_string(&a).unwrap_or_default());
            }
        })?;
        if !mount.is_empty() {
            mounts.push((handle, mount));
        }
        placed.insert(id, (handle, page));
    }

    for (child, parent) in mounts {
        let Some(&(parent, _)) = placed.get(&parent) else { continue };
        let cycle = {
            let store = s.doc()?.space(&s.space()).ok_or_else(|| bad(CMD, "no such space"))?;
            mount_cycles(store, child, parent)
        };
        if cycle {
            skips.add("mount that would loop");
        } else {
            s.doc_mut()?.modify_entity(child, |e| {
                if let EntityKind::Insert(i) = &mut e.kind {
                    put_attr(i, "MOUNT", parent.hex());
                }
            })?;
        }
    }

    let mut wires = 0;
    for c in list(&file, "conns", MAX_WIRES) {
        let (Some(&(from, fp)), Some(&(to, tp))) = (placed.get(&text(c, "from")), placed.get(&text(c, "to"))) else {
            skips.add("wire to a device that is not on this page");
            continue;
        };
        if fp != tp {
            skips.add("wire between devices on different pages");
            continue;
        }
        if from == to {
            skips.add("wire from a device to itself");
            continue;
        }
        let protocol = match text(c, "type").as_str() {
            "poe" | "PoE" => Protocol::Network,
            id => Protocol::from_id(id).unwrap_or_else(|| {
                skips.add("wire of an unknown type (drawn as network)");
                Protocol::Network
            }),
        };
        let mut link = WireLink::new(from, side(c, "fromPort", Side::Right), to, side(c, "toPort", Side::Left), protocol);
        link.points = list(c, "points", MAX_WIRE_POINTS).filter_map(|q| to_drawing(number(q, "x")? + offsets[fp], number(q, "y")?)).collect();
        link.label_t = number(c, "labelT").map(|t| t.clamp(0.0, 1.0));
        link.name = Some(text(c, "label").trim().to_string()).filter(|n| !n.is_empty());
        add_wire(s, &link)?;
        wires += 1;
    }

    let mut unplaced_rows = 0;
    for pid in unplaced {
        let id = super::find_product(&library, &pid).map(|p| p.id.clone());
        match id {
            Some(id) => {
                schedule::run_unplaced_add(s, &json!({ "product": id }))?;
                unplaced_rows += 1;
            }
            None => skips.add("unplaced item whose product is not in the library"),
        }
    }

    let title = file.get("titleBlock").cloned().unwrap_or(Value::Null);
    let fields: serde_json::Map<String, Value> = TITLE_MAP
        .into_iter()
        .map(|(from, to)| (to.to_string(), text(&title, from)))
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(k, t)| (k, json!(t)))
        .collect();
    if !fields.is_empty() {
        sheet::run_project_set(s, &json!({ "fields": fields }))?;
    }

    let shapes = super::shapes::import(s, list(&file, "shapes", super::shapes::MAX_SHAPES).filter_map(|v| Some((v, offsets[page_idx(v)?]))), |r| {
        skips.add(r)
    })?;
    let mut skipped: Vec<String> = skips.0.into_iter().map(|(r, n)| format!("{n} x {r}")).collect();
    if pages.len() > 1 && !all {
        skipped.push(format!("{} other page(s); pass `page` to import another", pages.len() - 1));
    }
    Ok(
        json!({ "devices": placed.len(), "wires": wires, "unplaced": unplaced_rows, "shapes": shapes, "standIns": stand_ins, "skipped": skipped, "pages": if all { pages.len() } else { 1 } }),
    )
}

/// The file's accessory lines as `csd.device.set` writes them: `{product, qty}`, junk rows dropped.
fn accessories(d: &Value) -> Value {
    let rows: Vec<Value> = list(d, "accessories", 200)
        .filter_map(|a| {
            let id = [text(a, "productId"), text(a, "prodId")].into_iter().find(|t| !t.is_empty())?;
            let qty = number(a, "quantity")?.floor();
            (qty >= 1.0).then(|| json!({ "product": id, "qty": qty.min(100_000.0) as u64 }))
        })
        .collect();
    Value::Array(rows)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    const PRODUCT: &str = "CHAMMQ50";

    fn import(json: &str) -> (Session, Value) {
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-{}-{:p}.cpsd", std::process::id(), json));
        std::fs::write(&path, json).unwrap();
        let mut s = Session::new();
        let r = s.execute("csd.import.cpsd", &json!({ "path": path.to_str().unwrap() }));
        let _ = std::fs::remove_file(&path);
        let r = r.unwrap_or_else(|e| json!({ "error": e.to_string() }));
        (s, r)
    }

    #[test]
    fn imports_devices_wires_unplaced_and_title() {
        let file = json!({
            "_format": 7,
            "titleBlock": { "project": "Show", "designer": "Dee", "system": "Rig" },
            "pages": [{ "id": "p1", "name": "" }],
            "nodes": [
                { "id": "a", "prodId": PRODUCT, "x": 100, "y": 100, "label": "Desk", "notes": "front", "mode": "17ch" },
                { "id": "b", "prodId": PRODUCT, "x": 400, "y": 100, "parentId": "a", "w": 90, "h": 100 },
                { "id": "c", "prodId": PRODUCT, "page": "unplaced" },
                { "id": "d", "prodId": "NOPE-9", "x": 50, "y": 300 }
            ],
            "conns": [
                { "id": "w", "from": "a", "to": "b", "fromPort": "right", "toPort": "lft", "type": "dmx",
                  "label": "Run 1", "labelT": 0.25, "points": [{ "x": 250, "y": 160 }, { "x": "bad" }] },
                { "from": "a", "to": "zzz", "type": "dmx" }
            ]
        });
        let (mut s, r) = import(&file.to_string());
        assert_eq!(r["devices"], 3, "{r}");
        assert_eq!((r["wires"].clone(), r["unplaced"].clone(), r["standIns"].clone()), (json!(1), json!(1), json!(1)), "{r}");
        let net = s.execute("csd.netlist", &json!({})).unwrap();
        assert_eq!(net["wires"][0]["protocol"], "dmx");
        let project = s.execute("csd.project", &json!({})).unwrap();
        assert_eq!(project["fields"]["project"], "Show");
        assert_eq!(project["fields"]["drawnBy"], "Dee");
        assert_eq!(s.execute("csd.unplaced.list", &json!({})).unwrap()["items"][0]["product"], PRODUCT);
    }

    #[test]
    fn imports_shapes_on_their_layer() {
        let file = json!({
            "pages": [{ "id": "p1" }, { "id": "p2" }],
            "shapes": [
                { "type": "line", "x1": 30, "y1": 30, "x2": 130, "y2": 30, "color": "#ff0000" },
                { "type": "rect", "x1": 130, "y1": 80, "x2": 30, "y2": 30 },
                { "type": "ellipse", "x1": 30, "y1": 30, "x2": 90, "y2": 50 },
                { "type": "ellipse", "x1": 30, "y1": 30, "x2": 30, "y2": 50 },
                { "type": "text", "x1": 30, "y1": 30, "x2": 130, "y2": 60, "text": "A {b}\\c\nd", "fontSize": 20, "textColor": "#0000ff" },
                { "type": "image", "src": "data:" },
                { "type": "line", "x1": 100, "page": "p2" },
                { "type": "rect", "x1": 1e308, "page": "p1" },
                7
            ]
        });
        let (mut s, r) = import(&file.to_string());
        assert_eq!(r["shapes"], 4, "{r}");
        let skipped = r["skipped"].to_string();
        assert!(skipped.contains("image shape") && skipped.contains("ellipse with no size") && skipped.contains("outside"), "{r}");
        let doc = s.doc().unwrap();
        assert!(doc.layer("CSD_SHAPES").is_some());
        let store = doc.space(&s.space()).unwrap();
        let kinds: Vec<_> = store.iter().filter(|e| e.common.layer == "CSD_SHAPES").map(|e| std::mem::discriminant(&e.kind)).collect();
        assert_eq!(kinds.len(), 4);
        let mt = store.iter().find_map(|e| if let EntityKind::MText(m) = &e.kind { Some(m) } else { None }).unwrap();
        assert_eq!((mt.height, mt.width), (20.0, 100.0));
        assert_eq!(cadcraft_fonts::plain_mtext(&mt.contents), "A {b}\\c\nd");
        assert!((mt.insert.x, mt.insert.y) == (0.0, Paper::Tabloid.size().1));
        assert!(s.execute("csd.netlist", &json!({})).is_ok());
    }

    #[test]
    fn off_page_target_becomes_the_page_name() {
        let file = json!({
            "pages": [{ "id": "p1", "name": "Stage" }, { "id": "p2", "name": "" }, { "id": "p3", "name": "Riser" }],
            "nodes": [
                { "id": "a", "prodId": "__off-page", "x": 0, "y": 0, "linkPage": "p3" },
                { "id": "b", "prodId": "__off-page", "x": 300, "y": 0, "linkPage": "p2" },
                { "id": "c", "prodId": "__off-page", "x": 600, "y": 0, "linkPage": "gone" },
                { "id": "d", "prodId": "CHAMMQ50", "x": 900, "y": 0, "linkPage": "p3" }
            ]
        });
        let (mut s, r) = import(&file.to_string());
        assert_eq!(r["devices"], 4, "{r}");
        let lib = s.csd_library.clone();
        let mut links: Vec<(String, String)> = {
            let store = s.doc().unwrap().space(&s.space()).unwrap();
            super::super::devices(store, &lib).iter().map(|d| (d.product.id.clone(), super::super::attr(d.insert, "LINKPAGE").to_string())).collect()
        };
        links.sort();
        let want = [("CHAMMQ50", ""), ("__off-page", ""), ("__off-page", "Drawing 2"), ("__off-page", "Riser")];
        assert_eq!(links, want.map(|(a, b)| (a.to_string(), b.to_string())));
        assert!(s.execute("csd.netlist", &json!({})).is_ok());
    }

    pub(in crate::cmd::csd) fn two_page_file() -> Value {
        json!({
            "pages": [{ "id": "p1", "name": "Stage" }, { "id": "p2", "name": "" }],
            "titleBlock": { "project": "Show" },
            "nodes": [
                { "id": "a", "prodId": PRODUCT, "x": 100, "y": 100 },
                { "id": "b", "prodId": PRODUCT, "x": 400, "y": 100, "page": "p1" },
                { "id": "c", "prodId": PRODUCT, "x": 100, "y": 100, "page": "p2" },
                { "id": "d", "prodId": PRODUCT, "x": 400, "y": 100, "page": "p2" },
                { "id": "o", "prodId": "__off-page", "x": 700, "y": 100, "page": "p1", "linkPage": "p2" }
            ],
            "conns": [
                { "from": "a", "to": "b", "type": "dmx" },
                { "from": "c", "to": "d", "type": "dmx", "points": [{ "x": 250, "y": 200 }] },
                { "from": "a", "to": "c", "type": "dmx" }
            ],
            "shapes": [{ "type": "line", "x1": 30, "y1": 30, "x2": 130, "y2": 30, "page": "p2" }]
        })
    }

    #[test]
    fn page_all_lays_each_page_on_its_own_sheet() {
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-all-{}.cpsd", std::process::id()));
        std::fs::write(&path, two_page_file().to_string()).unwrap();
        let mut s = Session::new();
        let r = s.execute("csd.import.cpsd", &json!({ "path": path.to_str().unwrap(), "page": "all" })).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            (r["devices"].clone(), r["wires"].clone(), r["shapes"].clone(), r["pages"].clone()),
            (json!(5), json!(2), json!(1), json!(2)),
            "{r}"
        );
        assert!(r["skipped"].to_string().contains("wire between devices on different pages"), "{r}");
        let pages = s.execute("csd.pages", &json!({})).unwrap();
        assert_eq!((pages["pages"][0]["name"].clone(), pages["pages"][1]["name"].clone()), (json!("Stage"), json!("Drawing 2")));
        let ps = page::pages(&s);
        let lib = s.csd_library.clone();
        let doc = s.doc().unwrap();
        let store = doc.space(&s.space()).unwrap();
        let mut on = [0usize; 2];
        for d in super::super::devices(store, &lib) {
            on[page::page_of(doc, &ps, d.insert.insert.xy()).unwrap()] += 1;
        }
        assert_eq!(on, [3, 2]);
        let link = super::super::devices(store, &lib).into_iter().find(|d| d.product.id == "__off-page").unwrap();
        assert_eq!(super::super::attr(link.insert, "LINKPAGE"), "Drawing 2");
        assert_eq!(s.execute("csd.project", &json!({})).unwrap()["fields"]["project"], "Show");
        // Without `page`, the one-page import is unchanged: page 1's three devices, no new sheets.
        std::fs::write(&path, two_page_file().to_string()).unwrap();
        let mut t = Session::new();
        let r = t.execute("csd.import.cpsd", &json!({ "path": path.to_str().unwrap() })).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(r["devices"], 3, "{r}");
        assert!(page::pages(&t).len() <= 1);
    }

    #[test]
    fn ortho_follows_the_file() {
        let (s, _) = import(r#"{"ortho":true}"#);
        assert!(s.settings.orthomode);
        let (s, _) = import(r#"{"ortho":"yes"}"#);
        assert!(!s.settings.orthomode);
    }

    #[test]
    fn a_huge_card_size_is_clamped_before_centring_and_poe_wires_are_network() {
        let (s, r) = import(
            r#"{"nodes":[{"id":"a","prodId":"CHAMMQ50","x":0,"y":0,"w":1e9,"h":1e9},{"id":"b","prodId":"CHAMMQ50","x":0,"y":0}],"conns":[{"from":"a","to":"b","type":"poe"}]}"#,
        );
        assert_eq!(r["devices"], 2, "{r}");
        let lib = s.csd_library.clone();
        let store = s.doc().unwrap().space(&s.space()).unwrap();
        assert!(super::super::devices(store, &lib).iter().all(|d| d.insert.insert.x.abs() < 2000.0));
        let links = super::super::wires(store);
        assert!(links.iter().all(|(_, l)| l.protocol == Protocol::Network));
    }

    #[test]
    fn hostile_files_do_not_panic() {
        for file in [
            "[]",
            "null",
            "{}",
            "not json",
            r#"{"nodes":[{"prodId":"X","x":1e308,"y":-1e308}],"conns":[{"from":"","to":""}]}"#,
            r#"{"nodes":[{"id":"a","prodId":"X","w":1e308,"h":-5,"accessories":[{"productId":"Y","quantity":1e300},5,null]},{"id":"a","prodId":"X"}],"conns":[{"from":"a","to":"a"}]}"#,
            r#"{"nodes":[{"id":"a","prodId":"X","parentId":"a"},{"id":"b","prodId":"X","parentId":"a"}],"pages":[1,null,{"id":5},"p"]}"#,
            r#"{"nodes":[{"id":"a","prodId":"X"},{"id":"b","prodId":"X"}],"conns":[{"from":"a","to":"b","type":"warp","fromPort":9,"labelT":"nan","points":[{"x":1e999,"y":0}]}],"titleBlock":7,"shapes":3}"#,
        ] {
            let (_, r) = import(file);
            assert!(r.is_object(), "{file}");
        }
    }

    #[test]
    fn missing_file_and_bad_page_are_errors() {
        let mut s = Session::new();
        assert!(s.execute("csd.import.cpsd", &json!({ "path": "/nonexistent/x.cpsd" })).is_err());
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-page-{}.cpsd", std::process::id()));
        std::fs::write(&path, "{}").unwrap();
        assert!(s.execute("csd.import.cpsd", &json!({ "path": path.to_str().unwrap(), "page": 9 })).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
