//! PoE wattage budget from ChamSys Designer's `poe.ts`. `poe_watts` is what a fixture draws;
//! `poe_port_watts` is what a supply carries per port. Cables are walked in both directions: the
//! drawing is a schematic, so `from`/`to` says nothing about power flow.

use crate::{Product, Protocol, WireLink};
use cadcraft_doc::Handle;
use std::collections::{HashMap, HashSet, VecDeque};

/// The most any one run may draw, whatever the supply (ChamSys issue #155). A calibration knob.
pub const MAX_RUN_WATTS: f64 = 110.0;

/// What the budget says about a proposed cable: over [`MAX_RUN_WATTS`] it is refused, otherwise a warning.
pub struct PoeVerdict {
    pub refuse: bool,
    pub message: String,
}

struct Net<'a> {
    products: HashMap<Handle, &'a Product>,
    links: &'a [WireLink],
    /// Network-cable neighbours of each device, built once.
    adj: HashMap<Handle, Vec<Handle>>,
}

fn is_driver(p: &Product) -> bool {
    p.poe_port_watts > 0.0
}
fn is_load(p: &Product) -> bool {
    p.poe_watts > 0.0
}

impl<'a> Net<'a> {
    fn new(devices: &'a [(Handle, &'a Product)], links: &'a [WireLink]) -> Self {
        let mut products = HashMap::new();
        for (h, p) in devices {
            products.entry(*h).or_insert(*p);
        }
        let mut adj: HashMap<Handle, Vec<Handle>> = HashMap::new();
        for l in links.iter().filter(|l| l.protocol == Protocol::Network) {
            adj.entry(l.from).or_default().push(l.to);
            if l.to != l.from {
                adj.entry(l.to).or_default().push(l.from);
            }
        }
        Net { products, links, adj }
    }

    fn product(&self, h: Handle) -> Option<&Product> {
        self.products.get(&h).copied()
    }

    fn neighbours(&self, h: Handle) -> impl Iterator<Item = Handle> + '_ {
        self.adj.get(&h).into_iter().flatten().copied()
    }

    /// Watts drawn by everything reachable from `start`, never walking back through `exclude`.
    fn chain_watts(&self, start: Handle, exclude: Option<Handle>) -> f64 {
        let mut seen: HashSet<Handle> = exclude.into_iter().collect();
        let mut queue = VecDeque::from([start]);
        let mut total = 0.0;
        while let Some(h) = queue.pop_front() {
            if !seen.insert(h) {
                continue;
            }
            let Some(p) = self.product(h) else { continue };
            total += p.poe_watts;
            queue.extend(self.neighbours(h).filter(|n| !seen.contains(n)));
        }
        total
    }

    /// The nearest supply feeding `device`'s run: its code, the watts already on that port, the port ceiling.
    fn chain_root(&self, device: Handle) -> Option<(&str, f64, f64)> {
        let mut seen = HashSet::from([device]);
        let mut queue = VecDeque::from([device]);
        while let Some(cur) = queue.pop_front() {
            for next in self.neighbours(cur) {
                if !seen.insert(next) {
                    continue;
                }
                let Some(p) = self.product(next) else { continue };
                if is_driver(p) {
                    return Some((&p.code, self.chain_watts(cur, Some(next)), p.poe_port_watts));
                }
                queue.push_back(next);
            }
        }
        None
    }

    /// Watts on each of `driver`'s ports. A cable drawn into the driver counts only from a fixture.
    fn driver_ports(&self, driver: Handle) -> Vec<f64> {
        self.links
            .iter()
            .filter(|l| l.protocol == Protocol::Network)
            .filter_map(|l| {
                let far = if l.from == driver {
                    l.to
                } else if l.to == driver && self.product(l.from).is_some_and(|p| is_load(p) && !is_driver(p)) {
                    l.from
                } else {
                    return None;
                };
                Some(self.chain_watts(far, Some(driver)))
            })
            .collect()
    }
}

fn over_cap(watts: f64) -> Option<PoeVerdict> {
    (watts > MAX_RUN_WATTS).then(|| PoeVerdict {
        refuse: true,
        message: format!("That would put {watts:.1}W on one PoE run — past the {MAX_RUN_WATTS}W ceiling a single run can carry. Split it across more than one feed."),
    })
}

fn warn(message: String) -> Option<PoeVerdict> {
    Some(PoeVerdict { refuse: false, message })
}

/// The budget's verdict on adding a PoE cable `from`–`to`. `ceiling` is the biggest per-port supply
/// in the library, for runs with no supply on them; 0 skips that warning.
pub fn poe_verdict(devices: &[(Handle, &Product)], links: &[WireLink], from: Handle, to: Handle, ceiling: f64) -> Option<PoeVerdict> {
    let net = Net::new(devices, links);
    let (fp, tp) = (net.product(from)?, net.product(to)?);
    for (driver, dp, load_end) in [(from, fp, to), (to, tp, from)] {
        if is_driver(dp) {
            let load = net.chain_watts(load_end, Some(driver));
            if let Some(v) = over_cap(load) {
                return Some(v);
            }
            if load > dp.poe_port_watts {
                return warn(format!(
                    "This port on {} would carry {load:.1}W (max {}W/port). Use a larger supply or split the run.",
                    dp.code, dp.poe_port_watts
                ));
            }
        }
    }
    if is_load(fp)
        && let Some((code, on_port, max)) = net.chain_root(from)
        && max > 0.0
    {
        let total = on_port + net.chain_watts(to, Some(from));
        if let Some(v) = over_cap(total) {
            return Some(v);
        }
        if total > max {
            return warn(format!("That would draw {total:.1}W on one {code} port (max {max}W). Use a larger supply or split the run."));
        }
    }
    if !is_driver(fp) && !is_driver(tp) && (is_load(fp) || is_load(tp)) && net.chain_root(from).or_else(|| net.chain_root(to)).is_none() {
        let total = net.chain_watts(from, Some(to)) + net.chain_watts(to, Some(from));
        if let Some(v) = over_cap(total) {
            return Some(v);
        }
        if ceiling > 0.0 && total > ceiling {
            return warn(format!(
                "This run draws {total:.1}W with no supply on it — more than the {ceiling:.0}W a single supply port carries. It will need more than one feed, or a split."
            ));
        }
    }
    None
}

/// The biggest per-port supply in `library`.
pub fn max_supply_watts(library: &[Product]) -> f64 {
    library.iter().map(|p| p.poe_port_watts).fold(0.0, f64::max)
}

/// A schedule warning when any port of the supply `driver` carries more than its ceiling.
pub fn port_overload(devices: &[(Handle, &Product)], links: &[WireLink], driver: Handle) -> Option<String> {
    let net = Net::new(devices, links);
    let p = net.product(driver).filter(|p| is_driver(p))?;
    let worst = net.driver_ports(driver).into_iter().fold(0.0, f64::max);
    (worst > p.poe_port_watts).then(|| format!("A port on {} carries {worst:.1}W (max {}W/port)", p.code, p.poe_port_watts))
}
