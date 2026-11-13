//! DMX modes, the patch list and its exports, as the web app's patch sheet does it: every device
//! that takes channels is packed into universes in patch order, no fixture straddling a universe.

use std::cmp::Ordering;

use cadcraft_csd::{CARD_H, Product, is_builtin};
use cadcraft_doc::{EntityKind, Handle};
use serde_json::{Value, json};

use super::{attr, bad, device_handles, product_attr, resolve, str_param};
use crate::cmd::targets;
use crate::{EngineError, Result, Session};

#[cfg(test)]
mod tests;

const UNIVERSE_SIZE: u32 = 512;
/// The attribute holding a device's chosen mode name; absent means the product's largest.
const MODE_TAG: &str = "MODE";

/// A product's modes from its `modes` cell (`17ch:17, 34ch:34`): the last colon splits name from
/// channels, and an entry with no name or no positive whole number is dropped.
fn parse_modes(p: &Product) -> Vec<(String, u32)> {
    p.modes
        .split(',')
        .filter_map(|e| {
            let (name, n) = e.rsplit_once(':')?;
            let n = n.trim().parse::<u32>().ok().filter(|&n| n > 0)?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), n))
        })
        .collect()
}

/// The widest mode, the first one on a tie.
fn widest(modes: &[(String, u32)]) -> Option<&(String, u32)> {
    modes.iter().reduce(|a, b| if b.1 > a.1 { b } else { a })
}

struct Entry {
    handle: Handle,
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
    Ok(store
        .iter()
        .filter_map(|e| {
            let EntityKind::Insert(i) = &e.kind else { return None };
            let product = resolve(&s.csd_library, product_attr(i)?).into_owned();
            if is_builtin(&product.id) {
                return None;
            }
            let modes = parse_modes(&product);
            let mode = Some(attr(i, MODE_TAG)).filter(|m| modes.iter().any(|x| x.0 == *m)).map(String::from);
            let channels = match &mode {
                Some(m) => modes.iter().find(|x| x.0 == *m).map(|x| x.1),
                None => widest(&modes).map(|x| x.1),
            }
            .unwrap_or(product.channels);
            Some(Entry { handle: e.handle, label: attr(i, "NAME").to_string(), product, modes, mode, channels, at: [i.insert.x, i.insert.y] })
        })
        .collect())
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
fn order(v: &mut [Entry], by: &str, cmd: &str) -> Result<()> {
    match by {
        "drawing" => {}
        "name" => v.sort_by(|a, b| natural(&a.label, &b.label)),
        "product" => v.sort_by(|a, b| a.product.name.to_lowercase().cmp(&b.product.name.to_lowercase()).then_with(|| natural(&a.label, &b.label))),
        "position" => {
            let band = |e: &Entry| (-e.at[1] / CARD_H).floor();
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

fn plan(s: &Session, by: &str, cmd: &str) -> Result<Plan> {
    let mut all = entries(s)?;
    // What the controllers on the drawing can originate.
    let budget = all.iter().map(|e| e.product.universes).fold(0u32, u32::saturating_add);
    order(&mut all, by, cmd)?;
    all.retain(|e| e.channels > 0);
    let slots = assign(&all.iter().map(|e| e.channels).collect::<Vec<_>>());
    let used = slots.iter().map(|&(u, _, n)| u.saturating_add(n).saturating_sub(1)).max().unwrap_or(0);
    let rows = all.into_iter().zip(slots).map(|(e, (u, a, _))| (e, u, a)).collect();
    Ok(Plan { rows, used, budget })
}

fn order_param(p: &Value) -> &str {
    str_param(p, "order").unwrap_or("drawing")
}

pub(super) fn run_patch(s: &mut Session, p: &Value) -> Result<Value> {
    let plan = plan(s, order_param(p), "csd.patch")?;
    let rows: Vec<Value> = plan
        .rows
        .iter()
        .map(|(e, universe, address)| {
            json!({
                "handle": e.handle.hex(), "label": e.label, "product": e.product.id, "name": e.product.name, "mode": e.mode,
                "assumedMode": e.mode_text(), "modes": e.modes.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(),
                "channels": e.channels, "universe": universe, "address": address,
            })
        })
        .collect();
    Ok(json!({ "rows": rows, "universes": { "used": plan.used, "budget": plan.budget, "over": plan.budget > 0 && plan.used > plan.budget } }))
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
            let offered = all.iter().find(|e| e.handle == *h).is_some_and(|e| e.modes.iter().any(|m| m.0 == mode));
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
    if t.contains([',', '"', '\n']) { format!("\"{}\"", t.replace('"', "\"\"")) } else { t }
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

pub(super) fn run_patch_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.patch.export";
    let path = str_param(p, "path").ok_or_else(|| bad(cmd, "`path` is required"))?;
    let plan = plan(s, order_param(p), cmd)?;
    let text = match str_param(p, "format") {
        Some("csv") => csv(&plan),
        Some("magicq") => magicq(&plan),
        _ => return Err(bad(cmd, "`format` must be csv or magicq")),
    };
    super::write_library(path, text.as_bytes()).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    Ok(json!({ "path": path, "rows": plan.rows.len() }))
}
