//! The device card: its symbol, where its ports are and how wires leave them. Sizes, colours and
//! fonts' relative sizes follow the web app's DeviceCard and Wires (dark theme).

use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{
    Attrib, AttribModes, AttribProps, Block, Entity, EntityKind, HAlign, Handle, Hatch, HatchLoop, LwPolyline, Text, TextStyle, VAlign, Wipeout,
};
use cadcraft_geom::{PolyVertex, Vec2};

use crate::{Deco, Product, Protocol, Side};

/// Card size in drawing units; the insertion point is the card centre. The body is the picture
/// area (55 high) over the 18 high label strip.
pub const CARD_W: f64 = 70.0;
pub const CARD_H: f64 = 73.0;
const STRIP_H: f64 = 18.0;
const BORDER: f64 = 1.5;
const RADIUS: f64 = 5.0;
const PAPER: u32 = 0xf5f2ec;
const INK: u32 = 0x1a1a1a;
const STUB: f64 = 24.0;
/// Extra stub length per lane, so wires sharing a side do not turn at the same distance.
const LANE: f64 = 6.0;
/// Radius of a wire's end dot, and the wire's width.
pub const TERM_R: f64 = 4.5;
pub const WIRE_W: f64 = 2.5;
const PILL_H: f64 = 14.0;
const CODE_H: f64 = 7.0;
const LABEL_H: f64 = 6.0;
/// Characters of the code on one line of the body, and lines kept.
const CODE_CHARS: usize = 9;
const CODE_LINES: usize = 3;
/// Width the strip text may use, in the estimate of `truncate_label`.
const LABEL_W: f64 = 60.0;
/// The text style of every card and badge text. The drawing needs it (see [`bold_style`]).
pub const BOLD_STYLE: &str = "CSDBOLD";
/// Darkest luma, 0 to 1, a text colour may have on the paper; see [`legible`].
const MAX_TEXT_LUMA: f64 = 0.45;

/// A wire's place on one side of a card: `index` of `total`, counted from the top or the left,
/// and its `lane`, how many wires turn between it and the edge's own end of the fan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub index: usize,
    pub total: usize,
    pub lane: usize,
}

impl Slot {
    pub const SOLO: Slot = Slot { index: 0, total: 1, lane: 0 };

    /// Position along the edge from -0.5 to 0.5, evenly spread with a gap at both ends.
    fn offset(self) -> f64 {
        let total = self.total.max(1);
        (self.index.min(total - 1) + 1) as f64 / (total + 1) as f64 - 0.5
    }

    fn stub(self) -> f64 {
        STUB + self.lane as f64 * LANE
    }
}

fn scale_sign(v: f64) -> f64 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// World position where slot `slot` of `side` meets a card inserted at `at`.
/// ponytail: slots are spread evenly along the edge; no manual port order, and no lane
/// assignment for legs that run collinear (the TS wireLanes).
pub fn port_point(at: Vec2, rotation: f64, scale: Vec2, side: Side, slot: Slot) -> Vec2 {
    let d = side.dir();
    let t = slot.offset();
    // Slots run left to right along a top or bottom edge and top to bottom along a side edge.
    let along = if d.x == 0.0 { Vec2 { x: t * CARD_W, y: 0.0 } } else { Vec2 { x: 0.0, y: -t * CARD_H } };
    let local = Vec2 { x: d.x * CARD_W / 2.0 + along.x, y: d.y * CARD_H / 2.0 + along.y };
    at + Vec2 { x: local.x * scale.x, y: local.y * scale.y }.rotate(rotation)
}

/// An edge of a placed card in world coordinates: its middle, the direction slots run along it,
/// the direction out of the card, and half its length.
pub struct EdgeFrame {
    anchor: Vec2,
    along: Vec2,
    out: Vec2,
    half: f64,
}

pub fn edge_frame(at: Vec2, rotation: f64, scale: Vec2, side: Side) -> EdgeFrame {
    let d = side.dir();
    let signed = |v: Vec2| Vec2 { x: v.x * scale_sign(scale.x), y: v.y * scale_sign(scale.y) }.rotate(rotation);
    let (along, half) =
        if d.x == 0.0 { (Vec2 { x: 1.0, y: 0.0 }, CARD_W / 2.0 * scale.x.abs()) } else { (Vec2 { x: 0.0, y: -1.0 }, CARD_H / 2.0 * scale.y.abs()) };
    EdgeFrame { anchor: port_point(at, rotation, scale, side, Slot::SOLO), along: signed(along), out: signed(d), half }
}

/// The wires on one edge in slot order, from where each one's far end is (the TS `portFan`):
/// wires heading one way take the slots on that half, and those turning furthest out take the
/// outermost ones, so they do not cross. `others` pairs a wire key with its far device's centre;
/// ties fall to the key. Each wire comes back with its slot; the lane counts inward from the
/// middle of the fan, so the wires that turn furthest out also leave the furthest.
pub fn fan_order<K: Ord + Copy>(frame: &EdgeFrame, others: &[(K, Vec2)]) -> Vec<(K, Slot)> {
    let key = |other: Vec2| {
        let d = other - frame.anchor;
        let (o, a) = (d.dot(frame.out), d.dot(frame.along));
        if a.abs() < frame.half {
            return [0.0, a, o];
        }
        let sign = if a > 0.0 { 1.0 } else { -1.0 };
        [sign, -sign * o.max(0.0), if o < 0.0 { -a } else { a }]
    };
    let mut keyed: Vec<([f64; 3], K)> = others.iter().map(|(k, p)| (key(*p), *k)).collect();
    keyed.sort_by(|(a, ka), (b, kb)| a.iter().zip(b).map(|(x, y)| x.total_cmp(y)).find(|o| o.is_ne()).unwrap_or(ka.cmp(kb)));
    let total = keyed.len();
    keyed
        .into_iter()
        .enumerate()
        .map(|(index, (k, id))| {
            let lane = if k[0] < 0.0 {
                index
            } else if k[0] > 0.0 {
                total - 1 - index
            } else {
                0
            };
            (id, Slot { index, total, lane })
        })
        .collect()
}

/// The direction `side` of a card faces in the world: mirrored by negative scale, then turned by
/// the rotation rounded to a quarter turn.
/// ponytail: non-right-angle rotations route as the nearest quarter turn, so stubs leave slightly off-perpendicular.
pub fn world_side(side: Side, rotation: f64, scale: Vec2) -> Side {
    const CCW: [Side; 4] = [Side::Right, Side::Top, Side::Left, Side::Bottom];
    let side = match side {
        Side::Left | Side::Right if scale.x < 0.0 => side.opposite(),
        Side::Top | Side::Bottom if scale.y < 0.0 => side.opposite(),
        _ => side,
    };
    let quarters = (rotation / std::f64::consts::FRAC_PI_2).round() as i64;
    let start = CCW.iter().position(|s| *s == side).unwrap_or(0) as i64;
    CCW.get((start + quarters).rem_euclid(4) as usize).copied().unwrap_or(side)
}

/// Orthogonal route from one port to another: a stub out of each (longer for outer lanes), then
/// a turn at the source's stub end across to the target's.
/// ponytail: naive ortho route, no obstacle avoidance; port the real wireRoute when overlaps matter.
pub fn route(from: Vec2, from_side: Side, from_slot: Slot, to: Vec2, to_side: Side, to_slot: Slot) -> Vec<Vec2> {
    let (d1, d2) = (from_side.dir(), to_side.dir());
    let a = from + d1 * from_slot.stub();
    let b = to + d2 * to_slot.stub();
    let corner = if d1.x == 0.0 { Vec2 { x: b.x, y: a.y } } else { Vec2 { x: a.x, y: b.y } };
    let mut pts = vec![from, a, corner, b, to];
    pts.dedup_by(|p, q| p.near(*q, 1e-9));
    // Facing stubs that overlap would run past a turn and double back; cut that spur.
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    for p in pts {
        while let [.., x, y] = out[..] {
            let (u, v) = (y - x, p - y);
            if (u.x * v.y - u.y * v.x).abs() < 1e-9 && u.dot(v) < 0.0 {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    out
}

/// Block name for a product: `CSD_<id>` with anything unsafe in a block name replaced by `_`.
pub fn block_name(product_id: &str) -> String {
    let safe: String = product_id.chars().take(200).map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("CSD_{safe}")
}

/// The code as the body shows it: wrapped at words, or inside a long word, to `CODE_CHARS` a line.
pub fn wrap_code(code: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in code.split_whitespace() {
        for chunk in word.chars().collect::<Vec<_>>().chunks(CODE_CHARS) {
            let chunk: String = chunk.iter().collect();
            match lines.last_mut() {
                Some(l) if l.chars().count() + 1 + chunk.chars().count() <= CODE_CHARS => {
                    l.push(' ');
                    l.push_str(&chunk);
                }
                _ => lines.push(chunk),
            }
        }
    }
    lines.truncate(CODE_LINES);
    lines
}

/// `label` cut with "..." to fit the strip.
/// ponytail: widths are per character class, in the web app's 9px bold type, not measured.
pub fn truncate_label(label: &str) -> String {
    let width = |c: char| {
        if c.is_lowercase() {
            5.2
        } else if c.is_whitespace() {
            3.0
        } else {
            6.6
        }
    };
    if label.chars().map(width).sum::<f64>() <= LABEL_W {
        return label.into();
    }
    let (mut used, mut out) = (8.0, String::new());
    for c in label.chars() {
        used += width(c);
        if used > LABEL_W {
            break;
        }
        out.push(c);
    }
    out + "..."
}

fn mix(a: u32, b: u32, t: f64) -> u32 {
    let (Rgb(ar, ag, ab), Rgb(br, bg, bb)) = (Rgb::from_u32(a), Rgb::from_u32(b));
    let ch = |x: u8, y: u8| (f64::from(x) * t + f64::from(y) * (1.0 - t)).round().clamp(0.0, 255.0) as u32;
    ch(ar, br) << 16 | ch(ag, bg) << 8 | ch(ab, bb)
}

/// The bold face the web app sets its text in. Installed TrueType fonts are found by name at run
/// time (never bundled); where this one is missing the text falls back to the thin stroke font.
/// ponytail: only "Arial Bold" is named; add a family list when a platform lacks it.
pub fn bold_style() -> TextStyle {
    TextStyle { name: BOLD_STYLE.into(), font: "Arial Bold".into(), ..TextStyle::default() }
}

/// `rgb` darkened, keeping its hue, until its luma is at most [`MAX_TEXT_LUMA`]: brand colours
/// like cyan and orange are unreadable as text on the paper. Dark colours are unchanged.
/// ponytail: sRGB luma, not a contrast ratio against the paper.
pub fn legible(rgb: u32) -> u32 {
    let Rgb(r, g, b) = Rgb::from_u32(rgb);
    let luma = (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0;
    if luma <= MAX_TEXT_LUMA {
        return rgb;
    }
    // INK is grey, so its luma is its channel.
    let ink = f64::from(INK & 0xff) / 255.0;
    mix(INK, rgb, (luma - MAX_TEXT_LUMA) / (luma - ink))
}

/// A mask that hides what was drawn before it, so the fill after it sits over a wire (the GPU
/// canvas draws every wide line after every fill; only a mask starts a new draw-order group).
/// `shape` must lie inside that fill, as the mask is background-coloured.
fn mask(boundary: Vec<Vec2>) -> EntityKind {
    EntityKind::Wipeout(Wipeout { boundary })
}

fn vertex(x: f64, y: f64, bulge: f64) -> PolyVertex {
    PolyVertex { p: Vec2 { x, y }, bulge, start_width: 0.0, end_width: 0.0 }
}

/// Closed rectangle with rounded corners of radii (bottom left, bottom right, top right, top left).
fn round_rect(x0: f64, y0: f64, x1: f64, y1: f64, [bl, br, tr, tl]: [f64; 4]) -> Vec<PolyVertex> {
    // A quarter circle is a bulge of tan(90 degrees / 4).
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    let corners = [
        ((x1, y0), br, (1.0, 0.0), (0.0, 1.0)),
        ((x1, y1), tr, (0.0, 1.0), (-1.0, 0.0)),
        ((x0, y1), tl, (-1.0, 0.0), (0.0, -1.0)),
        ((x0, y0), bl, (0.0, -1.0), (1.0, 0.0)),
    ];
    let mut v = Vec::new();
    for ((x, y), r, i, o) in corners {
        if r > 0.0 {
            v.push(vertex(x - i.0 * r, y - i.1 * r, bulge));
            v.push(vertex(x + o.0 * r, y + o.1 * r, 0.0));
        } else {
            v.push(vertex(x, y, 0.0));
        }
    }
    v
}

fn circle(c: Vec2, r: f64) -> Vec<PolyVertex> {
    vec![vertex(c.x - r, c.y, 1.0), vertex(c.x + r, c.y, 1.0)]
}

fn fill(vertices: Vec<PolyVertex>) -> EntityKind {
    EntityKind::Hatch(Hatch {
        pattern: "SOLID".into(),
        solid: true,
        loops: vec![HatchLoop { vertices, outer: true, sources: Vec::new() }],
        scale: 1.0,
        angle: 0.0,
        associative: false,
        style: 0,
        elevation: 0.0,
        gradient: None,
        origin: Vec2::ZERO,
        background: None,
        pattern_lines: Vec::new(),
    })
}

fn outline(vertices: Vec<PolyVertex>, width: f64) -> EntityKind {
    EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: width, elevation: 0.0, plinegen: false })
}

fn text(at: Vec2, height: f64, value: &str) -> Text {
    Text {
        insert: at.to3(0.0),
        align_pt: Some(at.to3(0.0)),
        height,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: BOLD_STYLE.into(),
        halign: HAlign::Center,
        valign: VAlign::Middle,
    }
}

/// Fills a block with entities in draw order, each in a true colour.
struct Painter<'a, F: FnMut() -> Handle> {
    block: Block,
    new_handle: &'a mut F,
}

impl<F: FnMut() -> Handle> Painter<'_, F> {
    fn entity(&mut self, kind: EntityKind, color: u32) -> Entity {
        let mut e = Entity::new((self.new_handle)(), kind);
        e.common.color = Color::True(Rgb::from_u32(color));
        e
    }

    fn add(&mut self, kind: EntityKind, color: u32) {
        let e = self.entity(kind, color);
        self.block.entities.push(e);
    }
}

/// The device symbol: paper with a rounded border in the brand colour, the product code on the
/// body (where a picture goes later), and a bordered label strip.
/// The hidden `PRODUCT` and `NAME` and the visible `LABEL` attribute definitions carry the
/// product id, the label and the label as the strip shows it. A product missing from the library
/// gets a dashed border, drawn with linetype scale `dash_scale`.
/// `new_handle` allocates drawing-unique handles for the block's entities (`Drawing::new_handle`).
pub fn device_block(product: &Product, mut new_handle: impl FnMut() -> Handle, dash_scale: f64) -> Block {
    let mut p = Painter { block: Block::new(&block_name(&product.id)), new_handle: &mut new_handle };
    let (hw, hh) = (CARD_W / 2.0 - BORDER / 2.0, CARD_H / 2.0 - BORDER / 2.0);
    let card = round_rect(-hw, -hh, hw, hh, [RADIUS - BORDER / 2.0; 4]);
    p.add(fill(card.clone()), PAPER);
    let mut border = p.entity(outline(card, BORDER), product.color);
    if product.missing {
        border.common.linetype = "DASHED2".into();
        border.common.ltscale = dash_scale;
    }
    p.block.entities.push(border);
    // The strip sits inside the card's border and has a border of its own: its top edge is the rule.
    let (sx, strip_top) = (hw - BORDER, -hh + STRIP_H);
    let r = RADIUS - BORDER - BORDER / 2.0 - 0.75;
    p.add(outline(round_rect(-sx, -hh, sx, strip_top, [r, r, 0.0, 0.0]), BORDER), product.color);
    // The outline's mitred top corners leave a notch against the card's border; the rule fills it.
    let (rx, ry) = (sx + BORDER / 2.0, BORDER / 2.0);
    p.add(fill(round_rect(-rx, strip_top - ry, rx, strip_top + ry, [0.0; 4])), product.color);

    let lines = wrap_code(&product.code);
    let body_mid = (hh + strip_top) / 2.0;
    let first = body_mid + (lines.len() as f64 - 1.0) / 2.0 * (CODE_H + 3.0);
    for (i, line) in lines.iter().enumerate() {
        p.add(EntityKind::Text(text(Vec2 { x: 0.0, y: first - i as f64 * (CODE_H + 3.0) }, CODE_H, line)), INK);
    }
    let attdef = |tag: &str, prompt: &str, at: Vec2, value: &str, invisible: bool| {
        EntityKind::AttDef(Attrib {
            tag: tag.into(),
            text: text(at, LABEL_H, value),
            invisible,
            constant: false,
            prompt: prompt.into(),
            props: AttribProps::default(),
            multiline: None,
            modes: AttribModes::default(),
        })
    };
    p.add(attdef("PRODUCT", "Product", Vec2::ZERO, &product.id, true), INK);
    p.add(attdef("NAME", "Name", Vec2::ZERO, "", true), INK);
    p.add(attdef("LABEL", "Label", Vec2 { x: 0.0, y: (-hh + strip_top) / 2.0 }, "", false), legible(product.color));
    p.block
}

/// Name of the block a wire decoration is an insert of: a dot per protocol, or its pill.
pub fn deco_block_name(protocol: Protocol, deco: Deco) -> String {
    format!("{}_{}", if deco == Deco::Pill { "CSDPILL" } else { "CSDDOT" }, protocol.id())
}

/// The block for a wire decoration, centred on its origin: the end dot, a disc in the protocol
/// colour with a paper ring, or the pill, the protocol's short label on a paper badge.
pub fn deco_block(protocol: Protocol, deco: Deco, mut new_handle: impl FnMut() -> Handle) -> Block {
    let mut p = Painter { block: Block::new(&deco_block_name(protocol, deco)), new_handle: &mut new_handle };
    let color = protocol.color();
    if deco == Deco::Pill {
        let label = protocol.label();
        let (hw, hh) = ((label.chars().count() as f64 * 5.4 + 10.0).max(26.0) / 2.0, PILL_H / 2.0);
        let shape = round_rect(-hw, -hh, hw, hh, [3.0; 4]);
        // Inside the rounded corners, which a rectangle's own corners would leave.
        let (mx, my) = (hw - 1.0, hh - 1.0);
        p.add(mask([(-mx, -my), (mx, -my), (mx, my), (-mx, my)].map(|(x, y)| Vec2 { x, y }).to_vec()), color);
        p.add(fill(shape.clone()), PAPER);
        p.add(outline(shape, 1.0), mix(color, PAPER, 0.45));
        p.add(EntityKind::Text(text(Vec2::ZERO, LABEL_H, label)), legible(color));
    } else {
        // A 12-gon inscribed in the disc.
        let ring = (0..12).map(|i| Vec2::from_angle(f64::from(i) * std::f64::consts::TAU / 12.0) * (TERM_R - 0.5)).collect();
        p.add(mask(ring), color);
        p.add(fill(circle(Vec2::ZERO, TERM_R)), color);
        p.add(outline(circle(Vec2::ZERO, TERM_R), BORDER), PAPER);
    }
    p.block
}

/// Where a wire's decorations go: its end dots on the ports,
/// and its pill on the middle of the longest segment (the first, when several tie).
/// ponytail: no user-set pill position (the TS labelT) and no pill on the router's free segment.
pub fn deco_points(path: &[Vec2]) -> Option<[(Deco, Vec2); 3]> {
    let (a, b) = (*path.first()?, *path.get(1)?);
    let z = *path.last()?;
    let mut longest = (a, b);
    for w in path.windows(2) {
        if let [p, q] = w
            && p.dist(*q) > longest.0.dist(longest.1)
        {
            longest = (*p, *q);
        }
    }
    Some([(Deco::Start, a), (Deco::End, z), (Deco::Pill, (longest.0 + longest.1) * 0.5)])
}
