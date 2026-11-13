//! The second label strip of a device card: `csd.device.set {twoLine}` and `csd.devices.set`.

use cadcraft_csd::{card_size, is_builtin};
use cadcraft_doc::{EntityKind, Handle};
use serde_json::{Value, json};

use super::{device_handles, ensure_device_block, product_attr, put_attr, resolve, targets, two_line};
use crate::cmd::bad;
use crate::{Result, Session};

/// Turns the second strip on or off for the devices among `hs`, switching each to the matching
/// block. An override of the card's size (`csd.size`) grows or shrinks by the strip, as the web
/// app's does. Built-ins and devices already so are left alone; returns how many changed.
pub(super) fn set(s: &mut Session, hs: &[Handle], on: bool) -> Result<usize> {
    let library = s.csd_library.clone();
    let mut changed = 0;
    for h in device_handles(s, hs)? {
        let Some(EntityKind::Insert(i)) = s.doc()?.entity(h).map(|e| &e.kind) else { continue };
        let product = resolve(&library, product_attr(i).unwrap_or_default()).into_owned();
        if is_builtin(&product.id) || two_line(i) == on {
            continue;
        }
        let name = ensure_device_block(s, &product, on)?;
        let (old, new) = (card_size(&product, !on), card_size(&product, on));
        s.doc_mut()?.modify_entity(h, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            i.block = name;
            put_attr(i, "TWOLINE", if on { "1".into() } else { String::new() });
            i.scale.y = i.scale.y.signum() * (old.y * i.scale.y.abs() + new.y - old.y) / new.y;
        })?;
        changed += 1;
    }
    Ok(changed)
}

pub(super) fn run_set(s: &mut Session, p: &Value) -> Result<Value> {
    let on = p.get("twoLine").and_then(Value::as_bool).ok_or_else(|| bad("csd.devices.set", "`twoLine` (true or false) is required"))?;
    let hs = targets(s, p)?;
    Ok(json!({ "changed": set(s, &hs, on)? }))
}
