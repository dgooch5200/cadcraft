//! The sheet: a 1632 x 1056 page frame with the title block along its bottom edge, and the line
//! key that lists the protocols a drawing uses. Dimensions follow the web app's sheet; the block
//! origin is the page's lower-left corner.

use super::{
    Attrib, AttribModes, AttribProps, Block, Entity, EntityKind, HAlign, Handle, INK, LwPolyline, Painter, PolyVertex, Protocol, Text, Vec2, legible,
    outline, text, vertex,
};

pub const TITLEBLOCK: &str = "CSD_TITLEBLOCK";
pub const LEGEND: &str = "CSD_LEGEND";

pub const PAGE_W: f64 = 1632.0;
pub const PAGE_H: f64 = 1056.0;
pub const MARGIN: f64 = 22.0;
pub const TITLE_H: f64 = 70.0;
/// Height of the line key, and of the power note under it; the legend's origin is the note's bottom.
pub const KEY_H: f64 = 18.0;
pub const NOTE_H: f64 = 13.0;
const BRAND_W: f64 = 125.0;
const ROW1_H: f64 = 40.0;
const FAINT: u32 = 0x777777;
const RULE: u32 = 0xbbbbbb;

/// The title block's fields: JSON key and attribute tag, in the order `csd.project` lists them.
pub const TITLE_FIELDS: [(&str, &str); 10] = [
    ("brand", "BRAND"),
    ("project", "PROJECT"),
    ("venue", "VENUE"),
    ("location", "LOCATION"),
    ("drawnBy", "DRAWNBY"),
    ("date", "DATE"),
    ("sheet", "SHEET"),
    ("rev", "REV"),
    ("title", "TITLE"),
    ("meta", "META"),
];

/// Row one's cells: attribute tag, caption, and width weight.
const COLUMNS: [(&str, &str, f64); 7] = [
    ("PROJECT", "Project", 200.0),
    ("VENUE", "Venue", 160.0),
    ("LOCATION", "Location", 150.0),
    ("DRAWNBY", "Drawn By", 155.0),
    ("DATE", "Date", 74.0),
    ("SHEET", "Sheet", 64.0),
    ("REV", "Rev", 52.0),
];
/// Row two's drawing title spans this many columns; the meta text takes the rest.
const SPLIT: usize = 4;

pub const POWER_NOTE: &str = "Warning: Power connection lines shown on the one-line are for illustration purposes only and do not include wattage \
     calculations, feasibility analysis, or engineering review. All control and electrical requirements must be independently verified by a \
     qualified professional.";

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<PolyVertex> {
    vec![vertex(x0, y0, 0.0), vertex(x1, y0, 0.0), vertex(x1, y1, 0.0), vertex(x0, y1, 0.0)]
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> EntityKind {
    let v = vec![vertex(x0, y0, 0.0), vertex(x1, y1, 0.0)];
    EntityKind::LwPolyline(LwPolyline { vertices: v, closed: false, const_width: width, elevation: 0.0, plinegen: false })
}

/// Left-aligned text, middle of its height on `y`.
fn label(x: f64, y: f64, height: f64, value: &str) -> Text {
    Text { halign: HAlign::Left, ..text(Vec2 { x, y }, height, value) }
}

fn attdef(tag: &str, prompt: &str, mut t: Text, value: &str) -> EntityKind {
    t.value = value.into();
    EntityKind::AttDef(Attrib {
        tag: tag.into(),
        text: t,
        invisible: false,
        constant: false,
        prompt: prompt.into(),
        props: AttribProps::default(),
        multiline: None,
        modes: AttribModes::default(),
    })
}

/// The page frame and the title block, with an attribute definition for each of [`TITLE_FIELDS`]
/// (all empty but the brand).
pub fn titleblock_block(mut new_handle: impl FnMut() -> Handle) -> Block {
    let mut p = Painter { block: Block::new(TITLEBLOCK), new_handle: &mut new_handle };
    let (x0, y0, x1, top) = (MARGIN, MARGIN, PAGE_W - MARGIN, MARGIN + TITLE_H);
    p.add(outline(rect(x0, y0, x1, PAGE_H - MARGIN), 0.75), 0xcccccc);
    p.add(outline(rect(x0, y0, x1, top), 1.5), INK);
    let (fx, row) = (x0 + BRAND_W, top - ROW1_H);
    p.add(line(fx, y0, fx, top, 1.0), INK);
    p.add(line(fx, row, x1, row, 1.0), INK);
    let cx = x0 + BRAND_W / 2.0;
    p.add(attdef("BRAND", "Brand", Text { halign: HAlign::Center, ..label(cx, top - 32.0, 6.5, "") }, "CHAUVET Professional"), INK);
    p.add(EntityKind::Text(Text { halign: HAlign::Center, ..label(cx, top - 52.0, 4.3, "SYSTEM SKETCH") }), FAINT);

    let total: f64 = COLUMNS.iter().map(|c| c.2).sum();
    let mut x = fx;
    let mut split = fx;
    for (i, (tag, caption, weight)) in COLUMNS.iter().enumerate() {
        if i == SPLIT {
            split = x;
        }
        if i > 0 {
            p.add(line(x, row, x, top, 1.0), 0xcccccc);
        }
        p.add(EntityKind::Text(label(x + 5.0, top - 10.0, 4.3, &caption.to_uppercase())), FAINT);
        p.add(attdef(tag, caption, label(x + 5.0, top - 25.0, 6.5, ""), ""), INK);
        x += (x1 - fx) * weight / total;
    }
    p.add(line(split, y0, split, row, 1.0), 0xcccccc);
    p.add(EntityKind::Text(label(fx + 5.0, row - 8.0, 4.3, "SYSTEM / DRAWING TITLE")), FAINT);
    p.add(attdef("TITLE", "Drawing title", label(fx + 5.0, row - 20.0, 6.5, ""), ""), INK);
    p.add(attdef("META", "Meta", label(split + 5.0, row - 15.0, 5.0, ""), ""), FAINT);
    p.block
}

/// The protocols a legend block lists, read from its labels.
pub fn legend_protocols(block: &Block) -> Vec<Protocol> {
    let on_key = |e: &std::sync::Arc<Entity>| match &e.kind {
        EntityKind::Text(t) => Protocol::ALL.into_iter().find(|p| p.label() == t.value),
        _ => None,
    };
    block.entities.iter().filter_map(on_key).collect()
}

/// The line key listing `used` (in [`Protocol::ALL`] order), each as a 28 unit line in its colour,
/// dashed where the protocol is; with the power note when `used` has power. `dash_scale` is the
/// linetype scale of the dashed ones.
pub fn legend_block(used: &[Protocol], dash_scale: f64, mut new_handle: impl FnMut() -> Handle) -> Block {
    let mut p = Painter { block: Block::new(LEGEND), new_handle: &mut new_handle };
    let w = PAGE_W - MARGIN * 2.0;
    p.add(outline(rect(0.0, NOTE_H, w, NOTE_H + KEY_H), 0.5), RULE);
    let mid = NOTE_H + KEY_H / 2.0;
    p.add(EntityKind::Text(label(5.0, mid, 5.0, "LINE KEY:")), FAINT);
    let protocols: Vec<Protocol> = Protocol::ALL.into_iter().filter(|x| used.contains(x)).collect();
    let item_w = ((w - 60.0) / protocols.len().max(1) as f64).min(120.0);
    for (i, pr) in protocols.iter().enumerate() {
        let x = 60.0 + i as f64 * item_w;
        let mut e = p.entity(line(x, mid, x + 28.0, mid, 2.0), pr.color());
        if pr.dashed() {
            e.common.linetype = "DASHED2".into();
            e.common.ltscale = dash_scale;
        }
        p.block.entities.push(e);
        p.add(EntityKind::Text(label(x + 32.0, mid, 5.0, pr.label())), legible(pr.color()));
    }
    if protocols.contains(&Protocol::Power) {
        p.add(EntityKind::Text(label(0.0, NOTE_H / 2.0 - 1.0, 5.0, POWER_NOTE)), FAINT);
    }
    p.block
}
