//! Drag to mount: a single device moved so its card lies wholly inside another device's card is
//! mounted in it, and one moved out of its host is unmounted (the web app's `mountTargetAt`).

use std::sync::Arc;

use cadcraft_csd::Product;
use cadcraft_doc::{Drawing, EntityKind, EntityStore, Handle, Insert};
use cadcraft_geom::Vec2;

use super::{card_of, mount_cycles, mount_of, product_attr, put_attr, spaces};

/// The card's axis-aligned box as `(min, max)`; the insert is its centre.
fn card_box(i: &Insert, products: &[Product]) -> (Vec2, Vec2) {
    let c = card_of(i, products);
    let (w, h) = (c.x * i.scale.x.abs(), c.y * i.scale.y.abs());
    let (s, k) = (i.rotation.sin().abs(), i.rotation.cos().abs());
    let half = Vec2::new((w * k + h * s) / 2.0, (w * s + h * k) / 2.0);
    (i.insert.xy() - half, i.insert.xy() + half)
}

/// The innermost device whose card wholly contains `mine`, other than `me` and what is mounted in it.
fn host_of(store: &EntityStore, products: &[Product], me: Handle, mine: (Vec2, Vec2)) -> Option<Handle> {
    store
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(i) = &e.kind else { return None };
            product_attr(i)?;
            let (lo, hi) = card_box(i, products);
            let inside = lo.x <= mine.0.x && lo.y <= mine.0.y && mine.1.x <= hi.x && mine.1.y <= hi.y;
            (e.handle != me && inside && !mount_cycles(store, me, e.handle)).then_some((e.handle, (hi.x - lo.x) * (hi.y - lo.y)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// If exactly one device moved between `before` and `doc`, mounts it in the card it now lies
/// inside, or unmounts it when there is none. Several moving at once is a selection move, which
/// the web app never re-parents either.
pub(super) fn auto_mount(before: &Drawing, doc: &mut Arc<Drawing>, products: &[Product]) {
    for space in spaces(doc) {
        let (Some(store), Some(old)) = (doc.space(&space), before.space(&space)) else { continue };
        let moved: Vec<Handle> = store
            .iter()
            .filter_map(|e| {
                let EntityKind::Insert(i) = &e.kind else { return None };
                product_attr(i)?;
                let Some(EntityKind::Insert(was)) = old.get(e.handle).map(|o| &o.kind) else { return None };
                (!was.insert.xy().near(i.insert.xy(), 1e-9)).then_some(e.handle)
            })
            .take(2)
            .collect();
        let [h] = moved[..] else { continue };
        let Some(EntityKind::Insert(i)) = store.get(h).map(|e| &e.kind) else { continue };
        let host = host_of(store, products, h, card_box(i, products));
        if host == mount_of(store, i) {
            continue;
        }
        let value = host.map(|p| p.hex()).unwrap_or_default();
        let _ = Arc::make_mut(doc).modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                put_attr(i, "MOUNT", value);
            }
        });
    }
}
