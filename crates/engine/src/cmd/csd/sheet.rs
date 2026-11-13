//! The sheet's title block, its project fields (attributes) and the line key.

use super::{
    Arc, DASH_LEN, Drawing, EntityKind, Handle, HashSet, Insert, Map, Protocol, Result, Session, Value, Vec2, attr, bad, dashed_scale, ensure_bold,
    json, product_attr, spaces, wires,
};
use cadcraft_csd::{LEGEND, MARGIN, Paper, TITLE_FIELDS, TITLE_H, TITLEBLOCK, legend_block, legend_protocols, titleblock_block};

use super::page;
use crate::cmd::point_param;
use cadcraft_doc::Space;

pub(super) fn find(s: &Session, name: &str) -> Option<(Handle, Insert)> {
    let store = s.doc().ok()?.space(&s.space())?;
    store.iter().find_map(|e| match &e.kind {
        EntityKind::Insert(i) if i.block == name => Some((e.handle, i.clone())),
        _ => None,
    })
}

fn paper_param(cmd: &str, p: &Value) -> Result<Option<Paper>> {
    match p.get("paper") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(Paper::from_id).map(Some).ok_or_else(|| bad(cmd, "`paper` must be \"tabloid\" or \"letter\"")),
    }
}

/// The paper the drawing's title block was drawn for; tabloid without one.
pub(super) fn paper_of(doc: &Drawing) -> Paper {
    doc.block(TITLEBLOCK).map_or(Paper::Tabloid, |b| Paper::of_block(b))
}

fn ensure_titleblock(s: &mut Session, at: Vec2, paper: Paper) -> Result<Handle> {
    if let Some((h, _)) = find(s, TITLEBLOCK) {
        return Ok(h);
    }
    let d = s.doc_mut()?;
    ensure_bold(d);
    if d.block(TITLEBLOCK).is_none() {
        let block = titleblock_block(paper, || d.new_handle());
        d.blocks.insert(TITLEBLOCK.into(), Arc::new(block));
    }
    crate::cmd::blocks::insert(s, TITLEBLOCK, at, 1.0, 0.0, &Map::new())
}

pub(super) fn run_titleblock(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_param(p, "at").unwrap_or(Vec2::ZERO);
    let paper = paper_param("csd.titleblock", p)?;
    let h = ensure_titleblock(s, at, paper.unwrap_or_default())?;
    if let Some(paper) = paper {
        set_paper(s, paper)?;
    }
    Ok(json!({ "handle": h.hex(), "paper": paper_of(s.doc()?).id() }))
}

/// Redraws the title block (field values live on its insert and stay) and the legend for `paper`.
fn set_paper(s: &mut Session, paper: Paper) -> Result<()> {
    if paper_of(s.doc()?) == paper {
        return Ok(());
    }
    let d = s.doc_mut()?;
    let block = titleblock_block(paper, || d.new_handle());
    d.blocks.insert(TITLEBLOCK.into(), Arc::new(block));
    sync_legend(&mut s.state_mut()?.doc);
    Ok(())
}

pub(super) fn run_paper(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.paper";
    let paper = paper_param(cmd, p)?.ok_or_else(|| bad(cmd, "`paper` (tabloid|letter) is required"))?;
    let h = find(s, TITLEBLOCK).map(|(h, _)| h).ok_or_else(|| bad(cmd, "the drawing has no title block; run csd.titleblock first"))?;
    set_paper(s, paper)?;
    Ok(json!({ "handle": h.hex(), "paper": paper.id() }))
}

pub(super) fn run_project(s: &mut Session, _: &Value) -> Result<Value> {
    let found = find(s, TITLEBLOCK);
    let fields: Map<String, Value> =
        TITLE_FIELDS.iter().map(|(key, tag)| ((*key).to_string(), json!(found.as_ref().map_or("", |(_, i)| attr(i, tag))))).collect();
    let devices = s
        .doc()?
        .space(&s.space())
        .map_or(0, |st| st.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if product_attr(i).is_some())).count());
    Ok(
        json!({ "fields": fields, "exists": found.is_some(), "paper": paper_of(s.doc()?).id(), "handle": found.map(|(h, _)| h.hex()), "devices": devices }),
    )
}

/// The bill of materials footer: the title block's filled-in project fields on one line.
pub(super) fn project_footer(s: &Session) -> Vec<String> {
    let Some((_, i)) = find(s, TITLEBLOCK) else { return Vec::new() };
    let parts: Vec<String> = [("PROJECT", "Project"), ("VENUE", "Venue"), ("DRAWNBY", "Drawn by"), ("DATE", "Date"), ("REV", "Rev")]
        .iter()
        .filter_map(|(tag, caption)| Some(attr(&i, tag).trim()).filter(|v| !v.is_empty()).map(|v| format!("{caption}: {v}")))
        .collect();
    if parts.is_empty() { Vec::new() } else { vec![parts.join("   ")] }
}

pub(super) fn run_project_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.project.set";
    let given = p.get("fields").and_then(Value::as_object).ok_or_else(|| bad(cmd, "`fields` (object) is required"))?;
    let mut edits = Vec::new();
    for (key, value) in given {
        let (_, tag) = TITLE_FIELDS.iter().find(|(k, _)| k == key).ok_or_else(|| bad(cmd, format!("unknown field \"{key}\"")))?;
        let text = value.as_str().ok_or_else(|| bad(cmd, format!("`{key}` must be text")))?;
        edits.push((*tag, text.to_string()));
    }
    let h = ensure_titleblock(s, Vec2::ZERO, Paper::default())?;
    // The fields are the drawing's, so every sheet takes them; with several sheets SHEET is "N of M".
    let sheets: Vec<Handle> = page::pages(s).iter().map(|g| g.handle).collect();
    let several = sheets.len() > 1;
    for sh in sheets {
        let edits = edits.clone();
        s.doc_mut()?.modify_entity(sh, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            for (tag, text) in edits {
                if several && tag == "SHEET" {
                    continue;
                }
                if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == tag) {
                    a.text.value = text;
                }
            }
        })?;
    }
    Ok(json!({ "handle": h.hex() }))
}

/// The protocols the wires carry, one list per page (a wire is on the page its first vertex lies on);
/// one list for the whole drawing when it has fewer than two pages.
fn used_by_page(doc: &Drawing, pages: &[page::Page]) -> Vec<Vec<Protocol>> {
    let mut used: Vec<HashSet<Protocol>> = vec![HashSet::new(); pages.len().max(1)];
    if pages.len() < 2 {
        used[0] = spaces(doc).iter().filter_map(|sp| doc.space(sp)).flat_map(wires).map(|(_, l)| l.protocol).collect();
    } else if let Some(store) = doc.space(&Space::Model) {
        for (h, l) in wires(store) {
            let Some(EntityKind::LwPolyline(p)) = store.get(h).map(|e| &e.kind) else { continue };
            let n = p.vertices.first().and_then(|v| page::page_of(doc, pages, v.p));
            if let Some(set) = n.and_then(|n| used.get_mut(n)) {
                set.insert(l.protocol);
            }
        }
    }
    used.iter().map(|u| Protocol::ALL.into_iter().filter(|p| u.contains(p)).collect()).collect()
}

/// Page `i`'s legend block: the first page keeps the plain name.
fn legend_name(i: usize) -> String {
    if i == 0 { LEGEND.into() } else { format!("{LEGEND}_{}", i + 1) }
}

/// Redefines each page's legend block when the protocols on it differ from the ones it lists, and
/// with several pages gives every sheet its own legend insert, placed on it as the first one is.
fn sync_legend(doc: &mut Arc<Drawing>) {
    let paper = paper_of(doc);
    let pages = doc.space(&Space::Model).map(page::pages_in).unwrap_or_default();
    let sets = used_by_page(doc, &pages);
    let names: Vec<String> = (0..sets.len()).map(legend_name).collect();
    let stale = |n: &str| n.starts_with(LEGEND) && !names.iter().any(|w| w == n);
    let outdated = sets.iter().zip(&names).any(|(u, n)| !doc.block(n).is_some_and(|b| legend_protocols(b) == *u && Paper::of_block(b) == paper));
    let first = doc.space(&Space::Model).and_then(|st| {
        st.iter().find_map(|e| match &e.kind {
            EntityKind::Insert(i) if i.block == LEGEND => Some(i.insert.xy()),
            _ => None,
        })
    });
    let missing = |doc: &Drawing| {
        let Some(st) = doc.space(&Space::Model) else { return Vec::new() };
        let has = |n: &str| st.iter().any(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block == n));
        (1..pages.len()).filter(|&i| !has(&names[i])).collect::<Vec<_>>()
    };
    if !outdated && !doc.blocks.keys().any(|n| stale(n)) && (first.is_none() || missing(doc).is_empty()) {
        return;
    }
    let d = Arc::make_mut(doc);
    ensure_bold(d);
    let dash = dashed_scale(d, DASH_LEN);
    for (used, name) in sets.iter().zip(&names) {
        let mut block = legend_block(paper, used, dash, || d.new_handle());
        block.name = name.clone();
        d.blocks.insert(name.clone(), Arc::new(block));
    }
    let gone: Vec<String> = d.blocks.keys().filter(|n| stale(n)).cloned().collect();
    for n in &gone {
        d.blocks.remove(n);
    }
    let strays: Vec<Handle> = d.space(&Space::Model).map_or(Vec::new(), |st| {
        st.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if gone.contains(&i.block))).map(|e| e.handle).collect()
    });
    strays.into_iter().for_each(|h| {
        d.remove_entity(h);
    });
    let Some(first) = first else { return };
    let offset = page::page_of(d, &pages, first).map_or(Vec2::new(MARGIN, MARGIN + TITLE_H), |n| first - pages[n].at);
    for i in missing(d) {
        let at = pages[i].at + offset;
        let ins = Insert { block: names[i].clone(), insert: cadcraft_geom::Vec3::new(at.x, at.y, 0.0), ..Insert::default() };
        let _ = d.add(&Space::Model, Default::default(), EntityKind::Insert(ins));
    }
}

/// Keeps an existing legend in step with the wires; does nothing (cheaply) on a drawing without one.
pub(super) fn refresh_legend(doc: &mut Arc<Drawing>) {
    if doc.block(LEGEND).is_some() {
        sync_legend(doc);
    }
}

pub(super) fn run_legend(s: &mut Session, p: &Value) -> Result<Value> {
    sync_legend(&mut s.state_mut()?.doc);
    if let Some((h, _)) = find(s, LEGEND) {
        return Ok(json!({ "handle": h.hex() }));
    }
    let above = find(s, TITLEBLOCK).map_or(Vec2::ZERO, |(_, i)| Vec2::new(i.insert.x, i.insert.y));
    let at = point_param(p, "at").unwrap_or(above + Vec2::new(MARGIN, MARGIN + TITLE_H));
    let h = crate::cmd::blocks::insert(s, LEGEND, at, 1.0, 0.0, &Map::new())?;
    Ok(json!({ "handle": h.hex() }))
}
