//! The sheet's title block, its project fields (attributes) and the line key.

use super::{
    Arc, DASH_LEN, Drawing, EntityKind, Handle, HashSet, Insert, Map, Protocol, Result, Session, Value, Vec2, attr, bad, dashed_scale, ensure_bold,
    json, product_attr, spaces, wires,
};
use cadcraft_csd::{LEGEND, MARGIN, Paper, TITLE_FIELDS, TITLE_H, TITLEBLOCK, legend_block, legend_protocols, titleblock_block};

use crate::cmd::point_param;

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

/// Grows the sheet (title block and line keys) about its corner until the rest of the drawing fits
/// inside its frame, centred; a drawing that already fits leaves it alone.
pub(super) fn run_titleblock_fit(s: &mut Session, _: &Value) -> Result<Value> {
    let cmd = "csd.titleblock.fit";
    let (tb, ins) = find(s, TITLEBLOCK).ok_or_else(|| bad(cmd, "the drawing has no title block; run csd.titleblock first"))?;
    let k0 = ins.scale.x;
    if !(k0.is_finite() && k0 > 0.0) {
        return Err(bad(cmd, "the title block's scale is not a positive number"));
    }
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| bad(cmd, "no such space"))?;
    let sheet: Vec<Handle> = store
        .iter()
        .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block == TITLEBLOCK || i.block.starts_with(LEGEND)))
        .map(|e| e.handle)
        .collect();
    let b = store
        .iter()
        .filter(|e| !sheet.contains(&e.handle))
        .map(|e| cadcraft_doc::entity_bounds(d, e, 0))
        .filter(|b| !b.is_empty() && b.min.is_finite() && b.max.is_finite())
        .fold(cadcraft_geom::Bounds2::EMPTY, |a, b| a.union(&b));
    let (w, h) = paper_of(d).size();
    let (aw, ah) = (w - 2.0 * MARGIN, h - 2.0 * MARGIN - TITLE_H);
    let origin = ins.insert.xy();
    let inside = |k: f64| {
        let lo = origin + Vec2::new(MARGIN, MARGIN + TITLE_H) * k;
        b.min.x >= lo.x && b.min.y >= lo.y && b.max.x <= lo.x + aw * k && b.max.y <= lo.y + ah * k
    };
    if b.is_empty() || inside(k0) {
        return Ok(json!({ "handle": tb.hex(), "scale": k0, "changed": false }));
    }
    let k = ((b.width() / aw).max(b.height() / ah).max(k0) * 100.0).ceil() / 100.0;
    let to = Vec2::new(b.min.x - (aw * k - b.width()) / 2.0 - MARGIN * k, b.min.y - (ah * k - b.height()) / 2.0 - (MARGIN + TITLE_H) * k);
    let m = cadcraft_geom::Mat3::scale_about(origin, k / k0).then(cadcraft_geom::Mat3::translate(to - origin));
    crate::cmd::modify::transform_entities(s, &sheet, &m, false)?;
    Ok(json!({ "handle": tb.hex(), "scale": k, "changed": true }))
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
    let paper = paper_of(doc);
    if doc.block(LEGEND).is_some_and(|b| legend_protocols(b) == used && Paper::of_block(b) == paper) {
        return;
    }
    let d = Arc::make_mut(doc);
    ensure_bold(d);
    let dash = dashed_scale(d, DASH_LEN);
    let block = legend_block(paper, &used, dash, || d.new_handle());
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
