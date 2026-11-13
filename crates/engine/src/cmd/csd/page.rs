//! Pages: each is a title-block sheet in model space. A device is on the sheet whose frame holds
//! its insert point, so nothing is stored per device; the page name is a hidden attribute of the
//! sheet's title-block insert.

use std::sync::Arc;

use cadcraft_csd::{OFF_PAGE_ID, TITLE_FIELDS, TITLEBLOCK};
use cadcraft_doc::{Drawing, EntityKind, EntityStore, Handle, Insert, Space};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::sheet::{self, paper_of};
use super::{Result, Session, attr, bad, devices, product_attr};
use crate::cmd::str_param;

const NAME_TAG: &str = "PAGENAME";
const SHEET_TAG: &str = "SHEET";
/// Space between a sheet's right edge and the next sheet's left edge.
const GAP: f64 = 200.0;

pub(super) struct Page {
    pub handle: Handle,
    pub at: Vec2,
    pub scale: Vec2,
    /// The PAGENAME attribute, empty when the page was never named.
    pub explicit: String,
}

impl Page {
    /// The name shown to the user: the given one, else "Drawing N" (ChamSys's default).
    pub fn name(&self, index: usize) -> String {
        if self.explicit.trim().is_empty() { format!("Drawing {}", index + 1) } else { self.explicit.clone() }
    }
}

/// The title-block sheets of `store`, left to right.
pub(super) fn pages_in(store: &EntityStore) -> Vec<Page> {
    let mut v: Vec<Page> = store
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Insert(i) if i.block == TITLEBLOCK => {
                Some(Page { handle: e.handle, at: i.insert.xy(), scale: Vec2::new(i.scale.x, i.scale.y), explicit: attr(i, NAME_TAG).to_string() })
            }
            _ => None,
        })
        .collect();
    v.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
    v
}

/// The display names of the sheets in `store`, left to right.
pub fn page_names(store: &EntityStore) -> Vec<String> {
    pages_in(store).iter().enumerate().map(|(n, g)| g.name(n)).collect()
}

pub(super) fn pages(s: &Session) -> Vec<Page> {
    s.doc().ok().and_then(|d| d.space(&s.space())).map(pages_in).unwrap_or_default()
}

/// The index of the sheet whose frame holds `p`.
pub(super) fn page_of(doc: &Drawing, pages: &[Page], p: Vec2) -> Option<usize> {
    let (w, h) = paper_of(doc).size();
    pages.iter().position(|g| {
        let (x, y) = (p.x - g.at.x, p.y - g.at.y);
        (0.0..=w * g.scale.x.abs()).contains(&x) && (0.0..=h * g.scale.y.abs()).contains(&y)
    })
}

/// A page given as its 1-based number or its name.
pub(super) fn page_param(cmd: &str, p: &Value, pages: &[Page]) -> Result<usize> {
    let found = match p.get("page") {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)).filter(|i| *i < pages.len()),
        Some(Value::String(t)) => pages.iter().enumerate().position(|(i, g)| g.name(i) == *t),
        _ => None,
    };
    found.ok_or_else(|| bad(cmd, format!("`page` must be a page number from 1 to {} or a page name", pages.len())))
}

/// Sets `tag` on a title-block insert, adding it (hidden, copied from the SHEET field) when missing.
pub(super) fn set_tag(i: &mut Insert, tag: &str, value: String) {
    if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == tag) {
        a.text.value = value;
    } else if let Some(mut a) = i.attribs.iter().find(|a| a.tag == SHEET_TAG).cloned() {
        a.tag = tag.into();
        a.invisible = true;
        a.text.value = value;
        i.attribs.push(a);
    }
}

/// Names page `n` (0-based); the sheet must exist.
pub(super) fn name_page(s: &mut Session, n: usize, name: &str) -> Result<()> {
    let Some(g) = pages(s).get(n).map(|g| g.handle) else { return Ok(()) };
    s.doc_mut()?.modify_entity(g, |e| {
        if let EntityKind::Insert(i) = &mut e.kind {
            set_tag(i, NAME_TAG, name.to_string());
        }
    })?;
    Ok(())
}

/// Writes "N of M" into every sheet's SHEET field when there is more than one page.
pub(super) fn refresh_labels(doc: &mut Arc<Drawing>) {
    if doc.block(TITLEBLOCK).is_none() {
        return;
    }
    let Some(store) = doc.space(&Space::Model) else { return };
    let ps = pages_in(store);
    if ps.len() < 2 {
        return;
    }
    let stale: Vec<(Handle, String)> = ps
        .iter()
        .enumerate()
        .filter_map(|(n, g)| {
            let want = format!("{} of {}", n + 1, ps.len());
            let Some(EntityKind::Insert(i)) = store.get(g.handle).map(|e| &e.kind) else { return None };
            (attr(i, SHEET_TAG) != want).then_some((g.handle, want))
        })
        .collect();
    if stale.is_empty() {
        return;
    }
    let d = Arc::make_mut(doc);
    for (h, want) in stale {
        let _ = d.modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                set_tag(i, SHEET_TAG, want);
            }
        });
    }
}

/// A warning naming the devices that moved or were added between `before` and `doc` and sit on no sheet,
/// when their space has two or more pages (a device's page is where it lies).
pub(super) fn off_sheet_warning(before: &Drawing, doc: &Drawing) -> Option<String> {
    const NAMED: usize = 3;
    let mut off: Vec<String> = Vec::new();
    for space in super::spaces(doc) {
        let (Some(store), Some(old)) = (doc.space(&space), before.space(&space)) else { continue };
        let ps = pages_in(store);
        if ps.len() < 2 {
            continue;
        }
        for e in store.iter() {
            let EntityKind::Insert(i) = &e.kind else { continue };
            let moved = match old.get(e.handle).map(|o| &o.kind) {
                Some(EntityKind::Insert(was)) => !was.insert.xy().near(i.insert.xy(), 1e-9),
                _ => true,
            };
            if product_attr(i).is_some() && moved && page_of(doc, &ps, i.insert.xy()).is_none() {
                off.push(attr(i, "NAME").to_string());
            }
        }
    }
    let named = off.iter().take(NAMED).map(String::as_str).collect::<Vec<_>>().join(", ");
    let more = if off.len() > NAMED { ", ..." } else { "" };
    (!off.is_empty()).then(|| format!("Warning: {} device(s) are off every sheet ({named}{more}); they will not print.", off.len()))
}

pub(super) fn run_pages(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = pages(s).iter().enumerate().map(|(n, g)| json!({ "index": n + 1, "name": g.name(n), "handle": g.handle.hex() })).collect();
    Ok(json!({ "pages": list }))
}

/// Zooms the view to a page, given directly or as the off-page connector (`device`) that links to it.
pub(super) fn run_goto(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.page.goto";
    let ps = pages(s);
    let n = match p.get("device") {
        Some(_) => {
            let h = super::super::curves::handle_param(p, "device").ok_or_else(|| bad(cmd, "`device` must be a handle"))?;
            let store = s.doc()?.space(&s.space()).ok_or_else(|| bad(cmd, "no such space"))?;
            let d = devices(store, &s.csd_library).into_iter().find(|d| d.handle == h && d.product.id == OFF_PAGE_ID);
            let link = d.map(|d| attr(d.insert, "LINKPAGE").to_string()).ok_or_else(|| bad(cmd, "`device` is not an off-page connector"))?;
            page_param(cmd, &json!({ "page": link }), &ps)?
        }
        None => page_param(cmd, p, &ps)?,
    };
    let g = &ps[n];
    let (w, h) = paper_of(s.doc()?).size();
    let b = g.at + Vec2::new(w * g.scale.x.abs(), h * g.scale.y.abs());
    crate::cmd::view::zoom(s, &json!({ "mode": "window", "p1": [g.at.x, g.at.y], "p2": [b.x, b.y] }))?;
    Ok(json!({ "page": n + 1, "name": g.name(n) }))
}

/// Adds the next sheet to the right of the last; returns its handle.
pub(super) fn add_page(s: &mut Session) -> Result<Handle> {
    let cmd = "csd.page.add";
    if pages(s).is_empty() {
        sheet::run_titleblock(s, &json!({}))?;
    }
    let ps = pages(s);
    let last = ps.last().ok_or_else(|| bad(cmd, "the drawing has no title block"))?;
    let at = last.at + Vec2::new(paper_of(s.doc()?).size().0 * last.scale.x.abs() + GAP, 0.0);
    let fields: Vec<(&str, String)> = {
        let Some(EntityKind::Insert(i)) = s.doc()?.space(&s.space()).and_then(|st| st.get(last.handle)).map(|e| &e.kind) else {
            return Err(bad(cmd, "the last sheet vanished"));
        };
        TITLE_FIELDS.iter().filter(|(k, _)| *k != "sheet").map(|(_, tag)| (*tag, attr(i, tag).to_string())).collect()
    };
    let h = crate::cmd::blocks::insert(s, TITLEBLOCK, at, 1.0, 0.0, &Default::default())?;
    // The project fields are the drawing's, so a new sheet starts with the last one's.
    s.doc_mut()?.modify_entity(h, |e| {
        let EntityKind::Insert(i) = &mut e.kind else { return };
        for (tag, v) in fields {
            if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == tag) {
                a.text.value = v;
            }
        }
    })?;
    Ok(h)
}

pub(super) fn run_add(s: &mut Session, p: &Value) -> Result<Value> {
    let h = add_page(s)?;
    let n = pages(s).len();
    if let Some(name) = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        name_page(s, n - 1, name)?;
    }
    let library = s.csd_library.clone();
    let ortho = s.settings.orthomode;
    super::refresh(&mut s.state_mut()?.doc, &library, ortho);
    Ok(json!({ "handle": h.hex(), "index": n, "pages": n }))
}

pub(super) fn run_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.page.rename";
    let ps = pages(s);
    let n = page_param(cmd, p, &ps)?;
    let name = str_param(p, "name").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "`name` is required"))?.to_string();
    let old = ps[n].name(n);
    let links: Vec<Handle> = {
        let store = s.doc()?.space(&s.space()).ok_or_else(|| bad(cmd, "no such space"))?;
        devices(store, &s.csd_library).iter().filter(|d| d.product.id == OFF_PAGE_ID && attr(d.insert, "LINKPAGE") == old).map(|d| d.handle).collect()
    };
    name_page(s, n, &name)?;
    for l in &links {
        s.doc_mut()?.modify_entity(*l, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                super::put_attr(i, "LINKPAGE", name.clone());
            }
        })?;
    }
    Ok(json!({ "handle": ps[n].handle.hex(), "name": name, "links": links.len() }))
}

pub(super) fn run_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.page.remove";
    let ps = pages(s);
    let n = page_param(cmd, p, &ps)?;
    if ps.len() < 2 {
        return Err(bad(cmd, "a drawing keeps at least one page"));
    }
    let held = {
        let d = s.doc()?;
        let store = d.space(&s.space()).ok_or_else(|| bad(cmd, "no such space"))?;
        store
            .iter()
            .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if product_attr(i).is_some() && page_of(d, &ps, i.insert.xy()) == Some(n)))
            .count()
    };
    if held > 0 {
        return Err(bad(cmd, format!("page {} holds {held} device(s); move or delete them first", n + 1)));
    }
    let d = s.doc_mut()?;
    d.remove_entity(ps[n].handle);
    // The last page goes back to the user's own Sheet field.
    if ps.len() == 2 {
        d.modify_entity(ps[1 - n].handle, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                set_tag(i, SHEET_TAG, String::new());
            }
        })?;
    }
    Ok(json!({ "pages": ps.len() - 1 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCT: &str = "CHAMMQ50";

    fn place(s: &mut Session, id: &str, at: [f64; 2]) -> String {
        s.execute("csd.place", &json!({ "product": id, "at": at })).unwrap()["handle"].as_str().unwrap().to_string()
    }

    fn field(s: &Session, h: Handle, tag: &str) -> String {
        match &s.doc().unwrap().space(&s.space()).unwrap().get(h).unwrap().kind {
            EntityKind::Insert(i) => attr(i, tag).to_string(),
            _ => String::new(),
        }
    }

    fn sheet_fields(s: &Session, tag: &str) -> Vec<String> {
        pages(s).iter().map(|g| field(s, g.handle, tag)).collect()
    }

    #[test]
    fn add_lists_pages_left_to_right_and_labels_the_sheets() {
        let mut s = Session::new();
        s.execute("csd.project.set", &json!({ "fields": { "project": "Show", "sheet": "A1" } })).unwrap();
        assert_eq!(sheet_fields(&s, SHEET_TAG), ["A1"], "a single page keeps the user's own field");
        s.execute("csd.page.add", &json!({ "name": "Riser" })).unwrap();
        s.execute("csd.page.add", &json!({})).unwrap();
        let r = s.execute("csd.pages", &json!({})).unwrap();
        let names: Vec<_> = r["pages"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["Drawing 1", "Riser", "Drawing 3"]);
        assert_eq!(sheet_fields(&s, SHEET_TAG), ["1 of 3", "2 of 3", "3 of 3"]);
        let xs: Vec<f64> = pages(&s).iter().map(|g| g.at.x).collect();
        assert_eq!(xs, [0.0, 1832.0, 3664.0]);
        s.execute("csd.project.set", &json!({ "fields": { "venue": "Hall" } })).unwrap();
        assert_eq!(sheet_fields(&s, "VENUE"), ["Hall"; 3], "project fields are shared by every sheet");
        assert_eq!(sheet_fields(&s, "PROJECT"), ["Show"; 3]);
    }

    #[test]
    fn remove_refuses_a_page_with_devices_and_restores_the_single_sheet_field() {
        let mut s = Session::new();
        s.execute("csd.page.add", &json!({ "name": "Two" })).unwrap();
        let h = place(&mut s, PRODUCT, [1932.0, 500.0]);
        let e = s.execute("csd.page.remove", &json!({ "page": 2 })).unwrap_err().to_string();
        assert!(e.contains("holds 1 device"), "{e}");
        assert!(s.execute("csd.page.remove", &json!({ "page": 3 })).is_err());
        s.execute("erase", &json!({ "handles": [h] })).unwrap();
        let r = s.execute("csd.page.remove", &json!({ "page": "Two" })).unwrap();
        assert_eq!(r["pages"], 1);
        assert_eq!(sheet_fields(&s, SHEET_TAG), [""]);
        assert!(s.execute("csd.page.remove", &json!({ "page": 1 })).is_err(), "the last page stays");
    }

    #[test]
    fn moving_a_device_off_every_sheet_warns_only_with_two_pages() {
        let mut s = Session::new();
        let h = place(&mut s, PRODUCT, [100.0, 500.0]);
        let mv = |s: &mut Session, to: f64| s.execute("move", &json!({ "handles": [h], "delta": [to, 0] })).unwrap();
        let warned = |s: &Session| s.log.iter().any(|m| m.contains("off every sheet"));
        s.execute("csd.titleblock", &json!({})).unwrap();
        mv(&mut s, 1650.0);
        assert!(!warned(&s), "a single page has no other sheet to be on");
        s.execute("csd.page.add", &json!({})).unwrap();
        mv(&mut s, 100.0);
        assert!(!warned(&s), "still on a sheet");
        mv(&mut s, -2000.0);
        assert!(warned(&s));
    }

    #[test]
    fn placing_a_device_off_every_sheet_warns_only_with_two_pages() {
        let mut s = Session::new();
        s.execute("csd.titleblock", &json!({})).unwrap();
        let warned = |s: &Session| s.log.iter().any(|m| m.contains("off every sheet"));
        place(&mut s, PRODUCT, [-2000.0, 500.0]);
        assert!(!warned(&s), "a single page has no other sheet to be on");
        s.execute("csd.page.add", &json!({})).unwrap();
        place(&mut s, PRODUCT, [100.0, 500.0]);
        assert!(!warned(&s), "on a sheet");
        place(&mut s, PRODUCT, [-2000.0, 500.0]);
        assert!(warned(&s));
    }

    #[test]
    fn connect_refuses_endpoints_on_different_sheets() {
        let mut s = Session::new();
        s.execute("csd.page.add", &json!({})).unwrap();
        let a = place(&mut s, PRODUCT, [100.0, 500.0]);
        let b = place(&mut s, PRODUCT, [400.0, 500.0]);
        let c = place(&mut s, PRODUCT, [2200.0, 500.0]);
        let link = |from: &str, to: &str| json!({ "from": from, "fromSide": "rgt", "to": to, "toSide": "lft", "protocol": "dmx" });
        s.execute("csd.connect", &link(&a, &b)).unwrap();
        let e = s.execute("csd.connect", &link(&a, &c)).unwrap_err().to_string();
        assert!(e.contains("off-page connector") && e.contains("page 1") && e.contains("page 2"), "{e}");
    }

    #[test]
    fn goto_zooms_to_the_sheet_of_a_page_or_an_off_page_link() {
        let mut s = Session::new();
        s.execute("csd.page.add", &json!({ "name": "Riser" })).unwrap();
        let link = place(&mut s, OFF_PAGE_ID, [100.0, 500.0]);
        let centre = |s: &Session| s.state().unwrap().zoom_frame().center;
        s.execute("csd.page.goto", &json!({ "page": "Riser" })).unwrap();
        assert!(centre(&s).x > 1832.0, "{:?}", centre(&s));
        s.execute("csd.page.goto", &json!({ "page": 1 })).unwrap();
        assert!(centre(&s).x < 1832.0);
        s.execute("csd.device.set", &json!({ "handle": link, "linkPage": "Riser" })).unwrap();
        s.execute("csd.page.goto", &json!({ "device": link })).unwrap();
        assert!(centre(&s).x > 1832.0);
        s.execute("csd.device.set", &json!({ "handle": link, "linkPage": "Nowhere" })).unwrap();
        assert!(s.execute("csd.page.goto", &json!({ "device": link })).is_err());
    }

    #[test]
    fn renaming_a_page_moves_the_off_page_links_to_it() {
        let mut s = Session::new();
        s.execute("csd.page.add", &json!({})).unwrap();
        let link = place(&mut s, OFF_PAGE_ID, [100.0, 500.0]);
        let other = place(&mut s, OFF_PAGE_ID, [300.0, 500.0]);
        let h = |t: &str| Handle::parse_hex(t).unwrap();
        for (handle, to) in [(&link, "Drawing 2"), (&other, "Elsewhere")] {
            let set = |e: &mut cadcraft_doc::Entity| {
                if let EntityKind::Insert(i) = &mut e.kind {
                    super::super::put_attr(i, "LINKPAGE", to.into());
                }
            };
            s.doc_mut().unwrap().modify_entity(h(handle), set).unwrap();
        }
        let r = s.execute("csd.page.rename", &json!({ "page": 2, "name": "Riser" })).unwrap();
        assert_eq!(r["links"], 1);
        assert_eq!((field(&s, h(&link), "LINKPAGE"), field(&s, h(&other), "LINKPAGE")), ("Riser".to_string(), "Elsewhere".to_string()));
        assert!(s.execute("csd.page.rename", &json!({ "page": 9, "name": "x" })).is_err());
    }
}
