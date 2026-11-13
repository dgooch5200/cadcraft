//! Lighting-control system designer: devices are block references of `CSD_` blocks, wires are
//! polylines whose xdata ([`WireLink`]) names the two devices they join. The post-command pass
//! keeps every wire on its devices' ports.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cadcraft_csd::{
    BOLD_STYLE, CARD_H, CARD_W, Deco, OFF_PAGE_ID, Product, Protocol, Side, Slot, WIRE_W, WireLink, accessory_products, bold_style, can_connect,
    canvas_code, card_block_name, card_size, deco_block, deco_block_name, deco_from_xdata, deco_points, deco_xdata, device_block, edge_frame,
    fan_order, is_builtin, is_two_line_block, library_json, lookup, max_supply_watts, parse_library, parse_products, poe_verdict, port_point,
    product_record, route_via, truncate_label, used, world_side,
};
use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{Common, Drawing, Entity, EntityKind, EntityStore, Handle, Insert, Layer, LwPolyline, Space};
use cadcraft_geom::{Mat3, PolyVertex, Vec2};
use serde_json::{Map, Value, json};

use super::curves::handle_param;

mod arrange;
mod connect;
mod cpsd;
mod cpsd_out;
mod group;
mod html;
mod library_csv;
mod lpro;
mod mount;
mod new;
mod patch;
mod place;
mod print;
mod schedule;
mod shapes;
mod sheet;
mod twoline;
mod units;
mod wire;
use super::{CommandSpec, always, bad, point_req, str_param, targets};
use crate::{EngineError, Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    let mut all = own_specs();
    all.extend(wire::specs());
    all.extend(library_csv::specs());
    all
}

fn own_specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("csd.place", "Place Device", run_place)
            .params("{product: id of a library product, at: [x,y], label? (default the product code)} → {handle}; interactive: places the armed product at each point")
            .interactive(place::PlaceM::start),
        CommandSpec::new("csd.place.grid", "Place Device Grid", place::run_grid)
            .params("{product: id of a library product, at: [x,y] (first card's centre), rows, cols: >= 1 (rows * cols <= 400), dx?, dy?: spacing (default card plus 24)} → {handles}; rows run downwards"),
        CommandSpec::new("csd.new", "New Lighting Drawing", new::run_new)
            .params("{template?: a .cpsd file to start from, paper?: tabloid|letter (default tabloid)} → {index, title, paper, template}; a new drawing with the title block and line key, plus the template's first page when given (an unreadable template is an error and leaves no drawing)")
            .menu(&["File", "New Lighting Drawing..."])
            .enabled(always)
            .noundo(),
        CommandSpec::new("csd.import.cpsd", "Import ChamSys Designer Project", cpsd::run_import)
            .params("{path: a .cpsd file, page?: 1-based number or id (default the first page)} → {devices, wires, unplaced, shapes, skipped: [reasons]}; builds the page's devices, wires (waypoints, label position, name), shapes and text (lines, rectangles, ellipses and text on layer CSD_SHAPES), off-page targets (as the page's name), the ortho setting, the unplaced list and the title block into the current drawing"),
        CommandSpec::new("csd.import.lpro", "Import Luminary Project", lpro::run_import)
            .params("{path: a Luminary Pro .lpro file, choices?: {typeKey: product id, \"\" skips}, preview?: true} → {types: [{key, name, manufacturer, luminaire, catalog, quantity, product, matched, suggestions}], fixtures, skipped}; adds each fixture type's count to the unplaced list as the library product it names (types the file does not name specifically are skipped unless `choices` picks one); `preview` only reports"),
        CommandSpec::new("csd.export.cpsd", "Export ChamSys Designer Project", cpsd_out::run_export)
            .params("{path} → {path, devices, wires, unplaced, shapes, skipped: [reasons]}; writes the drawing as a one-page .cpsd (devices, mounts, wires with waypoints and names, unplaced items, title block, the products they use, the ortho setting, off-page targets that name the exported page, and lines, rectangles, ellipses and text on layer CSD_SHAPES as shapes; what cannot be written is counted in skipped)")
            .noundo(),
        CommandSpec::new("csd.clear", "Clear Drawing", run_clear)
            .params("{} → {devices, wires}; removes every device, wire and wire decoration; the title block, line key and unplaced list stay"),
        CommandSpec::new("csd.display", "Card Display", run_display)
            .params("{codesOnly?: bool} → {codesOnly}; codes only shows the code instead of the product picture, saved with the drawing (no pictures yet, so no visible change)"),
        CommandSpec::new("csd.size", "Size Devices", run_size)
            .params("{handles? (default the selection), w?, h?: card size in drawing units, 10..1000, null resets that axis} → {sized}"),
        CommandSpec::new("csd.replace", "Replace Devices", run_replace)
            .params("{product: id of a library product, handles? (default the selection), allOfProduct?: bool (default false: also every other device on the drawing with the same product as a target)} → {replaced}; devices already that product are skipped"),
        CommandSpec::new("csd.device.info", "Device Info", run_device_info)
            .params("{handle? (default the single selected device)} → {handle, product: {id, name, code, brand, cat, color: 0xRRGGBB number}, known, label, notes, linkPage, mount: null | {handle, label} (the device this one is mounted in; layout only), accessories: [{product, name, code, category, qty}], supplyByOthers: bool, ports: [{protocol, label, used, capacity, pooled}], connections: [{wire, protocol, dir: out|in, other: {handle, label}}], sameProduct, at: [x,y], size: [w,h], libSize: [w,h]}")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.device.set", "Set Device Properties", run_device_set)
            .params("{handle? (default the single selected device), label?: text (empty = the product code), notes?: text, mount?: handle of the device to mount in (\"\" = unmount; not itself or anything mounted in it), accessories?: [{product: id, qty}] (replaces the list; qty 0 drops a line), supplyByOthers?: bool (left off the bill of materials, accessories included), twoLine?: bool (a second label strip: the product code above, the label below; the card grows by one strip and the bottom ports move down; not for built-ins), linkPage?: text, the page an off-page connector continues on (shown as \"See <text>\"; off-page connectors only)} → {handle}"),
        CommandSpec::new("csd.devices.info", "Devices Info", group::run_info)
            .params("{handles? (default the selection)} → {count, products: [{id, name, code, count}], size: {w, h}: card size in drawing units, null where the devices differ, twoLine: {devices: how many are not built-ins, on: how many of those have two strips}}")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.devices.accessory", "Add Accessory to Devices", group::run_accessory)
            .params("{product: id of a library product, handles? (default the selection), qty?: whole number (default 1)} → {changed}; sets that accessory line to `qty` on every device (the same as csd.device.set on each)"),
        CommandSpec::new("csd.devices.set", "Set Devices Properties", twoline::run_set)
            .params("{handles? (default the selection), twoLine: bool, as `csd.device.set`; built-ins are skipped} → {changed}"),
        CommandSpec::new("csd.devices.label", "Label Devices", group::run_label)
            .params("{handles? (default the selection), base: text (blank = each product code), start?: integer, pad?: digits to zero-pad the number to (default 0)} → {labels: [{handle, label}]}; without `start` every device gets `base`; with it, `base` plus a running number from `start` in reading order (top to bottom, left to right)"),
        CommandSpec::new("csd.device.delete", "Delete Devices", run_device_delete)
            .params("{handles? (default the selection)} → {deleted, wires}; also removes every wire linked to them and the wires' decorations"),
        CommandSpec::new("csd.select.product", "Select All of This Product", arrange::run_select_product)
            .params("{device: handle} → {count}; selects every device of that device's product")
            .noundo(),
        CommandSpec::new("csd.device.at", "Device at Point", arrange::run_device_at)
            .params("{at: [x,y]} → {handle: hex | null}; the topmost device card covering the point")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.align", "Align Devices", arrange::run_align)
            .params("{mode: left|top|right|bottom, handles? (default the selection)} → {moved}; aligns card edges"),
        CommandSpec::new("csd.distribute", "Distribute Devices", arrange::run_distribute)
            .params("{axis: x|y, handles? (default the selection)} → {moved}; equal gaps between cards, the outer two stay put (needs 3+)"),
        CommandSpec::new("csd.schedule", "Placed Devices", schedule::run_schedule)
            .params("{} → {devices: [{handle, product, name, category, code, label, notes, mountedIn: label of the device it is mounted in, supplyByOthers, accessories: [{product, code, name, qty}], portsInUse: text like \"PoE 1/3, DMX 2/2\", connections: wire count, poeLoad: watts the device's PoE outlets feed, poeWarning: text when a PoE port carries more than its supply's per-port max (else null), controlGear}]}; control gear first")
            .noundo(),
        CommandSpec::new("csd.unplaced.list", "Unplaced Items", schedule::run_unplaced_list)
            .params("{} → {items: [{index, product, qty, code, category, name}]}; saved with the drawing")
            .noundo(),
        CommandSpec::new("csd.unplaced.add", "Add Unplaced Item", schedule::run_unplaced_add)
            .params("{product: id of a library product, qty? (default 1; adds to the product's row)} → {index}"),
        CommandSpec::new("csd.unplaced.set", "Set Unplaced Quantity", schedule::run_unplaced_set).params("{index, qty: >= 1} → {index}"),
        CommandSpec::new("csd.unplaced.remove", "Remove Unplaced Item", schedule::run_unplaced_remove).params("{index} → {removed}"),
        CommandSpec::new("csd.unplaced.place", "Place Unplaced Item", schedule::run_unplaced_place)
            .params("{index} → {product}; arms the row's product like csd.arm. Placing a product (csd.place) takes one off its row")
            .noundo(),
        CommandSpec::new("csd.bom", "Bill of Materials", schedule::run_bom)
            .params("{} → {rows: [{qty, product, code, category, name, labels, notes, brand}]}: placed and unplaced plus device accessories, by category (control gear first) then model id; devices supplied by others and their accessories are left out")
            .noundo(),
        CommandSpec::new("csd.bom.export", "Export Bill of Materials", schedule::run_bom_export)
            .params("{path, format: csv|pdf|html, footer?: [lines printed under the PDF table] (default the title block's project, venue, drawn by, date and rev)} → {path, rows}")
            .noundo(),
        CommandSpec::new("csd.device.mode", "Set Device DMX Mode", patch::run_device_mode)
            .params("{handles? (default the selection), mode: a mode name the product offers (empty or null = the product's largest)} → {changed}"),
        CommandSpec::new("csd.patch", "DMX Patch", patch::run_patch)
            .params("{order?: drawing|name|product|position (default: the drawing's saved order)} → {order: the saved order, addressing: bool, rows: [{handle: null for an unplaced product, carriedBy: the device an accessory rides on or null, label, product, name, mode: chosen mode or null, assumedMode, modes: [names], channels, universe, address}], universes: {used, budget, over}}; devices that take channels (each with its accessories after it, one row per unit, and the unplaced products last) packed into 512-channel universes in order, none straddling a universe; budget is the universes the placed controllers originate")
            .noundo(),
        CommandSpec::new("csd.patch.setup", "DMX Patch Setup", patch::run_patch_setup)
            .params("{order?: drawing|name|product|position, addressing?: bool} → {order, addressing}; saves the patch order and the addressing switch with the drawing (csd.patch and its export use them; addressing off patches nothing)"),
        CommandSpec::new("csd.patch.export", "Export DMX Patch", patch::run_patch_export)
            .params("{path, format: csv|magicq|pdf|html, order?} → {path, rows}; magicq is MagicQ's Fixture Patch CSV (Head No, DMX, Position, Hang, Manufacturer, Model, Mode)")
            .noundo(),
        CommandSpec::new("csd.arm", "Arm Product", place::run_arm).params("{product: id of a library product} → {product}").noundo(),
        CommandSpec::new("csd.protocol", "Wire Protocol", run_protocol)
            .params("{protocol: network|netdata|10scene|dmx|0-10v|contact|power|midi|epix3p|epix4p} → {protocol}; the wire type csd.connect draws next")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.connect", "Connect Devices", run_connect)
            .params("{from: handle, fromSide: top|bot|lft|rgt, to: handle, toSide, protocol: network|netdata|10scene|dmx|0-10v|contact|power|midi|epix3p|epix4p} → {handle, warning?: PoE over the supply's port budget}; interactive: click a port on each of two devices")
            .interactive(connect::ConnectM::start),
        CommandSpec::new("csd.netlist", "Device Netlist", run_netlist)
            .params("{} → {devices: [{handle, product, label, at}], wires: [{handle, from, fromSide, to, toSide, protocol}]}")
            .noundo(),
        CommandSpec::new("csd.titleblock", "Title Block", sheet::run_titleblock)
            .params("{at?: [x,y] the page's lower-left corner (default 0,0), paper?: tabloid|letter (default tabloid; an existing title block is resized when given)} → {handle, paper}; inserts the CSD_TITLEBLOCK sheet frame and title block if the drawing has none, else returns the existing one"),
        CommandSpec::new("csd.titleblock.fit", "Fit Sheet to Drawing", sheet::run_titleblock_fit)
            .params("{} → {handle, scale, changed}; scales the title block and line keys up about their corner, centred, until the rest of the drawing fits inside the frame; does nothing when it already fits"),
        CommandSpec::new("csd.print", "Print Sheet", print::run_print)
            .params("{path, format?: pdf|svg|png (default the path's extension)} -> the plot result; plots the title block's page (else the drawing's extents) fit to 11x17in or 8.5x11in landscape on white; from the UI with no path opens a save dialog")
            .noundo(),
        CommandSpec::new("csd.paper", "Paper Size", sheet::run_paper)
            .params("{paper: tabloid|letter} → {handle, paper}; redraws the title block and line key for 11x17 or 8.5x11 landscape, keeping the field values; needs a title block"),
        CommandSpec::new("csd.project", "Project Info", sheet::run_project)
            .params("{} → {fields: {brand, project, venue, location, drawnBy, date, sheet, rev, title, meta}, exists, paper, handle, devices}; the title block's attributes, empty text when there is no title block")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.project.set", "Set Project Info", sheet::run_project_set)
            .params("{fields: {brand?, project?, venue?, location?, drawnBy?, date?, sheet?, rev?, title?, meta?: text}} → {handle}; inserts the title block first when there is none"),
        CommandSpec::new("csd.legend", "Line Key", sheet::run_legend)
            .params("{at?: [x,y] (default above the title block)} → {handle}; inserts the LINE KEY legend of the protocols in use if there is none, else refreshes and returns it; it then follows the wires"),
        CommandSpec::new("csd.library.load", "Load Product Library", run_library_load)
            .params("{path: JSON library file} → {count}")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.library.list", "List Products", run_library_list)
            .params("{query? (id, name or code contains), category?, accessoriesFor? (product ids: only accessories that are general or of those products' series)} → {products: [records without img], categories: [cat, first-seen order]}")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.library.set", "Set Product Library", run_library_set)
            .params("{products: [records]} → {count}; a record without `img` (or other extra keys) keeps the old product's")
            .noundo()
            .enabled(always),
        CommandSpec::new("csd.library.save", "Save Product Library", run_library_save)
            .params("{path} → {path, count}")
            .noundo()
            .enabled(always),
    ]
}

fn run_protocol(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "protocol").ok_or_else(|| bad("csd.protocol", "`protocol` is required"))?;
    s.csd_protocol = Protocol::from_id(id).ok_or_else(|| bad("csd.protocol", format!("unknown protocol \"{id}\"")))?;
    Ok(json!({ "protocol": id }))
}

/// A device on the drawing: a `CSD_` block reference with a `PRODUCT` attribute. A product the
/// library lacks is a [`stand_in`].
struct Device<'a> {
    handle: Handle,
    product: Cow<'a, Product>,
    insert: &'a Insert,
}

impl Device<'_> {
    /// The card's size before the insert's scale.
    fn size(&self) -> Vec2 {
        card_size(&self.product, two_line(self.insert))
    }
}

/// Whether the card shows a second strip: the `TWOLINE` attribute.
fn two_line(i: &Insert) -> bool {
    attr(i, "TWOLINE") == "1"
}

/// The size, before scale, of the card `i` is an insert of.
fn card_of(i: &Insert, products: &[Product]) -> Vec2 {
    card_size(&resolve(products, product_attr(i).unwrap_or_default()), two_line(i))
}

fn is_device_block(i: &Insert) -> bool {
    i.block.starts_with("CSD_")
}

/// The product named `id`: exact, else ignoring case and punctuation, so a file saved with
/// `logic-poex` finds `LOGICPOEX`; else the one product whose code matches the same way.
fn find_product<'a>(products: &'a [Product], id: &str) -> Option<&'a Product> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let k = key(id);
    lookup(products, id).or_else(|| products.iter().find(|p| key(&p.id) == k)).or_else(|| {
        let mut by_code = products.iter().filter(|p| key(&p.code) == k);
        by_code.next().filter(|_| by_code.next().is_none())
    })
}

/// What a card whose product the library lacks is backed by: no ports, like a library product
/// flagged `missing`.
fn stand_in(id: &str) -> Product {
    Product { id: id.into(), name: format!("{id} (not in library)"), code: id.into(), color: 0x888888, missing: true, ..Product::default() }
}

fn resolve<'a>(products: &'a [Product], id: &str) -> Cow<'a, Product> {
    find_product(products, id).map_or_else(|| Cow::Owned(stand_in(id)), Cow::Borrowed)
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
            Some(Device { handle: e.handle, product: resolve(products, id), insert })
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

const CORNER_R: f64 = 6.0;

/// The wire's polyline: its route, with each interior bend of an `ortho` route rounded by an arc
/// (a bulge of tan(turn / 4)), kept short enough that neighbouring bends never overlap.
fn vertices(path: &[Vec2], ortho: bool) -> Vec<PolyVertex> {
    let v = |p: Vec2, bulge| PolyVertex { p, bulge, start_width: WIRE_W, end_width: WIRE_W };
    if !ortho {
        return path.iter().map(|&p| v(p, 0.0)).collect();
    }
    let mut out = Vec::with_capacity(path.len());
    for (i, &p) in path.iter().enumerate() {
        let (Some(&prev), Some(&next)) = (i.checked_sub(1).and_then(|j| path.get(j)), path.get(i + 1)) else {
            out.push(v(p, 0.0));
            continue;
        };
        let (a, b) = (p - prev, next - p);
        let (la, lb) = (a.dist(Vec2::ZERO), b.dist(Vec2::ZERO));
        let turn = (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y);
        if la < 1e-9 || lb < 1e-9 || turn.abs() < 1e-6 || turn.abs() > std::f64::consts::PI - 0.01 {
            out.push(v(p, 0.0));
            continue;
        }
        let t = (CORNER_R * (turn.abs() / 2.0).tan()).min(la / 2.0).min(lb / 2.0);
        out.push(v(p - a * (t / la), (turn / 4.0).tan()));
        out.push(v(p + b * (t / lb), 0.0));
    }
    out
}

fn same(a: &[PolyVertex], b: &[PolyVertex]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| p.p.near(q.p, 1e-9) && p.bulge == q.bulge && p.start_width == q.start_width && p.end_width == q.end_width)
}

/// A wire's computed route and what the pass should write back to its link.
struct Laid {
    wire: Handle,
    link: WireLink,
    path: Vec<Vec2>,
    /// The link's waypoints after moving with both ends, if they did.
    moved: Option<Vec<Vec2>>,
}

/// The path of every linked wire in `store` whose two devices exist. Each device edge's wires
/// are dealt slots in fan order, so a wire's route depends on its neighbours on both edges.
fn layout(store: &EntityStore, products: &[Product], ortho: bool) -> Vec<Laid> {
    layout_with(store, products, ortho, None)
}

/// [`layout`] with one wire's waypoints replaced, to preview an edit.
fn layout_with(store: &EntityStore, products: &[Product], ortho: bool, over: Option<(Handle, &[Vec2])>) -> Vec<Laid> {
    let device = |h| match store.get(h).map(|x| &x.kind) {
        Some(EntityKind::Insert(i)) if is_device_block(i) => Some(i),
        _ => None,
    };
    let wires: Vec<(Handle, WireLink, &Insert, &Insert, Option<(Vec2, Vec2)>)> = store
        .iter()
        .filter(|e| !e.common.xdata.is_empty() && matches!(e.kind, EntityKind::LwPolyline(_)))
        .filter_map(|e| {
            let mut link = WireLink::from_xdata(&e.common.xdata)?;
            if let Some((_, points)) = over.filter(|(h, _)| *h == e.handle) {
                link.points = points.to_vec();
            }
            let (a, b) = (device(link.from)?, device(link.to)?);
            let EntityKind::LwPolyline(pl) = &e.kind else { return None };
            let was = pl.vertices.first().zip(pl.vertices.last()).map(|(f, l)| (f.p, l.p));
            Some((e.handle, link, a, b, was))
        })
        .collect();
    let mut sizes: HashMap<Handle, Vec2> = HashMap::new();
    for (_, l, a, b, _) in &wires {
        sizes.entry(l.from).or_insert_with(|| card_of(a, products));
        sizes.entry(l.to).or_insert_with(|| card_of(b, products));
    }
    let size = |h: Handle| sizes.get(&h).copied().unwrap_or(Vec2::new(CARD_W, CARD_H));
    // Per device edge: each wire end with the far device's centre; `true` is the wire's `from` end.
    let mut edges: HashMap<(Handle, Side), Vec<((Handle, bool), Vec2)>> = HashMap::new();
    for (w, l, a, b, _) in &wires {
        edges.entry((l.from, l.from_side)).or_default().push(((*w, true), b.insert.xy()));
        edges.entry((l.to, l.to_side)).or_default().push(((*w, false), a.insert.xy()));
    }
    let mut slots: HashMap<(Handle, bool), Slot> = HashMap::new();
    for ((dev, side), ends) in &edges {
        let Some(i) = device(*dev) else { continue };
        let frame = edge_frame(i.insert.xy(), i.rotation, Vec2::new(i.scale.x, i.scale.y), size(*dev), *side);
        slots.extend(fan_order(&frame, ends));
    }
    let end = |i: &Insert, h: Handle, side, slot| {
        let scale = Vec2::new(i.scale.x, i.scale.y);
        (port_point(i.insert.xy(), i.rotation, scale, size(h), side, slot), world_side(side, i.rotation, scale))
    };
    wires
        .into_iter()
        .map(|(w, l, a, b, was)| {
            let (sa, sb) = (slots.get(&(w, true)).copied().unwrap_or(Slot::SOLO), slots.get(&(w, false)).copied().unwrap_or(Slot::SOLO));
            let ((p, side_p), (q, side_q)) = (end(a, l.from, l.from_side, sa), end(b, l.to, l.to_side, sb));
            // Both ends shifted by one vector (the devices were moved together): the waypoints go along.
            let shift = was.map(|(f, t)| (p - f, q - t)).filter(|(d, e)| d.len() > 1e-9 && d.near(*e, 1e-6)).map(|(d, _)| d);
            let moved = shift.filter(|_| !l.points.is_empty()).map(|d| l.points.iter().map(|&pt| pt + d).collect::<Vec<_>>());
            let points = moved.as_ref().unwrap_or(&l.points);
            let path = route_via(p, side_p, sa, q, side_q, sb, points, ortho);
            Laid { wire: w, link: l, path, moved }
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
    links: Vec<(Handle, WireLink)>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.routes.is_empty() && self.links.is_empty() && self.erase.is_empty() && self.set.is_empty() && self.add.is_empty() && self.lift.is_empty()
    }
}

/// What differs between `store` and the drawing the linked wires call for: every wire on its
/// route with exactly its decorations, no decoration of a wire that is not there, and the
/// decorations on top of everything else. A wire with a missing device keeps its geometry and
/// its decorations, so undoing the erase finds it linked again.
fn plan(store: &EntityStore, products: &[Product], ortho: bool) -> Plan {
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
    for Laid { wire, link, path, moved } in layout(store, products, ortho) {
        let protocol = link.protocol;
        if let Some(points) = moved {
            plan.links.push((wire, WireLink { points, ..link.clone() }));
        }
        let vs = vertices(&path, ortho);
        if !matches!(store.get(wire).map(|e| &e.kind), Some(EntityKind::LwPolyline(pl)) if same(&pl.vertices, &vs)) {
            plan.routes.push((wire, vs));
        }
        let layer = store.get(wire).map(|e| e.common.layer.clone()).unwrap_or_default();
        for (deco, at) in deco_points(&path, ortho, link.label_t).into_iter().flatten() {
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
    for (h, link) in plan.links {
        let _ = d.modify_entity(h, |e| e.common.xdata = link.to_xdata());
    }
    for (h, w) in plan.set {
        let _ = d.modify_entity(h, |e| {
            e.kind = EntityKind::Insert(w.insert);
            e.common.layer = w.layer;
            e.common.color = Color::ByLayer;
        });
    }
    for w in plan.add {
        ensure_bold(d);
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

fn spaces(doc: &Drawing) -> Vec<Space> {
    std::iter::once(Space::Model).chain(doc.layouts.iter().map(|l| Space::Paper(l.name.clone()))).collect()
}

/// Brings every linked wire and its decorations in line with the devices.
/// ponytail: every wire is re-routed after every command; index by changed handles (as assoc.rs
/// does) when drawings get large.
pub(crate) fn refresh(doc: &mut Arc<Drawing>, products: &[Product], ortho: bool) {
    sheet::refresh_legend(doc);
    show_labels(doc);
    for space in spaces(doc) {
        let p = doc.space(&space).map(|st| plan(st, products, ortho)).unwrap_or_default();
        apply(doc, &space, p);
    }
}

/// The most `CSD_` blocks and attribute definitions [`refresh_blocks`] looks at, so a hostile drawing can't stall it.
const MAX_REFRESH: usize = 10_000;

/// Replaces the drawing's generated card and decoration blocks with the current design, so drawings
/// saved earlier and library edits show. A device block whose product is not in `library` is left
/// alone. Entity handles are ignored when comparing, so an up-to-date drawing is not touched.
/// Returns whether the drawing changed.
pub(crate) fn refresh_blocks(doc: &mut Arc<Drawing>, library: &[Product]) -> bool {
    let product_of = |b: &cadcraft_doc::Block| {
        b.entities.iter().take(MAX_REFRESH).find_map(|e| match &e.kind {
            EntityKind::AttDef(a) if a.tag == "PRODUCT" => lookup(library, &a.text.value),
            _ => None,
        })
    };
    let devices: Vec<(&Product, bool)> = doc
        .blocks
        .iter()
        .filter(|(n, _)| n.starts_with("CSD_"))
        .take(MAX_REFRESH)
        .filter_map(|(n, b)| Some((product_of(b)?, is_two_line_block(n))))
        .collect();
    let decos: Vec<(Protocol, Deco)> = Protocol::ALL
        .iter()
        .flat_map(|&p| [Deco::Start, Deco::Pill].map(|d| (p, d)))
        .filter(|&(p, d)| doc.block(&deco_block_name(p, d)).is_some())
        .collect();
    if devices.is_empty() && decos.is_empty() {
        return false;
    }
    let mut d = (**doc).clone();
    let mut changed = HashSet::new();
    let mut put = |d: &mut Drawing, block: cadcraft_doc::Block| {
        let same = d.block(&block.name).is_some_and(|o| {
            o.base == block.base
                && o.entities.len() == block.entities.len()
                && o.entities.iter().zip(block.entities.iter()).all(|(a, b)| a.common == b.common && same_kind(&a.kind, &b.kind))
        });
        if !same {
            changed.insert(block.name.clone());
            d.blocks.insert(block.name.clone(), Arc::new(block));
        }
    };
    for (product, two_line) in devices {
        ensure_bold(&mut d);
        let dash = if product.missing || is_builtin(&product.id) { dashed_scale(&mut d, BORDER_DASH) } else { 1.0 };
        let block = device_block(product, two_line, || d.new_handle(), dash);
        put(&mut d, block);
    }
    for (p, k) in decos {
        let block = deco_block(p, k, || d.new_handle());
        put(&mut d, block);
    }
    // A redrawn card may have moved its attributes (a built-in drawn at its own size, say).
    let refit: Vec<Handle> = spaces(&d)
        .iter()
        .filter_map(|sp| d.space(sp))
        .flat_map(|st| st.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(i) if changed.contains(&i.block))).map(|e| e.handle))
        .collect();
    for h in refit {
        let blk = d.entity(h).and_then(|e| match &e.kind {
            EntityKind::Insert(i) => d.block(&i.block).cloned(),
            _ => None,
        });
        let _ = d.modify_entity(h, |e| {
            if let (EntityKind::Insert(i), Some(blk)) = (&mut e.kind, &blk) {
                fit_attribs(blk, i);
            }
        });
    }
    let changed = !changed.is_empty();
    if changed {
        *doc = Arc::new(d);
    }
    changed
}

/// Kinds equal as a saved drawing reads them back: a wipeout's boundary gains its closing point.
fn same_kind(a: &EntityKind, b: &EntityKind) -> bool {
    match (a, b) {
        (EntityKind::Wipeout(a), EntityKind::Wipeout(b)) => {
            let (a, b) = (a.closed_boundary(), b.closed_boundary());
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| p.near(*q, 1e-9))
        }
        // An aligned text's insertion point is derived from its font on reading.
        (EntityKind::Text(a), EntityKind::Text(b)) if a.align_pt.is_some() => cadcraft_doc::Text { insert: b.insert, ..a.clone() } == *b,
        (EntityKind::AttDef(a), EntityKind::AttDef(b)) if a.text.align_pt.is_some() => {
            let text = cadcraft_doc::Text { insert: b.text.insert, ..a.text.clone() };
            cadcraft_doc::Attrib { text, ..a.clone() } == *b
        }
        _ => a == b,
    }
}

/// Refreshes every open drawing after the library changed.
fn refresh_open_drawings(s: &mut Session) {
    let library = s.csd_library.clone();
    for st in &mut s.docs {
        refresh_blocks(&mut st.doc, &library);
    }
}

fn run_place(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "product").ok_or_else(|| bad("csd.place", "`product` is required"))?;
    let at = point_req("csd.place", p, "at")?;
    let library = s.csd_library.clone();
    let product = lookup(&library, id).ok_or_else(|| bad("csd.place", format!("unknown product \"{id}\"")))?;
    let h = place_card(s, product, str_param(p, "label").unwrap_or_default(), at)?;
    schedule::consume(s.doc_mut()?, &product.id);
    Ok(json!({ "handle": h.hex() }))
}

/// Inserts `product`'s card at `at`; the strip shows `label`, or the short code until there is one
/// (a built-in has no code on its card, so its name).
fn place_card(s: &mut Session, product: &Product, label: &str, at: Vec2) -> Result<Handle> {
    let name = ensure_device_block(s, product, false)?;
    let default = if is_builtin(&product.id) { product.name.clone() } else { canvas_code(product) };
    let label = if label.trim().is_empty() { &default } else { label };
    let mut values = Map::new();
    values.insert("PRODUCT".into(), json!(product.id));
    values.insert("NAME".into(), json!(label));
    values.insert("LABEL".into(), json!(strip_text(product, false, label)));
    super::blocks::insert(s, &name, at, 1.0, 0.0, &values)
}

/// `label` as the card's label shows it: cut to the strip, or whole where a built-in wraps it.
fn strip_text(product: &Product, two_line: bool, label: &str) -> String {
    if is_builtin(&product.id) { label.into() } else { truncate_label(label, card_size(product, two_line).x) }
}

/// The name of `product`'s card block (the two-strip variant for `two_line`), defined in the drawing if it is not yet.
fn ensure_device_block(s: &mut Session, product: &Product, two_line: bool) -> Result<String> {
    let name = card_block_name(&product.id, two_line && !is_builtin(&product.id));
    if s.doc()?.block(&name).is_none() {
        let d = s.doc_mut()?;
        ensure_bold(d);
        let dash = if product.missing || is_builtin(&product.id) { dashed_scale(d, BORDER_DASH) } else { 1.0 };
        let block = device_block(product, two_line, || d.new_handle(), dash);
        d.blocks.insert(name.clone(), Arc::new(block));
    }
    Ok(name)
}

const MIN_SIZE: f64 = 10.0;
const MAX_SIZE: f64 = 1000.0;

/// One axis of `csd.size`: absent keeps `was`, null resets to the card's own size, a number sets it.
fn axis_scale(p: &Value, key: &str, card: f64, was: f64) -> Result<f64> {
    match p.get(key) {
        None => Ok(was),
        Some(Value::Null) => Ok(was.signum()),
        Some(v) => {
            let n = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad("csd.size", format!("`{key}` must be a number or null")))?;
            Ok(was.signum() * n.clamp(MIN_SIZE, MAX_SIZE) / card)
        }
    }
}

/// The device inserts among `hs`, each with its handle.
fn device_handles(s: &Session, hs: &[Handle]) -> Result<Vec<Handle>> {
    let d = s.doc()?;
    Ok(hs.iter().copied().filter(|h| matches!(d.entity(*h).map(|e| &e.kind), Some(EntityKind::Insert(i)) if is_device_block(i))).collect())
}

fn run_size(s: &mut Session, p: &Value) -> Result<Value> {
    let hs = device_handles(s, &targets(s, p)?)?;
    let library = s.csd_library.clone();
    // Validate against every device first so a bad value changes nothing.
    let mut scales = Vec::new();
    for h in &hs {
        if let Some(EntityKind::Insert(i)) = s.doc()?.entity(*h).map(|e| &e.kind) {
            let sign = |v: f64| if v == 0.0 { 1.0 } else { v };
            let card = card_of(i, &library);
            scales.push((*h, axis_scale(p, "w", card.x, sign(i.scale.x))?, axis_scale(p, "h", card.y, sign(i.scale.y))?));
        }
    }
    let d = s.doc_mut()?;
    for (h, x, y) in &scales {
        let blk = d.entity(*h).and_then(|e| match &e.kind {
            EntityKind::Insert(i) => d.block(&i.block).cloned(),
            _ => None,
        });
        d.modify_entity(*h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.scale.x = *x;
                i.scale.y = *y;
                if let Some(blk) = &blk {
                    fit_attribs(blk, i);
                }
            }
        })?;
    }
    Ok(json!({ "sized": scales.len() }))
}

fn run_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "product").ok_or_else(|| bad("csd.replace", "`product` is required"))?;
    let library = s.csd_library.clone();
    let product = lookup(&library, id).ok_or_else(|| bad("csd.replace", format!("unknown product \"{id}\"")))?;
    let mut hs = device_handles(s, &targets(s, p)?)?;
    if p.get("allOfProduct").and_then(Value::as_bool).unwrap_or(false) {
        let d = s.doc()?;
        let of = |h: Handle| match d.entity(h).map(|e| &e.kind) {
            Some(EntityKind::Insert(i)) => product_attr(i).map(str::to_string),
            _ => None,
        };
        let ids: HashSet<String> = hs.iter().filter_map(|h| of(*h)).collect();
        let all: Vec<Handle> = d.space(&s.space()).map(|st| st.iter().map(|e| e.handle).collect()).unwrap_or_default();
        let more: Vec<Handle> = all.into_iter().filter(|h| !hs.contains(h) && of(*h).is_some_and(|id| ids.contains(&id))).collect();
        hs.extend(more);
    }
    let name = ensure_device_block(s, product, false)?;
    let two_name = ensure_device_block(s, product, true)?;
    let d = s.doc_mut()?;
    let (blk, two_blk) = (d.block(&name).cloned(), d.block(&two_name).cloned());
    let mut replaced = 0;
    for h in hs {
        d.modify_entity(h, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            let Some(a) = i.attribs.iter_mut().find(|a| a.tag == "PRODUCT") else { return };
            if a.text.value != product.id {
                a.text.value = product.id.clone();
                let two = two_line(i) && two_name != name;
                if !two && two_line(i) {
                    put_attr(i, "TWOLINE", String::new());
                }
                i.block = if two { two_name.clone() } else { name.clone() };
                let label = strip_text(product, two, attr(i, "NAME"));
                put_attr(i, "LABEL", label);
                if let Some(blk) = if two { &two_blk } else { &blk } {
                    fit_attribs(blk, i);
                }
                replaced += 1;
            }
        })?;
    }
    Ok(json!({ "replaced": replaced }))
}

/// Adds the text style the cards' text uses, once.
fn ensure_bold(d: &mut Drawing) {
    if d.text_style(BOLD_STYLE).is_none() {
        d.text_styles.push(bold_style());
    }
}

fn side_param(p: &Value, key: &str) -> Result<Side> {
    str_param(p, key).and_then(Side::from_id).ok_or_else(|| bad("csd.connect", format!("`{key}` must be top, bot, lft or rgt")))
}

fn product_attr(i: &Insert) -> Option<&str> {
    if !is_device_block(i) {
        return None;
    }
    i.attribs.iter().find(|a| a.tag == "PRODUCT").map(|a| a.text.value.as_str())
}

fn attr<'a>(i: &'a Insert, tag: &str) -> &'a str {
    i.attribs.iter().find(|a| a.tag == tag).map(|a| a.text.value.as_str()).unwrap_or_default()
}

const MAX_ACCESSORY_LINES: usize = 200;
const MAX_ACCESSORY_QTY: u64 = 100_000;

/// The accessories a device carries, as `(product id, qty)`; the `ACCS` attribute holds JSON `[[id, qty], ...]`.
fn accessories_of(i: &Insert) -> Vec<(String, u64)> {
    let rows: Vec<(String, u64)> = serde_json::from_str(attr(i, "ACCS")).unwrap_or_default();
    rows.into_iter().filter(|r| r.1 > 0).take(MAX_ACCESSORY_LINES).map(|(id, q)| (id, q.min(schedule::MAX_QTY))).collect()
}

const MAX_LINK_PAGE_CHARS: usize = 120;

fn supplied_by_others(i: &Insert) -> bool {
    attr(i, "SUPPLY") == "1"
}

/// The device `i` is mounted in, if it is still on the drawing.
fn mount_of(store: &EntityStore, i: &Insert) -> Option<Handle> {
    let h = Handle::parse_hex(attr(i, "MOUNT"))?;
    matches!(store.get(h).map(|e| &e.kind), Some(EntityKind::Insert(p)) if product_attr(p).is_some()).then_some(h)
}

/// How deep racks may nest when a move carries what is mounted in them.
const MAX_MOUNT_DEPTH: usize = 8;

/// Devices mounted in a device that moved go with it, unless they moved too (in the same
/// selection, or on their own). Each pass carries one more level of nesting.
fn carry_mounted(before: &Drawing, doc: &mut Arc<Drawing>) {
    let at = |d: &Drawing, h: Handle| match d.entity(h).map(|e| &e.kind) {
        Some(EntityKind::Insert(i)) => Some(i.insert.xy()),
        _ => None,
    };
    for _ in 0..MAX_MOUNT_DEPTH {
        let moves: Vec<(Handle, Vec2)> = spaces(doc)
            .iter()
            .filter_map(|sp| doc.space(sp))
            .flat_map(|store| {
                store.iter().filter_map(|e| {
                    let EntityKind::Insert(i) = &e.kind else { return None };
                    let parent = mount_of(store, i)?;
                    let (p0, p1, c0, c1) = (at(before, parent)?, at(doc, parent)?, at(before, e.handle)?, at(doc, e.handle)?);
                    (!p0.near(p1, 1e-9) && c0.near(c1, 1e-9)).then_some((e.handle, p1 - p0))
                })
            })
            .collect();
        if moves.is_empty() {
            return;
        }
        let d = Arc::make_mut(doc);
        for (h, delta) in moves {
            let _ = d.modify_entity(h, |e| e.kind.transform(&Mat3::translate(delta)));
        }
    }
}

/// Whether mounting `h` in `parent` would put a device inside itself; the walk is bounded so a
/// hand-edited cycle cannot spin it.
fn mount_cycles(store: &EntityStore, h: Handle, parent: Handle) -> bool {
    let mut at = parent;
    for _ in 0..=store.iter().count() {
        if at == h {
            return true;
        }
        match store.get(at).map(|e| &e.kind) {
            Some(EntityKind::Insert(i)) => match mount_of(store, i) {
                Some(next) => at = next,
                None => return false,
            },
            _ => return false,
        }
    }
    true
}

/// The accessory lines of a `csd.device.set` call: duplicates merged, zero quantities dropped.
fn accessory_param(v: &Value) -> Result<Vec<(String, u64)>> {
    let cmd = "csd.device.set";
    let rows = v.as_array().ok_or_else(|| bad(cmd, "`accessories` must be a list of {product, qty}"))?;
    if rows.len() > MAX_ACCESSORY_LINES {
        return Err(bad(cmd, format!("at most {MAX_ACCESSORY_LINES} accessory lines")));
    }
    let mut out: Vec<(String, u64)> = Vec::new();
    for r in rows {
        let id = str_param(r, "product").filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "each accessory needs a `product` id"))?;
        let qty = r
            .get("qty")
            .and_then(Value::as_u64)
            .filter(|q| *q <= MAX_ACCESSORY_QTY)
            .ok_or_else(|| bad(cmd, "`qty` must be a whole number up to 100000"))?;
        match out.iter_mut().find(|o| o.0 == id) {
            Some(o) => o.1 = (o.1 + qty).min(MAX_ACCESSORY_QTY),
            None => out.push((id.to_string(), qty)),
        }
    }
    out.retain(|o| o.1 > 0);
    Ok(out)
}

/// The one device `handle` (default: the selection) names.
fn one_device(s: &Session, cmd: &str, p: &Value) -> Result<Handle> {
    match device_handles(s, &targets(s, p)?)?.as_slice() {
        [h] => Ok(*h),
        _ => Err(bad(cmd, "`handle` (or a selection of exactly one device) is required")),
    }
}

/// (protocol, connections used, capacity) of every protocol `product` has ports for or `h` is wired with.
fn port_usage(product: &Product, links: &[WireLink], h: Handle) -> Vec<(Protocol, usize, u64)> {
    Protocol::ALL
        .iter()
        .filter_map(|&pr| {
            let (n, cap) = (used(links, h, pr), product.effective_capacity(pr));
            (!product.missing && (n > 0 || cap > 0)).then_some((pr, n, cap))
        })
        .collect()
}

fn run_device_info(s: &mut Session, p: &Value) -> Result<Value> {
    let h = one_device(s, "csd.device.info", p)?;
    let products = s.csd_library.clone();
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
    let Some(EntityKind::Insert(i)) = store.get(h).map(|e| &e.kind) else { return Err(bad("csd.device.info", "no such device")) };
    let id = product_attr(i).unwrap_or_default();
    let product = resolve(&products, id);
    let card = card_size(&product, two_line(i));
    let links = wires(store);
    let only: Vec<WireLink> = links.iter().map(|(_, l)| l.clone()).collect();
    let ports: Vec<Value> = port_usage(&product, &only, h)
        .into_iter()
        .map(|(pr, used, cap)| json!({ "protocol": pr.id(), "label": pr.label(), "used": used, "capacity": cap, "pooled": pr == Protocol::NetData }))
        .collect();
    let name_of = |o: Handle| match store.get(o).map(|e| &e.kind) {
        Some(EntityKind::Insert(oi)) => attr(oi, "NAME").to_string(),
        _ => String::new(),
    };
    let connections: Vec<Value> = links
        .iter()
        .filter(|(_, l)| l.from == h || l.to == h)
        .map(|(w, l)| {
            let out = l.from == h;
            let other = if out { l.to } else { l.from };
            json!({ "wire": w.hex(), "protocol": l.protocol.id(), "dir": if out { "out" } else { "in" }, "other": { "handle": other.hex(), "label": name_of(other) } })
        })
        .collect();
    let same = store.iter().filter(|e| matches!(&e.kind, EntityKind::Insert(o) if product_attr(o) == Some(id))).count();
    Ok(json!({
        "handle": h.hex(),
        "product": { "id": product.id, "name": product.name, "code": product.code, "brand": product.brand, "cat": product.cat, "color": product.color },
        "known": !product.missing,
        "label": attr(i, "NAME"),
        "notes": attr(i, "NOTES"),
        "linkPage": attr(i, "LINKPAGE"),
        "mount": mount_of(store, i).map(|m| json!({ "handle": m.hex(), "label": name_of(m) })),
        "accessories": accessories_of(i).iter().map(|(id, qty)| {
            let a = resolve(&products, id);
            json!({ "product": a.id, "name": a.name, "code": a.code, "category": a.cat, "qty": qty })
        }).collect::<Vec<_>>(),
        "supplyByOthers": supplied_by_others(i),
        "ports": ports,
        "connections": connections,
        "sameProduct": same,
        "at": [i.insert.x, i.insert.y],
        "size": [card.x * i.scale.x.abs(), card.y * i.scale.y.abs()],
        "libSize": [card.x, card.y],
        "twoLine": two_line(i),
    }))
}

/// Moves each of `i`'s attributes to where `blk` now defines it, keeping its value; attributes the
/// block doesn't define (MOUNT, ACCS...) stay as they are.
fn fit_attribs(blk: &cadcraft_doc::Block, i: &mut Insert) {
    let m = i.transform(blk.base.xy());
    for e in blk.entities.iter() {
        let EntityKind::AttDef(def) = &e.kind else { continue };
        let Some(a) = i.attribs.iter_mut().find(|a| a.tag == def.tag) else { continue };
        let mut k = EntityKind::Text(def.text.clone());
        k.transform(&m);
        if let EntityKind::Text(mut t) = k {
            t.value = std::mem::take(&mut a.text.value);
            a.text = t;
        }
        a.multiline = def.multiline.as_ref().map(|ml| ml.transformed(&m));
    }
}

/// Sets attribute `tag` on a device card, adding it (hidden) if the card lacks it.
fn put_attr(i: &mut Insert, tag: &str, value: String) {
    if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == tag) {
        a.text.value = value;
    } else if let Some(mut a) = i.attribs.iter().find(|a| a.tag == "NAME" || a.tag == "PRODUCT").cloned() {
        a.tag = tag.into();
        a.invisible = true;
        a.text.value = value;
        i.attribs.push(a);
    }
}

/// A device's label; blank falls back to the card's short code.
fn apply_label(i: &mut Insert, label: String, product: &Product) {
    let l = if label.trim().is_empty() { canvas_code(product) } else { label };
    put_attr(i, "LABEL", strip_text(product, two_line(i), &l));
    put_attr(i, "NAME", l);
}

fn run_device_set(s: &mut Session, p: &Value) -> Result<Value> {
    let h = one_device(s, "csd.device.set", p)?;
    let text = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.clone())),
        Some(_) => Err(bad("csd.device.set", format!("`{key}` must be text"))),
    };
    let (label, notes) = (text("label")?, text("notes")?);
    let link_page = text("linkPage")?.map(|t| t.trim().chars().take(MAX_LINK_PAGE_CHARS).collect::<String>());
    if link_page.is_some() {
        let d = s.doc()?;
        let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
        if !matches!(store.get(h).map(|e| &e.kind), Some(EntityKind::Insert(i)) if product_attr(i) == Some(OFF_PAGE_ID)) {
            return Err(bad("csd.device.set", "`linkPage` applies to off-page connectors only"));
        }
    }
    let mount = match text("mount")? {
        None => None,
        Some(t) if t.is_empty() => Some(String::new()),
        Some(t) => {
            let parent = Handle::parse_hex(&t).ok_or_else(|| bad("csd.device.set", "`mount` must be a device handle"))?;
            let d = s.doc()?;
            let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
            if !matches!(store.get(parent).map(|e| &e.kind), Some(EntityKind::Insert(i)) if product_attr(i).is_some()) {
                return Err(bad("csd.device.set", "`mount` is not a device on this drawing"));
            }
            if mount_cycles(store, h, parent) {
                return Err(bad("csd.device.set", "a device cannot be mounted in itself or in something mounted in it"));
            }
            Some(parent.hex())
        }
    };
    let accessories = p.get("accessories").filter(|v| !v.is_null()).map(accessory_param).transpose()?;
    let supply = match p.get("supplyByOthers") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad("csd.device.set", "`supplyByOthers` must be true or false")),
    };
    let two = match p.get("twoLine") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad("csd.device.set", "`twoLine` must be true or false")),
    };
    if two.is_some() && matches!(s.doc()?.entity(h).map(|e| &e.kind), Some(EntityKind::Insert(i)) if product_attr(i).is_some_and(is_builtin)) {
        return Err(bad("csd.device.set", "`twoLine` does not apply to built-in devices"));
    }
    if let Some(on) = two {
        twoline::set(s, &[h], on)?;
    }
    let products = s.csd_library.clone();
    s.doc_mut()?.modify_entity(h, |e| {
        let EntityKind::Insert(i) = &mut e.kind else { return };
        if let Some(l) = label {
            apply_label(i, l, &resolve(&products, product_attr(i).unwrap_or_default()));
        }
        if let Some(n) = notes {
            put_attr(i, "NOTES", n);
        }
        if let Some(l) = link_page {
            put_attr(i, "LINKSEE", if l.is_empty() { String::new() } else { truncate_label(&format!("See {l}"), card_of(i, &products).x) });
            put_attr(i, "LINKPAGE", l);
        }
        if let Some(m) = mount {
            put_attr(i, "MOUNT", m);
        }
        if let Some(a) = accessories {
            put_attr(i, "ACCS", if a.is_empty() { String::new() } else { serde_json::to_string(&a).unwrap_or_default() });
        }
        if let Some(b) = supply {
            put_attr(i, "SUPPLY", if b { "1".into() } else { String::new() });
        }
    })?;
    Ok(json!({ "handle": h.hex() }))
}

fn run_device_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let devs: HashSet<Handle> = device_handles(s, &targets(s, p)?)?.into_iter().collect();
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
    let linked: HashSet<Handle> = wires(store).into_iter().filter(|(_, l)| devs.contains(&l.from) || devs.contains(&l.to)).map(|(h, _)| h).collect();
    let decos = store.iter().filter(|e| deco_from_xdata(&e.common.xdata).is_some_and(|(w, _)| linked.contains(&w))).map(|e| e.handle);
    let doomed: Vec<Handle> = devs.iter().copied().chain(linked.iter().copied()).chain(decos).collect();
    let d = s.doc_mut()?;
    doomed.into_iter().for_each(|h| {
        d.remove_entity(h);
    });
    s.set_selection(Vec::new());
    Ok(json!({ "deleted": devs.len(), "wires": linked.len() }))
}

fn run_clear(s: &mut Session, _: &Value) -> Result<Value> {
    let d = s.doc()?;
    let (mut devices, mut wire_hs, mut decos) = (Vec::new(), Vec::new(), Vec::new());
    for sp in spaces(d) {
        for e in d.space(&sp).into_iter().flat_map(|st| st.iter()) {
            match &e.kind {
                EntityKind::Insert(i) if is_device_block(i) && product_attr(i).is_some() => devices.push(e.handle),
                EntityKind::LwPolyline(_) if WireLink::from_xdata(&e.common.xdata).is_some() => wire_hs.push(e.handle),
                _ if deco_from_xdata(&e.common.xdata).is_some() => decos.push(e.handle),
                _ => {}
            }
        }
    }
    let d = s.doc_mut()?;
    devices.iter().chain(&wire_hs).chain(&decos).for_each(|h| {
        d.remove_entity(*h);
    });
    s.set_selection(Vec::new());
    Ok(json!({ "devices": devices.len(), "wires": wire_hs.len() }))
}

/// Header variable: 1 when cards show the product code in the body rather than its picture. As in
/// the web app's "codes only", the label strip is unaffected; with no product pictures yet every
/// card already shows its code, so the setting is stored for when they exist.
const CODES_ONLY: &str = "CSDCODESONLY";

fn codes_only(d: &Drawing) -> bool {
    d.header.i64(CODES_ONLY, 0) != 0
}

fn run_display(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(on) = p.get("codesOnly").and_then(Value::as_bool) {
        let d = s.doc_mut()?;
        if on {
            d.header.set_i64(CODES_ONLY, 1);
        } else {
            d.header.vars.remove(CODES_ONLY);
        }
    }
    Ok(json!({ "codesOnly": codes_only(s.doc()?) }))
}

/// Shows every card's label again in drawings saved when "codes only" used to hide it.
fn show_labels(doc: &mut Arc<Drawing>) {
    let stale = |i: &Insert| is_device_block(i) && i.attribs.iter().any(|a| a.tag == "LABEL" && a.invisible);
    let todo: Vec<Handle> = spaces(doc)
        .iter()
        .filter_map(|sp| doc.space(sp))
        .flat_map(|st| st.iter())
        .filter(|e| matches!(&e.kind, EntityKind::Insert(i) if stale(i)))
        .map(|e| e.handle)
        .collect();
    if todo.is_empty() {
        return;
    }
    let d = Arc::make_mut(doc);
    for h in todo {
        let _ = d.modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.attribs.iter_mut().filter(|a| a.tag == "LABEL").for_each(|a| a.invisible = false);
            }
        });
    }
}

fn run_connect(s: &mut Session, p: &Value) -> Result<Value> {
    let handle = |key| handle_param(p, key).ok_or_else(|| bad("csd.connect", format!("`{key}` (object handle) is required")));
    let link = WireLink::new(
        handle("from")?,
        side_param(p, "fromSide")?,
        handle("to")?,
        side_param(p, "toSide")?,
        str_param(p, "protocol").and_then(Protocol::from_id).ok_or_else(|| bad("csd.connect", "`protocol` is not a known protocol id"))?,
    );
    let products = s.csd_library.clone();
    let warning = {
        let d = s.doc()?;
        let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
        let devs = devices(store, &products);
        let list: Vec<(Handle, &Product)> = devs.iter().map(|x| (x.handle, &*x.product)).collect();
        let existing: Vec<WireLink> = wires(store).into_iter().map(|(_, l)| l).collect();
        can_connect(&list, &existing, &link).map_err(|r| EngineError::Other(r.message))?;
        (link.protocol == Protocol::Network)
            .then(|| poe_verdict(&list, &existing, link.from, link.to, max_supply_watts(&products)))
            .flatten()
            .map(|v| v.message)
    };
    let h = add_wire(s, &link)?;
    let ortho = s.settings.orthomode;
    let library = s.csd_library.clone();
    refresh(&mut s.state_mut()?.doc, &library, ortho);
    Ok(match warning {
        Some(w) => json!({ "handle": h.hex(), "warning": w }),
        None => json!({ "handle": h.hex() }),
    })
}

/// Adds the wire for `link` without checking the ports or routing it.
fn add_wire(s: &mut Session, link: &WireLink) -> Result<Handle> {
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
    Ok(h)
}

/// The entity linetype scale that makes the first dash of `DASH` `len` long, whatever units the
/// library was loaded in; loads the linetype if the drawing lacks it.
fn dashed_scale(d: &mut Drawing, len: f64) -> f64 {
    linetype_scale(d, DASH, len)
}

/// Like [`dashed_scale`] for the standard linetype `name`, sized by its first non-zero element.
fn linetype_scale(d: &mut Drawing, name: &str, len: f64) -> f64 {
    if d.linetype(name).is_none() {
        let lt = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == name);
        d.linetypes.extend(lt);
    }
    d.linetype(name).and_then(|lt| lt.pattern.iter().map(|e| e.length.abs()).find(|l| *l > 0.0)).map_or(1.0, |l| len / l)
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
    let bytes = read_library(path).map_err(|e| bad("csd.library.load", format!("{path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| bad("csd.library.load", format!("{path}: not UTF-8 text")))?;
    let products = parse_library(&text).map_err(|e| bad("csd.library.load", format!("{path}: {e}")))?;
    s.csd_library = Arc::new(products);
    refresh_open_drawings(s);
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
    let fits: Option<Vec<&str>> = p.get("accessoriesFor").and_then(Value::as_array).map(|a| {
        let fitting: Vec<&Product> = a.iter().filter_map(Value::as_str).filter_map(|id| s.csd_library.iter().find(|x| x.id == id)).collect();
        accessory_products(&s.csd_library, &fitting).iter().map(|x| x.id.as_str()).collect()
    });
    let hit = |x: &Product| {
        fits.as_ref().is_none_or(|f| f.contains(&x.id.as_str()))
            && category.is_none_or(|c| x.cat == c)
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
    refresh_open_drawings(s);
    Ok(json!({ "count": s.csd_library.len() }))
}

fn run_library_save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("csd.library.save", "`path` is required"))?;
    write_library(path, library_json(&s.csd_library).as_bytes()).map_err(|e| bad("csd.library.save", format!("{path}: {e}")))?;
    Ok(json!({ "path": path, "count": s.csd_library.len() }))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_library(path: &str) -> std::io::Result<Vec<u8>> {
    crate::capped_read::read_capped(std::path::Path::new(path), cadcraft_csd::MAX_LIBRARY_BYTES)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_library(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    crate::atomic_write::write_atomic_making_folders(std::path::Path::new(path), bytes)
}

#[cfg(target_arch = "wasm32")]
fn read_library(_: &str) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "files are not available on the web"))
}

#[cfg(target_arch = "wasm32")]
fn write_library(_: &str, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "files are not available on the web"))
}

pub(crate) fn after_command(s: &mut Session, before: Option<&Arc<Drawing>>) {
    // Ortho is a session setting, so a toggle must re-route although no drawing changed.
    let ortho = s.settings.orthomode;
    let restyled = std::mem::replace(&mut s.csd_ortho, ortho) != ortho;
    let library = s.csd_library.clone();
    if let Ok(st) = s.state_mut()
        && (restyled || before.is_some_and(|b| !Arc::ptr_eq(b, &st.doc)))
    {
        if let Some(b) = before {
            mount::auto_mount(b, &mut st.doc, &library);
            mount::stack(&mut st.doc);
            mount::lift_wires(b, &mut st.doc);
            carry_mounted(b, &mut st.doc);
        }
        refresh(&mut st.doc, &library, ortho);
    }
}

#[cfg(test)]
mod tests;
