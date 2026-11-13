//! The sheet's title block, its project fields (attributes) and the line key.

use super::{
    Arc, DASH_LEN, Drawing, EntityKind, Handle, HashSet, Insert, Map, Protocol, Result, Session, Value, Vec2, attr, bad, dashed_scale, ensure_bold,
    json, product_attr, spaces, wires,
};
use cadcraft_csd::{LEGEND, MARGIN, TITLE_FIELDS, TITLE_H, TITLEBLOCK, legend_block, legend_protocols, titleblock_block};

use crate::cmd::point_param;

fn find(s: &Session, name: &str) -> Option<(Handle, Insert)> {
    let store = s.doc().ok()?.space(&s.space())?;
    store.iter().find_map(|e| match &e.kind {
        EntityKind::Insert(i) if i.block == name => Some((e.handle, i.clone())),
        _ => None,
    })
}

fn ensure_titleblock(s: &mut Session, at: Vec2) -> Result<Handle> {
    if let Some((h, _)) = find(s, TITLEBLOCK) {
        return Ok(h);
    }
    let d = s.doc_mut()?;
    ensure_bold(d);
    if d.block(TITLEBLOCK).is_none() {
        let block = titleblock_block(|| d.new_handle());
        d.blocks.insert(TITLEBLOCK.into(), Arc::new(block));
    }
    crate::cmd::blocks::insert(s, TITLEBLOCK, at, 1.0, 0.0, &Map::new())
}

pub(super) fn run_titleblock(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_param(p, "at").unwrap_or(Vec2::ZERO);
    Ok(json!({ "handle": ensure_titleblock(s, at)?.hex() }))
}

pub(super) fn run_project(s: &mut Session, _: &Value) -> Result<Value> {
    let found = find(s, TITLEBLOCK);
    let fields: Map<String, Value> =
        TITLE_FIELDS.iter().map(|(key, tag)| ((*key).to_string(), json!(found.as_ref().map_or("", |(_, i)| attr(i, tag))))).collect();
    let devices = s
        .doc()?
        .space(&s.space())
        .map_or(0, |st| st.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if product_attr(i).is_some())).count());
    Ok(json!({ "fields": fields, "exists": found.is_some(), "handle": found.map(|(h, _)| h.hex()), "devices": devices }))
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
    let h = ensure_titleblock(s, Vec2::ZERO)?;
    s.doc_mut()?.modify_entity(h, |e| {
        let EntityKind::Insert(i) = &mut e.kind else { return };
        for (tag, text) in edits {
            if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == tag) {
                a.text.value = text;
            }
        }
    })?;
    Ok(json!({ "handle": h.hex() }))
}

/// The protocols any wire in the drawing carries.
fn used_protocols(doc: &Drawing) -> Vec<Protocol> {
    let used: HashSet<Protocol> = spaces(doc).iter().filter_map(|sp| doc.space(sp)).flat_map(wires).map(|(_, l)| l.protocol).collect();
    Protocol::ALL.into_iter().filter(|p| used.contains(p)).collect()
}

/// Redefines the legend block when the protocols in use differ from the ones it lists.
fn sync_legend(doc: &mut Arc<Drawing>) {
    let used = used_protocols(doc);
    if doc.block(LEGEND).is_some_and(|b| legend_protocols(b) == used) {
        return;
    }
    let d = Arc::make_mut(doc);
    ensure_bold(d);
    let dash = dashed_scale(d, DASH_LEN);
    let block = legend_block(&used, dash, || d.new_handle());
    d.blocks.insert(LEGEND.into(), Arc::new(block));
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
