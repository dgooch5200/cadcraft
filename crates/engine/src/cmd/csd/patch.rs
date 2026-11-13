//! DMX modes, the patch list and its exports, as the web app's patch sheet does it: every device
//! that takes channels is packed into universes in patch order, no fixture straddling a universe.

use std::cmp::Ordering;

use cadcraft_csd::{CARD_H, Product, is_builtin};
use cadcraft_doc::{EntityKind, HVal, Handle};
use serde_json::{Value, json};

use super::{accessories_of, attr, bad, device_handles, product_attr, resolve, schedule, str_param};
use crate::cmd::targets;
use crate::{EngineError, Result, Session};

#[cfg(test)]
mod tests;

const UNIVERSE_SIZE: u32 = 512;
/// The attribute holding a device's chosen mode name; absent means the product's largest.
pub(super) const MODE_TAG: &str = "MODE";
/// Header variables: the drawing's patch order (absent = drawing order) and `0` when addressing is
/// switched off (absent = on).
const ORDER_VAR: &str = "CSDPATCHORDER";
const ADDRESSING_VAR: &str = "CSDADDRESSING";
const ORDERS: [&str; 4] = ["drawing", "name", "product", "position"];
/// Accessory and unplaced quantities are user-sized; this bounds the rows they expand to.
const MAX_ROWS: usize = 20_000;
/// Matches the library loader's channel cap.
const MAX_MODE_CHANNELS: u32 = 1_000_000;

/// A product's modes from its `modes` cell (`17ch:17, 34ch:34`): the last colon splits name from
/// channels, and an entry with no name or no positive whole number is dropped.
fn parse_modes(p: &Product) -> Vec<(String, u32)> {
    p.modes
        .split(',')
        .filter_map(|e| {
            let (name, n) = e.rsplit_once(':')?;
            let n = n.trim().parse::<u32>().ok().filter(|&n| n > 0)?.min(MAX_MODE_CHANNELS);
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), n))
        })
        .collect()
}

/// The widest mode, the first one on a tie.
fn widest(modes: &[(String, u32)]) -> Option<&(String, u32)> {
    modes.iter().reduce(|a, b| if b.1 > a.1 { b } else { a })
}

#[derive(Clone)]
struct Entry {
    /// None for an unplaced product: it is on no sheet to locate or re-mode.
    handle: Option<Handle>,
    /// The device an accessory rides on; it follows that device in the patch, whatever the order.
    carried_by: Option<String>,
    accessories: Vec<Entry>,
    label: String,
    product: Product,
    modes: Vec<(String, u32)>,
    /// The chosen mode, if the product still offers it.
    mode: Option<String>,
    channels: u32,
    at: [f64; 2],
}

impl Entry {
    /// The mode a console is told: the chosen one, else the one the budget assumes.
    fn mode_text(&self) -> String {
        self.mode.clone().or_else(|| widest(&self.modes).map(|m| m.0.clone())).unwrap_or_else(|| format!("{} ch", self.product.channels))
    }
}

fn entries(s: &Session) -> Result<Vec<Entry>> {
    let d = s.doc()?;
    let store = d.space(&s.space()).ok_or_else(|| EngineError::Other("no such space".into()))?;
    let mut left = MAX_ROWS;
    Ok(store
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(i) = &e.kind else { return None };
            let product = resolve(&s.csd_library, product_attr(i)?).into_owned();
            if is_builtin(&product.id) {
                return None;
            }
            let label = attr(i, "NAME").to_string();
            let mut dev = entry(Some(e.handle), label.clone(), product, attr(i, MODE_TAG), [i.insert.x, i.insert.y]);
            for (id, qty) in accessories_of(i) {
                let part = resolve(&s.csd_library, &id).into_owned();
                if !is_builtin(&part.id) {
                    let a = Entry { carried_by: Some(label.clone()), ..entry(None, part.code.clone(), part, "", dev.at) };
                    let n = units(qty).min(left);
                    left -= n;
                    dev.accessories.extend(std::iter::repeat_n(a, n));
                }
            }
            Some(dev)
        })
        .collect())
}

fn units(qty: u64) -> usize {
    usize::try_from(qty).unwrap_or(MAX_ROWS).min(MAX_ROWS)
}

/// An entry for `product` at the chosen mode name `mode` (ignored if the product no longer offers it).
fn entry(handle: Option<Handle>, label: String, product: Product, mode: &str, at: [f64; 2]) -> Entry {
    let modes = parse_modes(&product);
    let mode = Some(mode).filter(|m| modes.iter().any(|x| x.0 == *m)).map(String::from);
    let channels = match &mode {
        Some(m) => modes.iter().find(|x| x.0 == *m).map(|x| x.1),
        None => widest(&modes).map(|x| x.1),
    }
    .unwrap_or(product.channels);
    Entry { handle, carried_by: None, accessories: Vec::new(), label, product, modes, mode, channels, at }
}

/// "2" < "10": digit runs compare as numbers, the rest case-insensitively.
fn natural(a: &str, b: &str) -> Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, t)) if *d == digit => t.extend(c.to_lowercase()),
                _ => out.push((digit, c.to_lowercase().collect())),
            }
        }
        out
    }
    for (x, y) in chunks(a).iter().zip(chunks(b).iter()) {
        let o = if x.0 && y.0 {
            let (p, q) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            p.len().cmp(&q.len()).then_with(|| p.cmp(q))
        } else {
            x.1.cmp(&y.1)
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// Orders `v` for patching: `drawing` keeps the drawing's order; `position` reads rows top to
/// bottom (a card height apart is another row) and each row left to right.
fn order(v: &mut [Entry], by: &str, cmd: &str, k: f64) -> Result<()> {
    match by {
        "drawing" => {}
        "name" => v.sort_by(|a, b| natural(&a.label, &b.label)),
        "product" => v.sort_by(|a, b| a.product.name.to_lowercase().cmp(&b.product.name.to_lowercase()).then_with(|| natural(&a.label, &b.label))),
        "position" => {
            let band = |e: &Entry| (-e.at[1] / (CARD_H * k)).floor();
            v.sort_by(|a, b| band(a).total_cmp(&band(b)).then_with(|| a.at[0].total_cmp(&b.at[0])));
        }
        _ => return Err(bad(cmd, "`order` must be drawing, name, product or position")),
    }
    Ok(())
}

/// Where each footprint lands as (universe, address, universes spanned), both 1-based. A fixture
/// never straddles a universe boundary; one wider than a universe takes whole universes alone.
fn assign(footprints: &[u32]) -> Vec<(u32, u32, u32)> {
    let (mut universe, mut used) = (1u32, 0u32);
    footprints
        .iter()
        .map(|&n| {
            let spans = n.div_ceil(UNIVERSE_SIZE).max(1);
            if spans > 1 {
                if used > 0 {
                    universe = universe.saturating_add(1);
                }
                let slot = (universe, 1, spans);
                universe = universe.saturating_add(spans);
                used = 0;
                return slot;
            }
            if used.saturating_add(n) > UNIVERSE_SIZE {
                universe = universe.saturating_add(1);
                used = 0;
            }
            let slot = (universe, used + 1, 1);
            used += n;
            slot
        })
        .collect()
}

struct Plan {
    rows: Vec<(Entry, u32, u32)>,
    used: u32,
    budget: u32,
}

/// The drawing's saved patch order, drawing order if it never chose one.
fn saved_order(d: &cadcraft_doc::Drawing) -> &str {
    d.header.get(ORDER_VAR).and_then(HVal::as_str).filter(|o| ORDERS.contains(o)).unwrap_or("drawing")
}

fn addressing_on(d: &cadcraft_doc::Drawing) -> bool {
    d.header.get(ADDRESSING_VAR).and_then(HVal::as_i64) != Some(0)
}

fn plan(s: &Session, by: &str, cmd: &str) -> Result<Plan> {
    let mut devs = entries(s)?;
    if !addressing_on(s.doc()?) {
        return Ok(Plan { rows: Vec::new(), used: 0, budget: 0 });
    }
    // What the controllers on the drawing can originate.
    let budget = devs.iter().map(|e| e.product.universes).fold(0u32, u32::saturating_add);
    order(&mut devs, by, cmd, super::units::px(s.doc()?))?;
    // Accessories follow their device and products not yet drawn come last, a unit each: four
    // fixtures need four addresses.
    let mut all: Vec<Entry> =
        devs.into_iter().flat_map(|d| std::iter::once(Entry { accessories: Vec::new(), ..d.clone() }).chain(d.accessories)).collect();
    for (id, qty) in schedule::read(s.doc()?) {
        let part = resolve(&s.csd_library, &id).into_owned();
        if !is_builtin(&part.id) {
            all.extend(std::iter::repeat_n(entry(None, part.code.clone(), part, "", [0.0, 0.0]), units(qty).min(MAX_ROWS.saturating_sub(all.len()))));
        }
    }
    all.truncate(MAX_ROWS);
    all.retain(|e| e.channels > 0);
    let slots = assign(&all.iter().map(|e| e.channels).collect::<Vec<_>>());
    let used = slots.iter().map(|&(u, _, n)| u.saturating_add(n).saturating_sub(1)).max().unwrap_or(0);
    let rows = all.into_iter().zip(slots).map(|(e, (u, a, _))| (e, u, a)).collect();
    Ok(Plan { rows, used, budget })
}

fn order_param<'a>(s: &'a Session, p: &'a Value) -> &'a str {
    str_param(p, "order").unwrap_or_else(|| s.doc().map_or("drawing", saved_order))
}

pub(super) fn run_patch(s: &mut Session, p: &Value) -> Result<Value> {
    let plan = plan(s, order_param(s, p), "csd.patch")?;
    let rows: Vec<Value> = plan
        .rows
        .iter()
        .map(|(e, universe, address)| {
            json!({
                "handle": e.handle.map(|h| h.hex()), "carriedBy": e.carried_by, "label": e.label, "product": e.product.id, "name": e.product.name, "mode": e.mode,
                "assumedMode": e.mode_text(), "modes": e.modes.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(),
                "channels": e.channels, "universe": universe, "address": address,
            })
        })
        .collect();
    let d = s.doc()?;
    Ok(
        json!({ "order": saved_order(d), "addressing": addressing_on(d), "rows": rows, "universes": { "used": plan.used, "budget": plan.budget, "over": plan.budget > 0 && plan.used > plan.budget } }),
    )
}

pub(super) fn run_device_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.device.mode";
    let mode = match p.get("mode") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(m)) => m.trim().to_string(),
        Some(_) => return Err(bad(cmd, "`mode` must be text")),
    };
    let hs = device_handles(s, &targets(s, p)?)?;
    // Validate every device before changing any.
    if !mode.is_empty() {
        let all = entries(s)?;
        for h in &hs {
            let offered = all.iter().find(|e| e.handle == Some(*h)).is_some_and(|e| e.modes.iter().any(|m| m.0 == mode));
            if !offered {
                return Err(bad(cmd, format!("mode \"{mode}\" is not offered by device {}'s product", h.hex())));
            }
        }
    }
    let d = s.doc_mut()?;
    for h in &hs {
        d.modify_entity(*h, |e| {
            let EntityKind::Insert(i) = &mut e.kind else { return };
            if let Some(a) = i.attribs.iter_mut().find(|a| a.tag == MODE_TAG) {
                a.text.value = mode.clone();
            } else if !mode.is_empty()
                && let Some(mut a) = i.attribs.iter().find(|a| a.tag == "NAME" || a.tag == "PRODUCT").cloned()
            {
                a.tag = MODE_TAG.into();
                a.invisible = true;
                a.text.value = mode.clone();
                i.attribs.push(a);
            }
        })?;
    }
    Ok(json!({ "changed": hs.len() }))
}

/// ASCII only, because a `.csv` does not say its encoding and a console guesses: accents drop,
/// other Latin letters and typed punctuation map to their usual ASCII spelling, the rest is dropped.
fn ascii(t: &str) -> String {
    // Latin-1 letters U+00C0..=U+00FF folded to their base letter ('-' = no letter).
    const FOLD: &str = "AAAAAAACEEEEIIIIDNOOOOO-OUUUUYTsaaaaaaaceeeeiiiidnooooo-ouuuuyty";
    let mut out = String::new();
    for c in t.chars() {
        match c {
            ' '..='~' => out.push(c),
            '\u{2013}' | '\u{2014}' | '\u{b7}' => out.push('-'),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            '\u{d7}' => out.push('x'),
            '\u{b0}' => out.push_str("deg"),
            '\u{a0}' => out.push(' '),
            '\u{c6}' => out.push_str("AE"),
            '\u{e6}' => out.push_str("ae"),
            '\u{df}' => out.push_str("ss"),
            '\u{de}' => out.push_str("TH"),
            '\u{fe}' => out.push_str("th"),
            '\u{c0}'..='\u{ff}' => {
                let i = c as usize - 0xc0;
                out.extend(FOLD.chars().nth(i).filter(|&f| f != '-'));
            }
            _ => {}
        }
    }
    out.trim().to_string()
}

fn quoted(t: String) -> String {
    if t.contains([',', '"', '\n', '\r']) { format!("\"{}\"", t.replace('"', "\"\"")) } else { t }
}

/// MagicQ's Fixture Patch CSV: every field but the last is followed by a space before its comma,
/// an empty field is that lone space, lines end LF, and `DMX` is `%02d-%03d`. The device label goes
/// in `Position`; `Hang` stays empty.
fn magicq(plan: &Plan) -> String {
    let line = |f: &[String]| f.join(" ,");
    let mut out = line(&["Head No", "DMX", "Position", "Hang", "Manufacturer", "Model", "Mode"].map(String::from));
    out.push('\n');
    for (n, (e, u, a)) in plan.rows.iter().enumerate() {
        let f = [
            (n + 1).to_string(),
            format!("{u:02}-{a:03}"),
            quoted(ascii(&e.label)),
            String::new(),
            quoted(ascii(&e.product.brand)),
            quoted(ascii(&e.product.name)),
            quoted(ascii(&e.mode_text())),
        ];
        out.push_str(&line(&f));
        out.push('\n');
    }
    out
}

/// A plain patch list for spreadsheets, with the BOM export's formula-escaping.
fn csv(plan: &Plan) -> String {
    let cell = |t: &str| quoted(if t.starts_with(['=', '+', '-', '@']) { format!("'{t}") } else { t.to_string() });
    let mut out = String::from("Head No,Universe,Address,Label,Manufacturer,Product,Mode,Channels\r\n");
    for (n, (e, u, a)) in plan.rows.iter().enumerate() {
        let f = [
            (n + 1).to_string(),
            u.to_string(),
            a.to_string(),
            cell(&e.label),
            cell(&e.product.brand),
            cell(&e.product.name),
            cell(&e.mode_text()),
            e.channels.to_string(),
        ];
        out.push_str(&f.join(","));
        out.push_str("\r\n");
    }
    out
}

fn universes_text(plan: &Plan) -> String {
    format!("Universes used: {}{}", plan.used, if plan.budget > 0 { format!(" of {} available", plan.budget) } else { String::new() })
}

/// Rows grouped by universe, ascending: (universe, header title, header note, row indices).
fn universe_groups(plan: &Plan) -> Vec<(u32, String, String, Vec<usize>)> {
    use std::collections::BTreeMap;
    let size = u64::from(UNIVERSE_SIZE);
    let mut by_uni: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, (_, u, _)) in plan.rows.iter().enumerate() {
        by_uni.entry(*u).or_default().push(i);
    }
    by_uni
        .into_iter()
        .map(|(u, list)| {
            let used: u64 = list.iter().filter_map(|&i| plan.rows.get(i)).map(|(e, _, _)| u64::from(e.channels).min(size)).sum();
            (u, format!("Universe {u}"), format!("{used} of {size} channels used"), list)
        })
        .collect()
}

/// The patch page: a header row per universe with its channels used. A fixture spanning several
/// universes is listed under its first, with one note naming the last universe it reaches.
fn html(s: &Session, p: &Value, plan: &Plan) -> String {
    use super::html::{footer_lines, group_row, page, row};
    let size = u64::from(UNIVERSE_SIZE);
    let span = |e: &Entry, a: u32| (u64::from(a).saturating_sub(1) + u64::from(e.channels)).div_ceil(size).max(1);
    let mut body = String::new();
    for (u, title, note, list) in universe_groups(plan) {
        body.push_str(&group_row("group", 6, &title, &note));
        for i in list {
            let Some((e, _, a)) = plan.rows.get(i) else { continue };
            let last = (a + e.channels).saturating_sub(1).min(UNIVERSE_SIZE);
            let n = span(e, *a);
            let mut item = format!("{} - {}", e.product.code, e.product.name);
            if let Some(c) = &e.carried_by {
                item.push_str(&format!(" (accessory of {c})"));
            }
            let mode = if e.mode.is_some() { e.mode_text() } else { format!("{} (assumed)", e.mode_text()) };
            let addr = if n > 1 {
                format!("{a}-{last} (spans {n} universes, continued through universe {})", u64::from(u) + n - 1)
            } else {
                format!("{a}-{last}")
            };
            body.push_str(&row("", &[format!("{u}-{a}"), (i + 1).to_string(), format!("{} / {item}", e.label), mode, e.channels.to_string(), addr]));
        }
    }
    let mut footer = footer_lines(s, p);
    footer.insert(0, universes_text(plan));
    let info = [("Heads", plan.rows.len().to_string()), ("Universes", universes_text(plan).replace("Universes used: ", ""))];
    page("DMX Patch", &info, &["DMX", "Head", "Fixture", "Mode", "Ch", "Addresses"], &body, &footer)
}

/// The patch sheet: one row per head with its universe-address, as the web app's printPatch lists it.
fn pdf(s: &Session, p: &Value, plan: &Plan) -> Result<Vec<u8>> {
    const COLS: [(&str, f64); 7] =
        [("DMX", 70.0), ("Head", 40.0), ("Fixture", 200.0), ("Product", 220.0), ("Mode", 120.0), ("Ch", 40.0), ("Addresses", 110.0)];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (_, title, note, list) in universe_groups(plan) {
        rows.push(vec![format!("{title}  {note}")]);
        for i in list {
            let Some((e, u, a)) = plan.rows.get(i) else { continue };
            let last = a.saturating_add(e.channels).saturating_sub(1).min(UNIVERSE_SIZE);
            let carried = e.carried_by.as_ref().map(|c| format!(" (accessory of {c})")).unwrap_or_default();
            rows.push(vec![
                format!("{u}-{a}"),
                (i + 1).to_string(),
                format!("{}{carried}", e.label),
                format!("{} - {}", e.product.code, e.product.name),
                e.mode_text(),
                e.channels.to_string(),
                format!("{a}-{last}"),
            ]);
        }
    }
    let mut footer = super::html::footer_lines(s, p);
    footer.insert(0, universes_text(plan));
    schedule::table_pdf("csd.patch.export", "DMX Patch", &COLS, &rows, &footer)
}

pub(super) fn run_patch_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.patch.export";
    let path = str_param(p, "path").ok_or_else(|| bad(cmd, "`path` is required"))?;
    let plan = plan(s, order_param(s, p), cmd)?;
    let bytes = match str_param(p, "format") {
        Some("csv") => csv(&plan).into_bytes(),
        Some("magicq") => magicq(&plan).into_bytes(),
        Some("pdf") => pdf(s, p, &plan)?,
        Some("html") => html(s, p, &plan).into_bytes(),
        _ => return Err(bad(cmd, "`format` must be csv, magicq, pdf or html")),
    };
    super::write_library(path, &bytes).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok(json!({ "path": path, "rows": plan.rows.len() }))
}

pub(super) fn run_patch_setup(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.patch.setup";
    let order = match str_param(p, "order") {
        Some(o) if ORDERS.contains(&o) => Some(o),
        Some(_) => return Err(bad(cmd, "`order` must be drawing, name, product or position")),
        None => None,
    };
    let addressing = match p.get("addressing") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad(cmd, "`addressing` must be true or false")),
    };
    let d = s.doc_mut()?;
    match order {
        Some("drawing") => drop(d.header.vars.remove(ORDER_VAR)),
        Some(o) => d.header.set_str(ORDER_VAR, o),
        None => {}
    }
    match addressing {
        Some(true) => drop(d.header.vars.remove(ADDRESSING_VAR)),
        Some(false) => d.header.set(ADDRESSING_VAR, HVal::Int(0)),
        None => {}
    }
    Ok(json!({ "order": saved_order(d), "addressing": addressing_on(d) }))
}
