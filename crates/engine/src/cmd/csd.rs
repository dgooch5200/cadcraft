//! Lighting-control system designer: devices are block references of `CSD_` blocks, wires are
//! polylines whose xdata ([`WireLink`]) names the two devices they join. The post-command pass
//! keeps every wire on its devices' ports.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cadcraft_csd::{
    Deco, Product, Protocol, Side, Slot, WIRE_W, WireLink, block_name, can_connect, deco_block, deco_block_name, deco_from_xdata, deco_points,
    deco_xdata, device_block, edge_frame, fan_order, library_json, parse_library, parse_products, port_point, product_record, route, truncate_label,
    world_side,
};
use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{Common, Drawing, Entity, EntityKind, EntityStore, Handle, Insert, Layer, LwPolyline, Space};
use cadcraft_geom::{PolyVertex, Vec2};
use serde_json::{Map, Value, json};

use super::curves::handle_param;

mod place;
use super::{CommandSpec, bad, point_req, str_param};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("csd.place", "Place Device", run_place)
            .params("{product: id of a library product, at: [x,y], label? (default the product code)} → {handle}; interactive: places the armed product at each point")
            .interactive(place::PlaceM::new),
        CommandSpec::new("csd.arm", "Arm Product", place::run_arm).params("{product: id of a library product} → {product}").noundo(),
        CommandSpec::new("csd.connect", "Connect Devices", run_connect)
            .params("{from: handle, fromSide: top|bot|lft|rgt, to: handle, toSide, protocol: network|netdata|10scene|dmx|0-10v|contact|power|midi|epix3p|epix4p} → {handle}"),
        CommandSpec::new("csd.netlist", "Device Netlist", run_netlist)
            .params("{} → {devices: [{handle, product, label, at}], wires: [{handle, from, fromSide, to, toSide, protocol}]}")
            .noundo(),
        CommandSpec::new("csd.library.load", "Load Product Library", run_library_load)
            .params("{path: JSON library file} → {count}")
            .noundo(),
        CommandSpec::new("csd.library.list", "List Products", run_library_list)
            .params("{query? (id, name or code contains), category?} → {products: [records without img], categories: [cat, first-seen order]}")
            .noundo(),
        CommandSpec::new("csd.library.set", "Set Product Library", run_library_set)
            .params("{products: [records]} → {count}; a record without `img` (or other extra keys) keeps the old product's")
            .noundo(),
        CommandSpec::new("csd.library.save", "Save Product Library", run_library_save)
            .params("{path} → {path, count}")
            .noundo(),
    ]
}

/// A device on the drawing: a `CSD_` block reference whose `PRODUCT` attribute names a known product.
struct Device<'a> {
    handle: Handle,
    product: &'a Product,
    insert: &'a Insert,
}

fn is_device_block(i: &Insert) -> bool {
    i.block.starts_with("CSD_")
}

fn devices<'a>(store: &'a EntityStore, products: &'a [Product]) -> Vec<Device<'a>> {
    store
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(insert) = &e.kind else { return None };
            if !is_device_block(insert) {
                return None;
            }
            let id = insert.attribs.iter().find(|a| a.tag == "PRODUCT")?.text.value.as_str();
            Some(Device { handle: e.handle, product: products.iter().find(|p| p.id == id)?, insert })
        })
        .collect()
}

fn wires(store: &EntityStore) -> Vec<(Handle, WireLink)> {
    store
        .iter()
        .filter(|e| !e.common.xdata.is_empty())
        .filter_map(|e| Some((e.handle, WireLink::from_xdata(&e.common.xdata).filter(|_| matches!(e.kind, EntityKind::LwPolyline(_)))?)))
        .collect()
}

/// The linetype dashed things use, and the length of a dash on a wire and on a missing product's border.
const DASH: &str = "DASHED2";
const DASH_LEN: f64 = 7.0;
const BORDER_DASH: f64 = 4.5;

fn vertices(path: &[Vec2]) -> Vec<PolyVertex> {
    path.iter().map(|p| PolyVertex { p: *p, bulge: 0.0, start_width: WIRE_W, end_width: WIRE_W }).collect()
}

fn same(a: &[PolyVertex], b: &[PolyVertex]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.p.near(q.p, 1e-9) && p.start_width == q.start_width && p.end_width == q.end_width)
}

/// The path of every linked wire in `store` whose two devices exist. Each device edge's wires
/// are dealt slots in fan order, so a wire's route depends on its neighbours on both edges.
fn layout(store: &EntityStore) -> Vec<(Handle, Protocol, Vec<Vec2>)> {
    let device = |h| match store.get(h).map(|x| &x.kind) {
        Some(EntityKind::Insert(i)) if is_device_block(i) => Some(i),
        _ => None,
    };
    let wires: Vec<(Handle, WireLink, &Insert, &Insert)> = store
        .iter()
        .filter(|e| !e.common.xdata.is_empty() && matches!(e.kind, EntityKind::LwPolyline(_)))
        .filter_map(|e| {
            let link = WireLink::from_xdata(&e.common.xdata)?;
            let (a, b) = (device(link.from)?, device(link.to)?);
            Some((e.handle, link, a, b))
        })
        .collect();
    // Per device edge: each wire end with the far device's centre; `true` is the wire's `from` end.
    let mut edges: HashMap<(Handle, Side), Vec<((Handle, bool), Vec2)>> = HashMap::new();
    for (w, l, a, b) in &wires {
        edges.entry((l.from, l.from_side)).or_default().push(((*w, true), b.insert.xy()));
        edges.entry((l.to, l.to_side)).or_default().push(((*w, false), a.insert.xy()));
    }
    let mut slots: HashMap<(Handle, bool), Slot> = HashMap::new();
    for ((dev, side), ends) in &edges {
        let Some(i) = device(*dev) else { continue };
        let frame = edge_frame(i.insert.xy(), i.rotation, Vec2::new(i.scale.x, i.scale.y), *side);
        slots.extend(fan_order(&frame, ends));
    }
    let end = |i: &Insert, side, slot| {
        let scale = Vec2::new(i.scale.x, i.scale.y);
        (port_point(i.insert.xy(), i.rotation, scale, side, slot), world_side(side, i.rotation, scale))
    };
    wires
        .iter()
        .map(|(w, l, a, b)| {
            let (sa, sb) = (slots.get(&(*w, true)).copied().unwrap_or(Slot::SOLO), slots.get(&(*w, false)).copied().unwrap_or(Slot::SOLO));
            let ((p, side_p), (q, side_q)) = (end(a, l.from_side, sa), end(b, l.to_side, sb));
            (*w, l.protocol, route(p, side_p, sa, q, side_q, sb))
        })
        .collect()
}

/// A decoration the drawing should have: an insert of its block on its wire's layer.
struct Want {
    wire: Handle,
    deco: Deco,
    protocol: Protocol,
    layer: String,
    insert: Insert,
}

impl Want {
    fn matches(&self, e: &Entity) -> bool {
        let EntityKind::Insert(i) = &e.kind else { return false };
        i.block == self.insert.block
            && i.insert.xy().near(self.insert.insert.xy(), 1e-9)
            && i.scale == self.insert.scale
            && i.rotation == 0.0
            && i.attribs.is_empty()
            && e.common.layer == self.layer
            && e.common.color == Color::ByLayer
    }
}

#[derive(Default)]
struct Plan {
    routes: Vec<(Handle, Vec<PolyVertex>)>,
    erase: Vec<Handle>,
    set: Vec<(Handle, Want)>,
    add: Vec<Want>,
    lift: Vec<Handle>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.routes.is_empty() && self.erase.is_empty() && self.set.is_empty() && self.add.is_empty() && self.lift.is_empty()
    }
}

/// What differs between `store` and the drawing the linked wires call for: every wire on its
/// route with exactly its decorations, no decoration of a wire that is not there, and the
/// decorations on top of everything else. A wire with a missing device keeps its geometry and
/// its decorations, so undoing the erase finds it linked again.
fn plan(store: &EntityStore) -> Plan {
    let mut plan = Plan::default();
    let wire_set: HashSet<Handle> = wires(store).into_iter().map(|(h, _)| h).collect();
    // A decoration of a wire that is not one (a stale or hostile handle) or a second one for the
    // same role (a copy) is erased. Entity xdata that does not parse is not a decoration at all.
    let mut have: HashMap<(Handle, Deco), Handle> = HashMap::new();
    let (mut decos, mut last_other) = (Vec::new(), None);
    for (i, e) in store.iter().enumerate() {
        match deco_from_xdata(&e.common.xdata) {
            Some(key) if wire_set.contains(&key.0) && !have.contains_key(&key) => {
                have.insert(key, e.handle);
                decos.push((i, e.handle));
            }
            Some(_) => plan.erase.push(e.handle),
            None => last_other = Some(i),
        }
    }
    plan.lift = decos.iter().filter(|(i, _)| last_other.is_some_and(|l| *i < l)).map(|(_, h)| *h).collect();
    for (wire, protocol, path) in layout(store) {
        let vs = vertices(&path);
        if !matches!(store.get(wire).map(|e| &e.kind), Some(EntityKind::LwPolyline(pl)) if same(&pl.vertices, &vs)) {
            plan.routes.push((wire, vs));
        }
        let layer = store.get(wire).map(|e| e.common.layer.clone()).unwrap_or_default();
        for (deco, at) in deco_points(&path).into_iter().flatten() {
            let insert = Insert { block: deco_block_name(protocol, deco), insert: at.to3(0.0), ..Insert::default() };
            let want = Want { wire, deco, protocol, layer: layer.clone(), insert };
            match have.get(&(wire, deco)) {
                Some(h) if store.get(*h).is_some_and(|e| want.matches(e)) => {}
                Some(h) => plan.set.push((*h, want)),
                None => plan.add.push(want),
            }
        }
    }
    plan
}

fn apply(doc: &mut Arc<Drawing>, space: &Space, plan: Plan) {
    if plan.is_empty() {
        return;
    }
    let d = Arc::make_mut(doc);
    for h in plan.erase {
        d.remove_entity(h);
    }
    for (h, vs) in plan.routes {
        let _ = d.modify_entity(h, |e| {
            if let EntityKind::LwPolyline(pl) = &mut e.kind {
                pl.vertices = vs;
            }
        });
    }
    for (h, w) in plan.set {
        let _ = d.modify_entity(h, |e| {
            e.kind = EntityKind::Insert(w.insert);
            e.common.layer = w.layer;
            e.common.color = Color::ByLayer;
        });
    }
    for w in plan.add {
        if d.block(&w.insert.block).is_none() {
            let block = deco_block(w.protocol, w.deco, || d.new_handle());
            d.blocks.insert(w.insert.block.clone(), Arc::new(block));
        }
        let common = Common { layer: w.layer, xdata: deco_xdata(w.wire, w.deco), ..Common::default() };
        let _ = d.add(space, common, EntityKind::Insert(w.insert));
    }
    if let Some(store) = d.space_mut(space) {
        plan.lift.into_iter().for_each(|h| {
            store.bring_to_front(h);
        });
    }
}

/// Brings every linked wire and its decorations in line with the devices.
/// ponytail: every wire is re-routed after every command; index by changed handles (as assoc.rs
/// does) when drawings get large.
fn refresh(doc: &mut Arc<Drawing>) {
    let spaces: Vec<Space> = std::iter::once(Space::Model).chain(doc.layouts.iter().map(|l| Space::Paper(l.name.clone()))).collect();
    for space in spaces {
        let p = doc.space(&space).map(plan).unwrap_or_default();
        apply(doc, &space, p);
    }
}

fn run_place(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "product").ok_or_else(|| bad("csd.place", "`product` is required"))?;
    let at = point_req("csd.place", p, "at")?;
    let library = s.csd_library.clone();
    let product = library.iter().find(|x| x.id == id).ok_or_else(|| bad("csd.place", format!("unknown product \"{id}\"")))?;
    let name = block_name(&product.id);
    if s.doc()?.block(&name).is_none() {
        let d = s.doc_mut()?;
        let dash = if product.missing { dashed_scale(d, BORDER_DASH) } else { 1.0 };
        let block = device_block(product, || d.new_handle(), dash);
        d.blocks.insert(name.clone(), Arc::new(block));
    }
    // The strip shows the label, or the short code until there is one.
    let label = str_param(p, "label").filter(|l| !l.trim().is_empty()).unwrap_or(&product.code);
    let mut values = Map::new();
    values.insert("PRODUCT".into(), json!(product.id));
    values.insert("NAME".into(), json!(label));
    values.insert("LABEL".into(), json!(truncate_label(label)));
    let h = super::blocks::insert(s, &name, at, 1.0, 0.0, &values)?;
    Ok(json!({ "handle": h.hex() }))
}

fn side_param(p: &Value, key: &str) -> Result<Side> {
    str_param(p, key).and_then(Side::from_id).ok_or_else(|| bad("csd.connect", format!("`{key}` must be top, bot, lft or rgt")))
}

fn run_connect(s: &mut Session, p: &Value) -> Result<Value> {
    let handle = |key| handle_param(p, key).ok_or_else(|| bad("csd.connect", format!("`{key}` (object handle) is required")));
    let link = WireLink {
        from: handle("from")?,
        from_side: side_param(p, "fromSide")?,
        to: handle("to")?,
        to_side: side_param(p, "toSide")?,
        protocol: str_param(p, "protocol").and_then(Protocol::from_id).ok_or_else(|| bad("csd.connect", "`protocol` is not a known protocol id"))?,
    };
    let products = s.csd_library.clone();
    {
        let d = s.doc()?;
        let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
        let list: Vec<(Handle, &Product)> = devices(store, &products).iter().map(|x| (x.handle, x.product)).collect();
        let existing: Vec<WireLink> = wires(store).into_iter().map(|(_, l)| l).collect();
        can_connect(&list, &existing, &link).map_err(|r| EngineError::Other(r.message))?;
    }
    let (layer, ltscale) = wire_layer(s.doc_mut()?, link.protocol);
    // The route comes from `refresh`, which also deals the neighbours' slots on both edges. Wires
    // go under the cards, as on the web app's sheet.
    let h =
        s.add_entity(EntityKind::LwPolyline(LwPolyline { vertices: Vec::new(), closed: false, const_width: 0.0, elevation: 0.0, plinegen: false }))?;
    s.doc_mut()?.modify_entity(h, |e| {
        e.common.xdata = link.to_xdata();
        e.common.layer = layer;
        e.common.ltscale = ltscale;
        e.common.color = Color::ByLayer;
        e.common.linetype = "ByLayer".into();
    })?;
    let space = s.space();
    if let Some(store) = s.doc_mut()?.space_mut(&space) {
        store.send_to_back(h);
    }
    refresh(&mut s.state_mut()?.doc);
    Ok(json!({ "handle": h.hex() }))
}

/// The entity linetype scale that makes the first dash of `DASH` `len` long, whatever units the
/// library was loaded in; loads the linetype if the drawing lacks it.
fn dashed_scale(d: &mut Drawing, len: f64) -> f64 {
    if d.linetype(DASH).is_none() {
        let lt = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == DASH);
        d.linetypes.extend(lt);
    }
    d.linetype(DASH).and_then(|lt| lt.pattern.first()).map(|e| e.length.abs()).filter(|l| *l > 0.0).map_or(1.0, |l| len / l)
}

/// The layer for `protocol`'s wires, created on first use in the protocol's colour (true
/// colour) and, for the dashed protocols, with a dashed linetype; also the linetype scale for
/// its wires.
fn wire_layer(d: &mut Drawing, protocol: Protocol) -> (String, f64) {
    let name = format!("CSD_{}", protocol.id());
    let scale = if protocol.dashed() { dashed_scale(d, DASH_LEN) } else { 1.0 };
    if d.layer(&name).is_none() {
        let mut layer = Layer::new(&name);
        layer.color = Color::True(Rgb::from_u32(protocol.color()));
        if protocol.dashed() {
            layer.linetype = DASH.into();
        }
        d.layers.push(layer);
    }
    (name, scale)
}

fn run_netlist(s: &mut Session, _p: &Value) -> Result<Value> {
    let products = s.csd_library.clone();
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
    let devs: Vec<Value> = devices(store, &products)
        .iter()
        .map(|x| {
            let label = x.insert.attribs.iter().find(|a| a.tag == "NAME").map(|a| a.text.value.as_str()).unwrap_or_default();
            json!({ "handle": x.handle.hex(), "product": x.product.id, "label": label, "at": [x.insert.insert.x, x.insert.insert.y] })
        })
        .collect();
    let wires: Vec<Value> = wires(store)
        .iter()
        .map(|(h, l)| json!({ "handle": h.hex(), "from": l.from.hex(), "fromSide": l.from_side.id(), "to": l.to.hex(), "toSide": l.to_side.id(), "protocol": l.protocol.id() }))
        .collect();
    Ok(json!({ "devices": devs, "wires": wires }))
}

fn run_library_load(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("csd.library.load", "`path` is required"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = crate::capped_read::read_capped(std::path::Path::new(path), cadcraft_csd::MAX_LIBRARY_BYTES)
        .map_err(|e| bad("csd.library.load", format!("{path}: {e}")))?;
    #[cfg(target_arch = "wasm32")]
    let bytes: Vec<u8> = return Err(bad("csd.library.load", "reading files is not available on the web"));
    let text = String::from_utf8(bytes).map_err(|_| bad("csd.library.load", format!("{path}: not UTF-8 text")))?;
    let products = parse_library(&text).map_err(|e| bad("csd.library.load", format!("{path}: {e}")))?;
    s.csd_library = Arc::new(products);
    Ok(json!({ "count": s.csd_library.len() }))
}

fn run_library_list(s: &mut Session, p: &Value) -> Result<Value> {
    let query = str_param(p, "query").map(str::to_lowercase).filter(|q| !q.is_empty());
    let category = str_param(p, "category").filter(|c| !c.is_empty());
    let mut categories: Vec<&str> = Vec::new();
    for x in s.csd_library.iter() {
        if !categories.contains(&x.cat.as_str()) {
            categories.push(&x.cat);
        }
    }
    let hit = |x: &Product| {
        category.is_none_or(|c| x.cat == c)
            && query.as_ref().is_none_or(|q| [&x.id, &x.name, &x.code].iter().any(|f| f.to_lowercase().contains(q.as_str())))
    };
    // Pictures and other long strings stay out so a list of hundreds stays small.
    let light = |mut r: Map<String, Value>| {
        r.retain(|k, v| k != "img" && !matches!(v, Value::String(t) if t.len() > 1000));
        Value::Object(r)
    };
    let products: Vec<Value> = s.csd_library.iter().filter(|x| hit(x)).map(|x| light(product_record(x))).collect();
    Ok(json!({ "products": products, "categories": categories }))
}

fn run_library_set(s: &mut Session, p: &Value) -> Result<Value> {
    let records = p.get("products").and_then(Value::as_array).ok_or_else(|| bad("csd.library.set", "`products` (array of records) is required"))?;
    let mut products = parse_products(records).map_err(|e| bad("csd.library.set", e))?;
    for x in &mut products {
        if let Some(old) = s.csd_library.iter().find(|o| o.id == x.id) {
            let mut extra = old.extra.clone();
            extra.append(&mut x.extra);
            x.extra = extra;
        }
    }
    s.csd_library = Arc::new(products);
    Ok(json!({ "count": s.csd_library.len() }))
}

fn run_library_save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("csd.library.save", "`path` is required"))?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::atomic_write::write_atomic_making_folders(std::path::Path::new(path), library_json(&s.csd_library).as_bytes())
        .map_err(|e| bad("csd.library.save", format!("{path}: {e}")))?;
    #[cfg(target_arch = "wasm32")]
    return Err(bad("csd.library.save", "writing files is not available on the web"));
    Ok(json!({ "path": path, "count": s.csd_library.len() }))
}

pub(crate) fn after_command(s: &mut Session, before: Option<&Arc<Drawing>>) {
    if let Ok(st) = s.state_mut()
        && before.is_some_and(|b| !Arc::ptr_eq(b, &st.doc))
    {
        refresh(&mut st.doc);
    }
}

#[cfg(test)]
mod tests;
