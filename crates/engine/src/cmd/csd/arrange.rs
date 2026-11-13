//! Selecting and arranging devices by their card edges.

use cadcraft_csd::{CARD_H, CARD_W};
use cadcraft_doc::{EntityKind, Handle, Insert};
use serde_json::{Value, json};

use super::{device_handles, devices, handle_param, point_req, str_param, targets};
use crate::cmd::bad;
use crate::{Result, Session};

/// A card's centre and the extents of its (rotated, scaled) bounding box.
struct Card {
    handle: Handle,
    x: f64,
    y: f64,
    hw: f64,
    hh: f64,
}

fn card(handle: Handle, i: &Insert) -> Card {
    let (w, t) = (CARD_W / 2.0 * i.scale.x.abs(), CARD_H / 2.0 * i.scale.y.abs());
    let (c, n) = (i.rotation.cos().abs(), i.rotation.sin().abs());
    Card { handle, x: i.insert.x, y: i.insert.y, hw: c * w + n * t, hh: n * w + c * t }
}

fn cards(s: &Session, p: &Value) -> Result<Vec<Card>> {
    let hs = device_handles(s, &targets(s, p)?)?;
    let d = s.doc()?;
    Ok(hs
        .into_iter()
        .filter_map(|h| match &d.entity(h)?.kind {
            EntityKind::Insert(i) => Some(card(h, i)),
            _ => None,
        })
        .collect())
}

fn place(s: &mut Session, moves: &[(Handle, f64, f64)]) -> Result<Value> {
    let d = s.doc_mut()?;
    for (h, x, y) in moves {
        d.modify_entity(*h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.insert.x = *x;
                i.insert.y = *y;
            }
        })?;
    }
    Ok(json!({ "moved": moves.len() }))
}

pub(super) fn run_select_product(s: &mut Session, p: &Value) -> Result<Value> {
    let h = handle_param(p, "device").ok_or_else(|| bad("csd.select.product", "`device` (object handle) is required"))?;
    let library = s.csd_library.clone();
    let hs: Vec<Handle> = {
        let all = devices(&s.doc()?.model, &library);
        let product =
            all.iter().find(|x| x.handle == h).map(|x| x.product.id.as_str()).ok_or_else(|| bad("csd.select.product", "`device` is not a device"))?;
        all.iter().filter(|x| x.product.id == product).map(|x| x.handle).collect()
    };
    let count = hs.len();
    s.set_selection(hs);
    Ok(json!({ "count": count }))
}

pub(super) fn run_align(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = str_param(p, "mode").unwrap_or("");
    let cs = cards(s, p)?;
    if cs.len() < 2 {
        return Err(bad("csd.align", "needs at least two devices"));
    }
    let fold = |f: fn(f64, f64) -> f64, v: &dyn Fn(&Card) -> f64| cs.iter().map(v).reduce(f).unwrap_or(0.0);
    let moves: Vec<_> = match mode {
        "left" => {
            let e = fold(f64::min, &|c| c.x - c.hw);
            cs.iter().map(|c| (c.handle, e + c.hw, c.y)).collect()
        }
        "right" => {
            let e = fold(f64::max, &|c| c.x + c.hw);
            cs.iter().map(|c| (c.handle, e - c.hw, c.y)).collect()
        }
        "top" => {
            let e = fold(f64::max, &|c| c.y + c.hh);
            cs.iter().map(|c| (c.handle, c.x, e - c.hh)).collect()
        }
        "bottom" => {
            let e = fold(f64::min, &|c| c.y - c.hh);
            cs.iter().map(|c| (c.handle, c.x, e + c.hh)).collect()
        }
        _ => return Err(bad("csd.align", "`mode` must be left, top, right or bottom")),
    };
    place(s, &moves)
}

pub(super) fn run_distribute(s: &mut Session, p: &Value) -> Result<Value> {
    let horizontal = match str_param(p, "axis") {
        Some("x") => true,
        Some("y") => false,
        _ => return Err(bad("csd.distribute", "`axis` must be x or y")),
    };
    let mut cs = cards(s, p)?;
    if cs.len() < 3 {
        return Err(bad("csd.distribute", "needs at least three devices"));
    }
    // (centre, half extent) along the axis.
    let along = |c: &Card| if horizontal { (c.x, c.hw) } else { (c.y, c.hh) };
    cs.sort_by(|a, b| along(a).0.total_cmp(&along(b).0));
    let (first, last) = (along(&cs[0]), along(&cs[cs.len() - 1]));
    let widths: f64 = cs.iter().map(|c| 2.0 * along(c).1).sum();
    let gap = ((last.0 + last.1) - (first.0 - first.1) - widths) / (cs.len() - 1) as f64;
    let mut edge = first.0 - first.1;
    let moves: Vec<_> = cs
        .iter()
        .map(|c| {
            let hw = along(c).1;
            let at = edge + hw;
            edge += 2.0 * hw + gap;
            if horizontal { (c.handle, at, c.y) } else { (c.handle, c.x, at) }
        })
        .collect();
    place(s, &moves)
}

/// The topmost device whose card covers `at`.
pub(super) fn run_device_at(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_req("csd.device.at", p, "at")?;
    let library = s.csd_library.clone();
    let hit = devices(&s.doc()?.model, &library)
        .iter()
        .map(|d| card(d.handle, d.insert))
        .rfind(|c| (at.x - c.x).abs() <= c.hw && (at.y - c.y).abs() <= c.hh);
    Ok(json!({ "handle": hit.map(|c| c.handle.hex()) }))
}
