//! `csd.export.cpsd`: writes the drawing as a ChamSys Designer project file, the inverse of
//! `csd.import.cpsd` (it shares that module's coordinate mapping and title-field table).

use std::collections::{BTreeSet, HashMap};

use cadcraft_csd::{is_builtin, product_record};
use cadcraft_doc::Handle;
use cadcraft_geom::Vec2;
use serde_json::{Map, Value, json};

use super::cpsd::{TITLE_MAP, from_drawing};
use super::patch::MODE_TAG;
use super::{accessories_of, attr, bad, devices, find_product, mount_of, page, schedule, sheet, str_param, two_line, wires, write_library};
use crate::{EngineError, Result, Session};

const CMD: &str = "csd.export.cpsd";
/// ChamSys's `_format`; 7 added pages.
const FORMAT: u32 = 7;
const PAGE_NAME: &str = "Canvas 1";
/// Unplaced quantities are written one node per unit; a huge quantity must not balloon the file.
const MAX_UNPLACED_NODES: u64 = 10_000;

/// `dx` is the sheet's distance from the first, taken off so each page starts at its own corner.
fn point(p: Vec2, dx: f64) -> Value {
    let (x, y) = from_drawing(Vec2::new(p.x - dx, p.y));
    json!({ "x": x, "y": y })
}

pub(super) fn run_export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let library = s.csd_library.clone();
    let ortho = s.settings.orthomode;
    let project = sheet::run_project(s, &Value::Null)?;
    let doc = s.doc()?;
    let store = doc.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;

    let sheets = page::pages_in(store);
    let multi = sheets.len() > 1;
    let origin = sheets.first().map_or(0.0, |g| g.at.x);
    let offset = |i: usize| sheets.get(i).map_or(0.0, |g| g.at.x - origin);
    let page_id = |i: usize| format!("page-{}", i + 1);
    // A page's name in the file; the one unnamed page of a single-page drawing is ChamSys's "Canvas 1".
    let page_names: Vec<String> = (0..sheets.len().max(1))
        .map(|i| sheets.get(i).map_or(String::new(), |g| g.explicit.trim().to_string()))
        .enumerate()
        .map(|(i, n)| {
            if !n.is_empty() {
                n
            } else if multi {
                format!("Drawing {}", i + 1)
            } else {
                PAGE_NAME.to_string()
            }
        })
        .collect();
    let devs = devices(store, &library);
    let on: HashMap<Handle, usize> = devs.iter().map(|d| (d.handle, page::page_of(doc, &sheets, d.insert.insert.xy()).unwrap_or(0))).collect();
    let ids: HashMap<Handle, String> = devs.iter().enumerate().map(|(n, d)| (d.handle, format!("d{n}"))).collect();
    let mut used: BTreeSet<String> = BTreeSet::new();

    let mut skipped: Vec<String> = Vec::new();
    let mut lost_links = 0;
    let mut nodes: Vec<Value> = Vec::new();
    for (n, d) in devs.iter().enumerate() {
        let i = d.insert;
        let card = d.size();
        let (w, h) = (card.x * i.scale.x.abs(), card.y * i.scale.y.abs());
        // The insert point is the card's centre; ChamSys keeps the top-left.
        let pg = on.get(&d.handle).copied().unwrap_or(0);
        let (cx, cy) = from_drawing(Vec2::new(i.insert.x - offset(pg), i.insert.y));
        let name = attr(i, "NAME");
        let mut node = json!({
            "id": format!("d{n}"),
            "prodId": d.product.id,
            "x": cx - w / 2.0,
            "y": cy - h / 2.0,
            "label": if name == d.product.code { "" } else { name },
            "notes": attr(i, "NOTES"),
        });
        if (w - card.x).abs() > 1e-9 || (h - card.y).abs() > 1e-9 {
            node["w"] = json!(w);
            node["h"] = json!(h);
        }
        if multi {
            node["page"] = json!(page_id(pg));
        }
        let target = attr(i, "LINKPAGE").trim();
        if !target.is_empty() {
            let to = (0..page_names.len()).find(|&n| target == page_names[n] || target == page_id(n) || target == format!("Drawing {}", n + 1));
            match to {
                Some(n) => node["linkPage"] = json!(page_id(n)),
                None => lost_links += 1,
            }
        }
        if two_line(i) {
            node["twoLine"] = json!(true);
        }
        let accs = accessories_of(i);
        if !accs.is_empty() {
            node["accessories"] = accs.iter().map(|(id, q)| json!({ "productId": id, "quantity": q })).collect();
            used.extend(accs.into_iter().map(|a| a.0));
        }
        if let Some(parent) = mount_of(store, i).and_then(|h| ids.get(&h)) {
            node["parentId"] = json!(parent);
        }
        if !attr(i, MODE_TAG).is_empty() {
            node["mode"] = json!(attr(i, MODE_TAG));
        }
        used.insert(d.product.id.clone());
        nodes.push(node);
    }

    let mut unplaced = 0u64;
    for (id, qty) in schedule::read(doc) {
        for _ in 0..qty.min(MAX_UNPLACED_NODES.saturating_sub(unplaced)) {
            nodes.push(json!({ "id": format!("u{unplaced}"), "prodId": id, "page": "unplaced", "x": 100, "y": 100, "label": "", "notes": "" }));
            unplaced += 1;
        }
        used.insert(id);
    }

    let conns: Vec<Value> = wires(store)
        .into_iter()
        .enumerate()
        .filter_map(|(n, (_, l))| {
            let (from, to) = (ids.get(&l.from)?, ids.get(&l.to)?);
            let dx = offset(on.get(&l.from).copied().unwrap_or(0));
            let mut c = json!({
                "id": format!("c{n}"), "from": from, "fromPort": l.from_side.id(), "to": to, "toPort": l.to_side.id(),
                "type": l.protocol.id(), "label": l.name.clone().unwrap_or_default(),
            });
            if !l.points.is_empty() {
                c["points"] = l.points.iter().map(|p| point(*p, dx)).collect();
            }
            if let Some(t) = l.label_t {
                c["labelT"] = json!(t);
            }
            Some(c)
        })
        .collect();

    // Built-ins are the app's own and a missing product has no record to carry.
    let products: Vec<Value> = used
        .iter()
        .filter(|id| !is_builtin(id))
        .filter_map(|id| find_product(&library, id).filter(|p| !p.missing))
        .map(|p| Value::Object(product_record(p)))
        .collect();

    let mut title = Map::new();
    title.insert("sheet".into(), json!(format!("1 of {}", page_names.len())));
    for (theirs, ours) in TITLE_MAP {
        title.insert(theirs.into(), json!(project["fields"][ours].as_str().unwrap_or_default()));
    }
    let (mut shapes, skipped_shapes) = super::shapes::export(store);
    if multi {
        for v in &mut shapes {
            let x = v["x1"].as_f64().unwrap_or(0.0) - from_drawing(Vec2::ZERO).0;
            let n = (0..sheets.len()).rev().find(|&n| x >= origin + offset(n)).unwrap_or(0);
            for k in ["x1", "x2"] {
                v[k] = json!(v[k].as_f64().unwrap_or(0.0) - offset(n));
            }
            v["page"] = json!(page_id(n));
        }
    }
    if skipped_shapes > 0 {
        skipped.push(format!("{skipped_shapes} x entity on CSD_SHAPES that is not a line, rectangle, ellipse or text"));
    }
    if lost_links > 0 && multi {
        skipped.push(format!("{lost_links} x off-page target that is not a page of this drawing"));
    } else if lost_links > 0 {
        skipped.push(format!("{lost_links} x off-page target that is not this page (the export has one page, \"{PAGE_NAME}\")"));
    }
    let file = json!({
        "_app": "ChamSys System Designer", "_format": FORMAT, "_ext": ".cpsd", "ortho": ortho,
        "titleBlock": title,
        "pages": page_names.iter().enumerate().map(|(n, name)| json!({ "id": page_id(n), "name": name })).collect::<Vec<_>>(),
        "products": products, "nodes": nodes, "conns": conns, "shapes": shapes,
    });
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| bad(CMD, e.to_string()))?;
    write_library(path, &bytes).map_err(|e| bad(CMD, format!("{path}: {e}")))?;
    Ok(
        json!({ "path": path, "devices": devs.len(), "wires": conns.len(), "unplaced": unplaced, "shapes": shapes.len(), "skipped": skipped, "pages": page_names.len() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCT: &str = "CHAMMQ50";
    const NODE: &str = "epix-flex-drive";

    fn place(s: &mut Session, id: &str, at: [f64; 2]) -> String {
        s.execute("csd.place", &json!({ "product": id, "at": at })).unwrap()["handle"].as_str().unwrap().to_string()
    }

    type Row = (String, String, Vec2, f64, f64);

    /// Devices as (product, label, centre, scale) sorted left to right, and wires as (from, to, link) by that order.
    fn summary(s: &Session) -> (Vec<Row>, Vec<(usize, usize, cadcraft_csd::WireLink)>) {
        let lib = s.csd_library.clone();
        let store = s.doc().unwrap().space(&s.space()).unwrap();
        let mut devs = devices(store, &lib);
        devs.sort_by(|a, b| a.insert.insert.x.total_cmp(&b.insert.insert.x));
        let rows = devs
            .iter()
            .map(|d| {
                (
                    d.product.id.clone(),
                    attr(d.insert, "NAME").to_string(),
                    Vec2::new(d.insert.insert.x, d.insert.insert.y),
                    d.insert.scale.x,
                    d.insert.scale.y,
                )
            })
            .collect();
        let pos = |h: Handle| devs.iter().position(|d| d.handle == h);
        let links = wires(store).into_iter().filter_map(|(_, l)| Some((pos(l.from)?, pos(l.to)?, l))).collect();
        (rows, links)
    }

    fn device_info(s: &mut Session, label: &str) -> Value {
        let lib = s.csd_library.clone();
        let store = s.doc().unwrap().space(&s.space()).unwrap();
        let h = devices(store, &lib).iter().find(|d| attr(d.insert, "NAME") == label).unwrap().handle;
        s.execute("csd.device.info", &json!({ "handle": h.hex() })).unwrap()
    }

    #[test]
    fn round_trips_through_the_importer() {
        let mut s = Session::new();
        let a = place(&mut s, PRODUCT, [0.0, 0.0]);
        let b = place(&mut s, NODE, [300.0, 80.0]);
        let c = place(&mut s, NODE, [600.0, -50.0]);
        let link = json!({ "from": a, "fromSide": "rgt", "to": b, "toSide": "lft", "protocol": "dmx" });
        let w = s.execute("csd.connect", &link).unwrap()["handle"].as_str().unwrap().to_string();
        s.execute("csd.wire.point.add", &json!({ "wire": w, "at": [150, 120] })).unwrap();
        s.execute("csd.wire.set", &json!({ "wire": w, "name": "Run 1" })).unwrap();
        s.execute("csd.wire.label", &json!({ "wire": w, "at": [150, 120] })).unwrap();
        let set = json!({ "handle": a, "label": "Desk", "notes": "front", "mount": c, "accessories": [{ "product": NODE, "qty": 2 }] });
        s.execute("csd.device.set", &set).unwrap();
        s.execute("csd.size", &json!({ "handles": [b], "w": 120, "h": 90 })).unwrap();
        s.execute("csd.unplaced.add", &json!({ "product": PRODUCT, "qty": 1 })).unwrap();
        s.execute("csd.project.set", &json!({ "fields": { "project": "Show", "drawnBy": "Dee", "title": "Rig" } })).unwrap();

        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-out-{}.cpsd", std::process::id()));
        let path = path.to_str().unwrap();
        let r = s.execute("csd.export.cpsd", &json!({ "path": path })).unwrap();
        assert_eq!((r["devices"].clone(), r["wires"].clone(), r["unplaced"].clone()), (json!(3), json!(1), json!(1)), "{r}");

        let mut t = Session::new();
        let r = t.execute("csd.import.cpsd", &json!({ "path": path })).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(
            (r["devices"].clone(), r["wires"].clone(), r["unplaced"].clone(), r["standIns"].clone()),
            (json!(3), json!(1), json!(1), json!(0)),
            "{r}"
        );
        assert_eq!(r["skipped"], json!([]), "{r}");

        let ((da, la), (db, lb)) = (summary(&s), summary(&t));
        assert_eq!(da.len(), db.len());
        for (x, y) in da.iter().zip(&db) {
            assert_eq!((&x.0, &x.1, x.3, x.4), (&y.0, &y.1, y.3, y.4));
            assert!(x.2.near(y.2, 1e-6), "{:?} vs {:?}", x.2, y.2);
        }
        let (x, y) = (&la[0], &lb[0]);
        assert_eq!((x.0, x.1, x.2.protocol, x.2.from_side, x.2.to_side, &x.2.name), (y.0, y.1, y.2.protocol, y.2.from_side, y.2.to_side, &y.2.name));
        assert_eq!(x.2.points.len(), 1);
        assert!(x.2.points[0].near(y.2.points[0], 1e-6));
        assert!(x.2.label_t.is_some() && y.2.label_t.is_some());

        let (ia, ib) = (device_info(&mut s, "Desk"), device_info(&mut t, "Desk"));
        assert_eq!((&ia["notes"], &ia["accessories"]), (&ib["notes"], &ib["accessories"]));
        assert!(ib["mount"].is_object());
        assert_eq!(t.execute("csd.unplaced.list", &json!({})).unwrap()["items"][0]["product"], PRODUCT);
        let f = t.execute("csd.project", &json!({})).unwrap()["fields"].clone();
        assert_eq!((&f["project"], &f["drawnBy"], &f["title"]), (&json!("Show"), &json!("Dee"), &json!("Rig")));
    }

    #[test]
    fn off_page_target_is_written_only_for_the_exported_page() {
        let mut s = Session::new();
        let a = place(&mut s, "__off-page", [0.0, 0.0]);
        let b = place(&mut s, "__off-page", [300.0, 0.0]);
        s.execute("csd.device.set", &json!({ "handle": a, "linkPage": " Canvas 1 " })).unwrap();
        s.execute("csd.device.set", &json!({ "handle": b, "linkPage": "Riser" })).unwrap();
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-link-{}.cpsd", std::process::id()));
        let path = path.to_str().unwrap();
        let r = s.execute("csd.export.cpsd", &json!({ "path": path })).unwrap();
        let file: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let _ = std::fs::remove_file(path);
        let links: Vec<_> = file["nodes"].as_array().unwrap().iter().map(|n| n.get("linkPage").cloned()).collect();
        assert_eq!(links.iter().flatten().collect::<Vec<_>>(), [&json!("page-1")]);
        assert_eq!(r["skipped"].as_array().map(Vec::len), Some(1), "{r}");
    }

    #[test]
    fn every_sheet_is_a_page_and_link_page_resolves_names() {
        let dir = std::env::temp_dir();
        let (a, b) =
            (dir.join(format!("cadcraft-pg-in-{}.cpsd", std::process::id())), dir.join(format!("cadcraft-pg-out-{}.cpsd", std::process::id())));
        std::fs::write(&a, super::super::cpsd::tests::two_page_file().to_string()).unwrap();
        let mut s = Session::new();
        s.execute("csd.import.cpsd", &json!({ "path": a.to_str().unwrap(), "page": "all" })).unwrap();
        let r = s.execute("csd.export.cpsd", &json!({ "path": b.to_str().unwrap() })).unwrap();
        let out: Value = serde_json::from_slice(&std::fs::read(&b).unwrap()).unwrap();
        assert_eq!(
            (r["devices"].clone(), r["wires"].clone(), r["pages"].clone(), r["skipped"].clone()),
            (json!(5), json!(2), json!(2), json!([])),
            "{r}"
        );
        assert_eq!(out["pages"], json!([{ "id": "page-1", "name": "Stage" }, { "id": "page-2", "name": "Drawing 2" }]));
        let nodes = out["nodes"].as_array().unwrap();
        let pages: Vec<_> = nodes.iter().map(|n| n["page"].as_str().unwrap().to_string()).collect();
        assert_eq!(pages.iter().filter(|p| *p == "page-2").count(), 2);
        let off = nodes.iter().find(|n| n["prodId"] == "__off-page").unwrap();
        assert_eq!(off["linkPage"], "page-2");
        // Each page is written from its own corner, as in the source file.
        let c = nodes.iter().find(|n| n["page"] == "page-2" && n["x"].as_f64().unwrap() < 200.0).unwrap();
        assert!((c["x"].as_f64().unwrap() - 100.0).abs() < 1e-6 && (c["y"].as_f64().unwrap() - 100.0).abs() < 1e-6, "{c}");
        assert_eq!(out["shapes"][0]["page"], "page-2");
        assert!((out["shapes"][0]["x1"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{}", out["shapes"][0]);
        assert!((out["conns"].as_array().unwrap().iter().find_map(|c| c["points"][0]["x"].as_f64()).unwrap() - 250.0).abs() < 1e-6);

        let mut t = Session::new();
        let r = t.execute("csd.import.cpsd", &json!({ "path": b.to_str().unwrap(), "page": "all" })).unwrap();
        let _ = (std::fs::remove_file(&a), std::fs::remove_file(&b));
        assert_eq!((r["devices"].clone(), r["wires"].clone(), r["skipped"].clone()), (json!(5), json!(2), json!([])), "{r}");
        let (da, db) = (summary(&s).0, summary(&t).0);
        for (x, y) in da.iter().zip(&db) {
            assert!(x.2.near(y.2, 1e-6), "{:?} vs {:?}", x.2, y.2);
        }
    }

    #[test]
    fn ortho_survives_the_round_trip() {
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-ortho-{}.cpsd", std::process::id()));
        let path = path.to_str().unwrap();
        let mut s = Session::new();
        s.execute("ortho", &json!({ "on": true })).unwrap();
        s.execute("csd.export.cpsd", &json!({ "path": path })).unwrap();
        let mut t = Session::new();
        t.execute("csd.import.cpsd", &json!({ "path": path })).unwrap();
        assert!(t.settings.orthomode);
        s.execute("ortho", &json!({ "on": false })).unwrap();
        s.execute("csd.export.cpsd", &json!({ "path": path })).unwrap();
        let file: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(file["ortho"], json!(false));
    }

    #[test]
    fn two_line_survives_the_round_trip() {
        let mut s = Session::new();
        let a = place(&mut s, PRODUCT, [0.0, 0.0]);
        s.execute("csd.device.set", &json!({ "handle": a, "twoLine": true, "label": "Desk" })).unwrap();
        let path = std::env::temp_dir().join(format!("cadcraft-cpsd-two-{}.cpsd", std::process::id()));
        let path = path.to_str().unwrap();
        s.execute("csd.export.cpsd", &json!({ "path": path })).unwrap();
        let file: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(file["nodes"][0]["twoLine"], json!(true));
        let mut t = Session::new();
        let r = t.execute("csd.import.cpsd", &json!({ "path": path })).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(r["skipped"], json!([]), "{r}");
        assert_eq!(device_info(&mut t, "Desk")["twoLine"], json!(true));
    }

    #[test]
    fn shapes_survive_the_round_trip_and_strangers_are_counted() {
        let file = json!({ "shapes": [
            { "type": "line", "x1": 40, "y1": 50, "x2": 130, "y2": 90, "color": "#ff0000" },
            { "type": "rect", "x1": 30, "y1": 30, "x2": 130, "y2": 80 },
            { "type": "ellipse", "x1": 30, "y1": 30, "x2": 50, "y2": 90 },
            { "type": "text", "x1": 30, "y1": 30, "x2": 130, "y2": 46, "text": "Hello", "fontSize": 16, "textColor": "#0000ff" },
        ] });
        let dir = std::env::temp_dir();
        let (a, b) =
            (dir.join(format!("cadcraft-shp-in-{}.cpsd", std::process::id())), dir.join(format!("cadcraft-shp-out-{}.cpsd", std::process::id())));
        std::fs::write(&a, file.to_string()).unwrap();
        let mut s = Session::new();
        s.execute("csd.import.cpsd", &json!({ "path": a.to_str().unwrap() })).unwrap();
        let r = s.execute("csd.export.cpsd", &json!({ "path": b.to_str().unwrap() })).unwrap();
        let stray = s.add_entity(cadcraft_doc::EntityKind::Point(cadcraft_doc::Point { p: cadcraft_geom::Vec3::ZERO, angle: 0.0 })).unwrap();
        s.doc_mut().unwrap().modify_entity(stray, |e| e.common.layer = "CSD_SHAPES".into()).unwrap();
        let r2 = s.execute("csd.export.cpsd", &json!({ "path": dir.join("cadcraft-shp-x.cpsd").to_str().unwrap() })).unwrap();
        let _ = std::fs::remove_file(dir.join("cadcraft-shp-x.cpsd"));
        let out: Value = serde_json::from_slice(&std::fs::read(&b).unwrap()).unwrap();
        let _ = (std::fs::remove_file(&a), std::fs::remove_file(&b));
        assert_eq!((r["shapes"].clone(), r["skipped"].clone()), (json!(4), json!([])), "{r}");
        assert_eq!(r2["shapes"], 4);
        assert_eq!(r2["skipped"].as_array().map(Vec::len), Some(1), "{r2}");
        let (got, want) = (out["shapes"].as_array().unwrap(), file["shapes"].as_array().unwrap());
        assert_eq!(got.len(), 4);
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g["type"], w["type"]);
            for k in ["x1", "y1", "x2", "y2"] {
                assert!((g[k].as_f64().unwrap() - w[k].as_f64().unwrap()).abs() < 1e-6, "{k}: {g} vs {w}");
            }
        }
        assert_eq!((&got[0]["color"], &got[3]["text"], &got[3]["textColor"]), (&json!("#ff0000"), &json!("Hello"), &json!("#0000ff")));
    }

    #[test]
    fn shape_style_survives_the_round_trip() {
        let file = json!({ "shapes": [
            { "type": "rect", "x1": 30, "y1": 30, "x2": 130, "y2": 80, "color": "#ff0000", "weight": 6, "strokeStyle": "dashed", "fill": "#00ff00", "fillOpacity": 0.5 },
            { "type": "text", "x1": 30, "y1": 30, "x2": 130, "y2": 46, "text": "Hi", "textAlign": "right", "textColor": "#0000ff" },
            { "type": "line", "x1": 0, "y1": 0, "x2": 50, "y2": 0, "strokeStyle": "dotted", "weight": 2 },
        ] });
        let dir = std::env::temp_dir();
        let (a, b) =
            (dir.join(format!("cadcraft-sty-in-{}.cpsd", std::process::id())), dir.join(format!("cadcraft-sty-out-{}.cpsd", std::process::id())));
        std::fs::write(&a, file.to_string()).unwrap();
        let mut s = Session::new();
        s.execute("csd.import.cpsd", &json!({ "path": a.to_str().unwrap() })).unwrap();
        let store = s.doc().unwrap().space(&s.space()).unwrap();
        let on: Vec<_> = store.iter().filter(|e| e.common.layer == "CSD_SHAPES").collect();
        let rect = on.iter().find(|e| matches!(e.kind, cadcraft_doc::EntityKind::LwPolyline(_))).unwrap();
        assert_eq!((rect.common.lineweight, rect.common.linetype.as_str()), (cadcraft_doc::Lineweight::Mm100(60), "DASHED"));
        let hatch = on.iter().find(|e| matches!(&e.kind, cadcraft_doc::EntityKind::Hatch(h) if h.solid)).unwrap();
        assert_eq!(hatch.common.transparency, cadcraft_doc::Transparency::Percent(50));
        let mt = on.iter().find_map(|e| if let cadcraft_doc::EntityKind::MText(m) = &e.kind { Some(m) } else { None }).unwrap();
        assert_eq!(mt.attach, 3);
        assert!(s.doc().unwrap().linetype("DASHED").is_some() && s.doc().unwrap().linetype("DOT").is_some());
        let r = s.execute("csd.export.cpsd", &json!({ "path": b.to_str().unwrap() })).unwrap();
        let out: Value = serde_json::from_slice(&std::fs::read(&b).unwrap()).unwrap();
        let _ = (std::fs::remove_file(&a), std::fs::remove_file(&b));
        assert_eq!(r["skipped"], json!([]), "{r}");
        let got = out["shapes"].as_array().unwrap();
        assert_eq!(got.len(), 3);
        let by = |t: &str| got.iter().find(|g| g["type"] == t).unwrap();
        let rc = by("rect");
        assert_eq!(
            (&rc["weight"], &rc["strokeStyle"], &rc["fill"], &rc["fillOpacity"]),
            (&json!(6.0), &json!("dashed"), &json!("#00ff00"), &json!(0.5))
        );
        assert_eq!(by("text")["textAlign"], "right");
        assert!((by("text")["x2"].as_f64().unwrap() - 130.0).abs() < 1e-6 && (by("text")["x1"].as_f64().unwrap() - 30.0).abs() < 1e-6);
        assert_eq!(by("line")["strokeStyle"], "dotted");
    }
}
