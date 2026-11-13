//! Hardware connection rules from ChamSys Designer's `rules.ts`: manufacturer limits that are not a
//! port count. Each refusal carries the ChamSys rule id. Walks are over the existing links, seeded
//! from both ends of the proposed one, so the two runs it would join count as one.

use crate::{Product, Protocol, Refusal, WireLink, used};
use cadcraft_doc::Handle;
use std::collections::{HashMap, HashSet};

/// Devices per 10Scene bus, not counting the device driving it.
pub const SCENE10_MAX_DEVICES_PER_SEGMENT: usize = 10;
pub const MR16_MAX_PER_POE_X: usize = 8;
pub const EPIX_FLEX_MAX_PER_SEGMENT: usize = 2;
pub const EPIX_FLEX_MAX_PER_OUTPUT: usize = 4;
pub const EPIX_LINE_MAX_PIXELS_PER_OUTPUT: u32 = 1000;

/// Longest needle first: "EPIX LINE 10 IP" is a prefix of "EPIX LINE 10 IP S".
const EPIX_LINE_PIXELS: [(&str, u32); 3] = [("EPIX LINE 10 IP S", 50), ("EPIX LINE 10 IP", 100), ("EPIXL10IP", 100)];

fn includes(p: &Product, needle: &str) -> bool {
    p.name.to_uppercase().contains(needle) || p.code.to_uppercase().contains(needle)
}
fn is_mr16(p: &Product) -> bool {
    includes(p, "MR16")
}
fn is_poe_x(p: &Product) -> bool {
    includes(p, "POE-X")
}
fn is_flex(p: &Product) -> bool {
    includes(p, "EPIX FLEX 20")
}
fn is_boost(p: &Product) -> bool {
    includes(p, "EPIX FLEX BOOST")
}
fn is_load(p: &Product) -> bool {
    p.poe_watts > 0.0
}
fn line_pixels(p: &Product) -> u32 {
    EPIX_LINE_PIXELS.iter().find(|(n, _)| includes(p, n)).map_or(0, |(_, px)| *px)
}
/// A 10Scene bus starts at a device with no 10Scene input.
fn is_bus_origin(p: &Product) -> bool {
    p.ports(Protocol::Scene10).0 == 0
}

struct Ctx<'a> {
    products: HashMap<Handle, &'a Product>,
    links: &'a [WireLink],
    /// Neighbours of each device per protocol, built once.
    adj: HashMap<(Protocol, Handle), Vec<Handle>>,
    from: (Handle, &'a Product),
    to: (Handle, &'a Product),
}

impl Ctx<'_> {
    fn product(&self, h: Handle) -> Option<&Product> {
        self.products.get(&h).copied()
    }

    /// Members reachable from either end over `protocol` links, crossing only members.
    fn run(&self, protocol: Protocol, is_member: impl Fn(&Product) -> bool) -> Vec<&Product> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut queue = vec![self.from.0, self.to.0];
        while let Some(h) = queue.pop() {
            let Some(p) = self.product(h) else { continue };
            if !is_member(p) || !seen.insert(h) {
                continue;
            }
            out.push(p);
            queue.extend(self.adj.get(&(protocol, h)).into_iter().flatten());
        }
        out
    }
}

fn scn_bus_max10(c: &Ctx) -> Option<String> {
    let n = c.run(Protocol::Scene10, |p| !is_bus_origin(p)).len();
    (n > SCENE10_MAX_DEVICES_PER_SEGMENT)
        .then(|| format!("10Scene limit: this bus would carry {n} devices — max is {SCENE10_MAX_DEVICES_PER_SEGMENT} per bus output"))
}

fn mr16_poe_x(c: &Ctx) -> Option<String> {
    for (near, far) in [(c.from.1, c.to.1), (c.to.1, c.from.1)] {
        if is_poe_x(near) && !is_mr16(far) && (is_load(far) || is_poe_x(far)) {
            return Some(format!("A POE-X takes PoE from a supply and feeds Logic MR16 fixtures only — \"{}\" is neither", far.code));
        }
        if is_mr16(near) && !is_mr16(far) && !is_poe_x(far) {
            return Some(format!("Logic MR16 takes PoE from a POE-X or another MR16 — \"{}\" is neither", far.code));
        }
    }
    None
}

fn mr16_max8(c: &Ctx) -> Option<String> {
    if !is_mr16(c.from.1) && !is_mr16(c.to.1) {
        return None;
    }
    let n = c.run(Protocol::Network, |p| is_mr16(p) || is_poe_x(p)).into_iter().filter(|p| is_mr16(p)).count();
    (n > MR16_MAX_PER_POE_X).then(|| format!("MR16 limit: this run would hold {n} fixtures — a POE-X carries at most {MR16_MAX_PER_POE_X}"))
}

fn fixture_net_thru(c: &Ctx) -> Option<String> {
    for (h, p) in [c.from, c.to] {
        if is_mr16(p) {
            return Some("Logic MR16 has no data port — NET passes through the POE-X, not the fixture".into());
        }
        if is_load(p) && used(c.links, h, Protocol::Network) == 0 {
            return Some(format!("\"{}\" needs a PoE cable powering it before it can pass NET on", p.code));
        }
    }
    None
}

fn epix_flex_max(c: &Ctx) -> Option<String> {
    if !is_flex(c.from.1) && !is_flex(c.to.1) {
        return None;
    }
    let leg = c.run(Protocol::Epix3p, is_flex).len();
    if leg > EPIX_FLEX_MAX_PER_SEGMENT {
        return Some(format!(
            "EPIX limit: this leg would carry {leg} Flex 20 — max is {EPIX_FLEX_MAX_PER_SEGMENT}, and a Flex Boost after the second drives {EPIX_FLEX_MAX_PER_SEGMENT} more"
        ));
    }
    let out = c.run(Protocol::Epix3p, |p| is_flex(p) || is_boost(p)).into_iter().filter(|p| is_flex(p)).count();
    (out > EPIX_FLEX_MAX_PER_OUTPUT).then(|| {
        format!("EPIX limit: this output would carry {out} Flex 20 — max is {EPIX_FLEX_MAX_PER_OUTPUT}, however many Flex Boosts are in the run")
    })
}

fn epix_line_max(c: &Ctx) -> Option<String> {
    if line_pixels(c.from.1) == 0 && line_pixels(c.to.1) == 0 {
        return None;
    }
    let px: u32 = c.run(Protocol::Epix4p, |p| line_pixels(p) > 0).into_iter().map(line_pixels).sum();
    (px > EPIX_LINE_MAX_PIXELS_PER_OUTPUT)
        .then(|| format!("EPIX limit: this output would drive {px} pixels — max is {EPIX_LINE_MAX_PIXELS_PER_OUTPUT}, and fewer in high-power mode"))
}

type Rule = (&'static str, Protocol, fn(&Ctx) -> Option<String>);

/// ChamSys's `CONNECTION_RULES`, in its order.
const RULES: [Rule; 6] = [
    ("scn-bus-max10", Protocol::Scene10, scn_bus_max10),
    ("logic-mr16-poe-x", Protocol::Network, mr16_poe_x),
    ("logic-mr16-max8", Protocol::Network, mr16_max8),
    ("logic-fixture-net-thru", Protocol::NetData, fixture_net_thru),
    ("epix-flex-max-per-output", Protocol::Epix3p, epix_flex_max),
    ("epix-line-max-per-output", Protocol::Epix4p, epix_line_max),
];

/// The first rule the proposed link breaks; the message ends with the rule id.
pub(crate) fn check_rules(
    devices: &[(Handle, &Product)],
    links: &[WireLink],
    proposed: &WireLink,
    from: &Product,
    to: &Product,
) -> Result<(), Refusal> {
    let mut products = HashMap::new();
    for (h, p) in devices {
        products.entry(*h).or_insert(*p);
    }
    let mut adj: HashMap<(Protocol, Handle), Vec<Handle>> = HashMap::new();
    for l in links {
        adj.entry((l.protocol, l.from)).or_default().push(l.to);
        if l.to != l.from {
            adj.entry((l.protocol, l.to)).or_default().push(l.from);
        }
    }
    let c = Ctx { products, links, adj, from: (proposed.from, from), to: (proposed.to, to) };
    for (id, protocol, check) in RULES {
        if protocol != proposed.protocol {
            continue;
        }
        if let Some(msg) = check(&c) {
            return Err(Refusal { code: "rule-violation", rule_id: Some(id), message: format!("{msg} [{id}]") });
        }
    }
    Ok(())
}
