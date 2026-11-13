//! Commands over several devices at once: summary and batch labels.

use cadcraft_csd::{CARD_H, is_builtin};
use cadcraft_doc::{EntityKind, Handle};
use serde_json::{Value, json};

use super::{
    MAX_ACCESSORY_LINES, accessories_of, apply_label, card_of, device_handles, find_product, product_attr, put_attr, resolve, str_param, targets,
    two_line,
};
use crate::cmd::bad;
use crate::{Result, Session};

/// `Some(v)` when every value is the same.
fn common(mut vals: impl Iterator<Item = f64>) -> Option<f64> {
    let first = vals.next()?;
    vals.all(|v| (v - first).abs() < 1e-9).then_some(first)
}

pub(super) fn run_info(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = device_handles(s, &targets(s, p)?)?;
    let library = s.csd_library.clone();
    let d = s.doc()?;
    let inserts: Vec<_> = hs
        .iter()
        .filter_map(|h| match &d.entity(*h)?.kind {
            EntityKind::Insert(i) => Some(i),
            _ => None,
        })
        .collect();
    let mut products: Vec<(String, String, String, usize)> = Vec::new();
    for i in &inserts {
        let id = product_attr(i).unwrap_or_default();
        match products.iter_mut().find(|x| x.0 == id) {
            Some(x) => x.3 += 1,
            None => {
                let pr = resolve(&library, id);
                products.push((id.to_string(), pr.name.clone(), pr.code.clone(), 1));
            }
        }
    }
    let products: Vec<Value> =
        products.into_iter().map(|(id, name, code, count)| json!({ "id": id, "name": name, "code": code, "count": count })).collect();
    let strips: Vec<bool> = inserts.iter().filter(|i| !product_attr(i).is_some_and(is_builtin)).map(|i| two_line(i)).collect();
    Ok(json!({
        "count": inserts.len(),
        "twoLine": { "devices": strips.len(), "on": strips.iter().filter(|b| **b).count() },
        "products": products,
        "size": {
            "w": common(inserts.iter().map(|i| card_of(i, &library).x * i.scale.x.abs())),
            "h": common(inserts.iter().map(|i| card_of(i, &library).y * i.scale.y.abs())),
        },
    }))
}

/// Top to bottom, left to right: cards whose centres are within half a card height share a row.
fn reading_order(mut at: Vec<(Handle, f64, f64)>, k: f64) -> Vec<Handle> {
    at.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut rows: Vec<Vec<(Handle, f64, f64)>> = Vec::new();
    for c in at {
        match rows.last_mut() {
            Some(r) if r.first().is_some_and(|f| f.2 - c.2 <= CARD_H * k / 2.0) => r.push(c),
            _ => rows.push(vec![c]),
        }
    }
    rows.into_iter()
        .flat_map(|mut r| {
            r.sort_by(|a, b| a.1.total_cmp(&b.1));
            r.into_iter().map(|c| c.0)
        })
        .collect()
}

pub(super) fn run_label(s: &mut Session, p: &Value) -> Result<Value> {
    let base = str_param(p, "base").ok_or_else(|| bad("csd.devices.label", "`base` (text) is required"))?;
    let start = match p.get("start") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| bad("csd.devices.label", "`start` must be an integer"))?),
    };
    let pad = match p.get("pad") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().filter(|n| *n <= 12).ok_or_else(|| bad("csd.devices.label", "`pad` must be an integer from 0 to 12"))? as usize,
    };
    let hs = device_handles(s, &targets(s, p)?)?;
    let at: Vec<_> = {
        let d = s.doc()?;
        hs.iter()
            .filter_map(|h| match &d.entity(*h)?.kind {
                EntityKind::Insert(i) => Some((*h, i.insert.x, i.insert.y)),
                _ => None,
            })
            .collect()
    };
    let order = if start.is_some() { reading_order(at, super::units::px(s.doc()?)) } else { at.into_iter().map(|c| c.0).collect() };
    // Work every label out first so a bad `start` changes nothing.
    let mut named = Vec::new();
    for (k, h) in order.into_iter().enumerate() {
        let label = match start {
            Some(n) => {
                let num = n.checked_add(k as i64).ok_or_else(|| bad("csd.devices.label", "`start` is too large"))?;
                format!("{base}{num:0pad$}")
            }
            None => base.to_string(),
        };
        named.push((h, label));
    }
    let library = s.csd_library.clone();
    let d = s.doc_mut()?;
    for (h, label) in &named {
        d.modify_entity(*h, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            apply_label(i, label.clone(), &resolve(&library, product_attr(i).unwrap_or_default()));
        })?;
    }
    let labels: Vec<Value> = named.iter().map(|(h, l)| json!({ "handle": h.hex(), "label": l })).collect();
    Ok(json!({ "labels": labels }))
}

/// Puts one of a library product on every device as an accessory: a line it already has goes back
/// to `qty`, and a device already at the line limit is skipped.
pub(super) fn run_accessory(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.devices.accessory";
    let id = str_param(p, "product").filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "`product` (library product id) is required"))?;
    if is_builtin(id) || find_product(&s.csd_library, id).is_none() {
        return Err(bad(cmd, "`product` is not a library product"));
    }
    let qty = match p.get("qty") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().filter(|q| (1..=100_000).contains(q)).ok_or_else(|| bad(cmd, "`qty` must be a whole number from 1 to 100000"))?,
    };
    let hs = device_handles(s, &targets(s, p)?)?;
    let mut changed = 0;
    let d = s.doc_mut()?;
    for h in hs {
        d.modify_entity(h, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            let mut lines = accessories_of(i);
            if let Some(l) = lines.iter_mut().find(|l| l.0 == id) {
                l.1 = qty;
            } else if lines.len() < MAX_ACCESSORY_LINES {
                lines.push((id.to_string(), qty));
            } else {
                return;
            }
            put_attr(i, "ACCS", serde_json::to_string(&lines).unwrap_or_default());
            changed += 1;
        })?;
    }
    Ok(json!({ "changed": changed }))
}
