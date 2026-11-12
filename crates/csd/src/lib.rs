//! Lighting-control system designer domain: products, device blocks, wire links and the
//! connection rules. Devices are block references; wires are polylines carrying a [`WireLink`]
//! in xdata.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use cadcraft_doc::{Handle, RawTag};
use cadcraft_geom::Vec2;

mod card;
mod library;

pub use card::*;
pub use library::*;

/// The xdata application name of wires.
const XDATA_APP: &str = "CSD";

/// The ten signal protocols. The ids are the on-disk strings and must never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    Network,
    NetData,
    Scene10,
    Dmx,
    V10,
    Contact,
    Power,
    Midi,
    Epix3p,
    Epix4p,
}

impl Protocol {
    pub const ALL: [Protocol; 10] = [
        Protocol::Network,
        Protocol::NetData,
        Protocol::Scene10,
        Protocol::Dmx,
        Protocol::V10,
        Protocol::Contact,
        Protocol::Power,
        Protocol::Midi,
        Protocol::Epix3p,
        Protocol::Epix4p,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Protocol::Network => "network",
            Protocol::NetData => "netdata",
            Protocol::Scene10 => "10scene",
            Protocol::Dmx => "dmx",
            Protocol::V10 => "0-10v",
            Protocol::Contact => "contact",
            Protocol::Power => "power",
            Protocol::Midi => "midi",
            Protocol::Epix3p => "epix3p",
            Protocol::Epix4p => "epix4p",
        }
    }

    pub fn from_id(id: &str) -> Option<Protocol> {
        Protocol::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Short label used in refusal messages.
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Network => "PoE",
            Protocol::NetData => "Net",
            Protocol::Scene10 => "10S",
            Protocol::Dmx => "DMX",
            Protocol::V10 => "0-10V",
            Protocol::Contact => "Cont",
            Protocol::Power => "PWR",
            Protocol::Midi => "MIDI",
            Protocol::Epix3p => "EPIX 3P",
            Protocol::Epix4p => "EPIX 4P",
        }
    }

    /// Cable colour as 0xRRGGBB.
    pub fn color(self) -> u32 {
        match self {
            Protocol::Network => 0xed3162,
            Protocol::NetData => 0x3377cc,
            Protocol::Scene10 => 0x6dbd45,
            Protocol::Dmx => 0x77cee5,
            Protocol::V10 => 0x8833cc,
            Protocol::Contact => 0xe8c830,
            Protocol::Power => 0xcc0000,
            Protocol::Midi => 0xdd6600,
            Protocol::Epix3p => 0x0f9b8e,
            Protocol::Epix4p => 0xb5179e,
        }
    }

    /// Drawn dashed: a shared bus rather than a point-to-point run.
    pub fn dashed(self) -> bool {
        matches!(self, Protocol::Scene10 | Protocol::Contact)
    }

    fn index(self) -> usize {
        Protocol::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// A side of the device card, with the on-disk ids `top`, `bot`, `lft`, `rgt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    pub fn id(self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Bottom => "bot",
            Side::Left => "lft",
            Side::Right => "rgt",
        }
    }

    pub fn from_id(id: &str) -> Option<Side> {
        [Side::Top, Side::Bottom, Side::Left, Side::Right].into_iter().find(|s| s.id() == id)
    }

    fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// Outward unit direction in the card's own frame.
    pub(crate) fn dir(self) -> Vec2 {
        match self {
            Side::Top => Vec2 { x: 0.0, y: 1.0 },
            Side::Bottom => Vec2 { x: 0.0, y: -1.0 },
            Side::Left => Vec2 { x: -1.0, y: 0.0 },
            Side::Right => Vec2 { x: 1.0, y: 0.0 },
        }
    }
}

/// A library product. `counts` holds (in, out) port counts per protocol, indexed by
/// [`Protocol::ALL`] order; use [`Product::ports`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub code: String,
    pub cat: String,
    pub brand: String,
    pub notes: String,
    /// DMX personalities as the library writes them, e.g. `17ch:17, 34ch:34`.
    pub modes: String,
    pub channels: u32,
    pub universes: u32,
    pub counts: [(u32, u32); 10],
    pub poe_port_watts: f64,
    pub poe_watts: f64,
    /// Brand colour as 0xRRGGBB.
    pub color: u32,
    /// Not in the library any more: drawn with a dashed border, as the web app does.
    pub missing: bool,
    /// Record keys not modelled above (`img`, `deviceNotes`, `iconW`...), kept so a save loses nothing.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Product {
    /// (in, out) port counts for `p`.
    pub fn ports(&self, p: Protocol) -> (u32, u32) {
        self.counts.get(p.index()).copied().unwrap_or((0, 0))
    }

    /// In plus out: direction is not enforced, a device has one budget per protocol.
    /// Power with no declared outlets is unmetered (`u64::MAX`).
    fn capacity(&self, p: Protocol) -> u64 {
        let (i, o) = self.ports(p);
        let n = u64::from(i) + u64::from(o);
        // ponytail: the TS also zeroes this for accessories (by category); Product has no category yet.
        if p == Protocol::Power && n == 0 { u64::MAX } else { n }
    }

    /// A PoE run counts against PoE ports alone; a data cable against every RJ45.
    fn effective_capacity(&self, p: Protocol) -> u64 {
        if p == Protocol::NetData { self.capacity(Protocol::Network).saturating_add(self.capacity(Protocol::NetData)) } else { self.capacity(p) }
    }
}

fn used(links: &[WireLink], dev: Handle, p: Protocol) -> usize {
    let pooled = p == Protocol::NetData;
    links.iter().filter(|l| (l.from == dev || l.to == dev) && (l.protocol == p || (pooled && l.protocol == Protocol::Network))).count()
}

/// A wire's two ends, stored in the polyline's xdata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireLink {
    pub from: Handle,
    pub from_side: Side,
    pub to: Handle,
    pub to_side: Side,
    pub protocol: Protocol,
}

impl WireLink {
    pub fn to_xdata(&self) -> Vec<RawTag> {
        let tag = |code, value: &str| RawTag { code, value: value.into() };
        vec![
            tag(1001, XDATA_APP),
            tag(1000, &self.from.hex()),
            tag(1000, self.from_side.id()),
            tag(1000, &self.to.hex()),
            tag(1000, self.to_side.id()),
            tag(1000, self.protocol.id()),
        ]
    }

    /// Finds the `CSD` application among other applications' groups; `None` if absent or malformed.
    pub fn from_xdata(tags: &[RawTag]) -> Option<WireLink> {
        let start = tags.iter().position(|t| t.code == 1001 && t.value == XDATA_APP)?;
        let mut v = tags.get(start + 1..)?.iter().take_while(|t| t.code != 1001).filter(|t| t.code == 1000).map(|t| t.value.as_str());
        Some(WireLink {
            from: Handle::parse_hex(v.next()?)?,
            from_side: Side::from_id(v.next()?)?,
            to: Handle::parse_hex(v.next()?)?,
            to_side: Side::from_id(v.next()?)?,
            protocol: Protocol::from_id(v.next()?)?,
        })
    }
}

/// The pieces drawn on top of a wire that do not belong to its polyline: a dot where each end
/// meets a card, and the protocol's pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Deco {
    Start,
    End,
    Pill,
}

impl Deco {
    fn id(self) -> &'static str {
        match self {
            Deco::Start => "start",
            Deco::End => "end",
            Deco::Pill => "pill",
        }
    }
}

/// Own application name, so a decoration never reads as a wire (`WireLink` looks for `CSD`).
const DECO_APP: &str = "CSDX";

/// Xdata marking an entity as `deco` of `wire`.
pub fn deco_xdata(wire: Handle, deco: Deco) -> Vec<RawTag> {
    let tag = |code, value: &str| RawTag { code, value: value.into() };
    vec![tag(1001, DECO_APP), tag(1000, &wire.hex()), tag(1000, deco.id())]
}

/// The wire and role a decoration's xdata names; `None` if absent or malformed.
pub fn deco_from_xdata(tags: &[RawTag]) -> Option<(Handle, Deco)> {
    let start = tags.iter().position(|t| t.code == 1001 && t.value == DECO_APP)?;
    let mut v = tags.get(start + 1..)?.iter().take_while(|t| t.code != 1001).filter(|t| t.code == 1000).map(|t| t.value.as_str());
    let wire = Handle::parse_hex(v.next()?)?;
    let role = v.next()?;
    let deco = [Deco::Start, Deco::End, Deco::Pill].into_iter().find(|d| d.id() == role)?;
    Some((wire, deco))
}

/// Why a connection was refused: a stable `code` and the user-facing `message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

fn refuse(code: &'static str, message: String) -> Result<(), Refusal> {
    Err(Refusal { code, message })
}

/// Self-connection, unknown device, port existence and capacity (including the asymmetric RJ45
/// pool). ponytail: no PoE wattage budget and no hardware rules; port those from connect.ts when needed.
pub fn can_connect(devices: &[(Handle, &Product)], links: &[WireLink], proposed: &WireLink) -> Result<(), Refusal> {
    let (a, b, p) = (proposed.from, proposed.to, proposed.protocol);
    if a == b {
        return refuse("self-connection", "A device cannot connect to itself".into());
    }
    let find = |h: Handle| devices.iter().find(|(d, _)| *d == h).map(|(_, prod)| *prod);
    let (Some(from), Some(to)) = (find(a), find(b)) else {
        return refuse("unknown-device", "One of these devices is no longer on the drawing".into());
    };
    let label = p.label();
    for prod in [from, to] {
        if prod.effective_capacity(p) == 0 {
            return refuse("no-ports-of-type", format!("{} has no {label} ports — check the active wire type", prod.code));
        }
    }
    for (h, prod) in [(a, from), (b, to)] {
        let cap = prod.effective_capacity(p);
        let n = used(links, h, p);
        if u64::try_from(n).is_ok_and(|n| n >= cap) {
            let message = if p == Protocol::NetData {
                format!("{}: all Ethernet ports in use ({cap} total)", prod.code)
            } else {
                format!("{}: all {label} connections used (max {cap})", prod.code)
            };
            return refuse("ports-exhausted", message);
        }
    }
    Ok(())
}

/// Five products from the ChamSys library, for tests and the first run.
pub fn sample_products() -> Vec<Product> {
    let p = |id: &str, name: &str, code: &str, ports: &[(Protocol, u32, u32)], poe_port_watts: f64, poe_watts: f64, color: u32| {
        let mut counts = [(0, 0); 10];
        for (proto, i, o) in ports {
            if let Some(c) = counts.get_mut(proto.index()) {
                *c = (*i, *o);
            }
        }
        Product {
            id: id.into(),
            name: name.into(),
            code: code.into(),
            cat: "Sample".into(),
            counts,
            poe_port_watts,
            poe_watts,
            color,
            ..Product::default()
        }
    };
    vec![
        p("CHAMMQ50", "MagicQ MagicQ50 Compact Console", "MQ50", &[(Protocol::Dmx, 0, 4), (Protocol::NetData, 0, 2)], 0.0, 0.0, 0x1565c0),
        p("logic-poex", "Logic POE-X Converter", "POE-X", &[(Protocol::Network, 0, 3)], 75.0, 0.0, 0x0277bd),
        p("epix-flex-drive", "EPIX Flex Drive", "EPIX FLEX DRIVE", &[(Protocol::Dmx, 1, 1), (Protocol::NetData, 1, 1)], 0.0, 0.0, 0x65edfa),
        p("logic-ar111m", "Logic AR111 Downlight M RGBW (45W)", "AR111-DNM", &[(Protocol::Network, 0, 2)], 0.0, 45.0, 0x0277bd),
        p("b565fc", "Ovation B-565FC", "B-565FC", &[(Protocol::Dmx, 1, 0)], 0.0, 0.0, 0xf7ae41),
        // The web app's stand-in for a product a file names that the library lacks.
        Product { missing: true, ..p("OVCYC1FCV2", "OVCYC1FCV2 (not in library)", "OVCYC1FCV2", &[], 0.0, 0.0, 0x888888) },
    ]
}

#[cfg(test)]
mod tests;
