//! ChamSys annotation shapes (`shapes` in a `.cpsd`) as ordinary entities on [`SHAPES_LAYER`], and back.

use cadcraft_doc::color::{Color, Rgb};
use cadcraft_doc::{Ellipse, EntityKind, EntityStore, Hatch, HatchLoop, Layer, Line, Lineweight, LwPolyline, MText, Transparency, mtext_literal};
use cadcraft_geom::{PolyVertex, TAU, Vec2, Vec3};
use serde_json::{Value, json};

use super::cpsd::{from_drawing, number, text, to_drawing};
use super::linetype_scale;
use crate::{Result, Session};

pub(super) const SHAPES_LAYER: &str = "CSD_SHAPES";
pub(super) const MAX_SHAPES: usize = 20_000;
const MAX_TEXT_CHARS: usize = 10_000;
const MAX_FONT: f64 = 1_000.0;
const DEFAULT_FONT: f64 = 16.0;
const EPS: f64 = 1e-6;
/// ChamSys stroke widths are in sheet units; one unit is 0.1 mm of lineweight.
const WEIGHT_UNIT: f64 = 10.0;
const DEFAULT_WEIGHT: f64 = 2.0;
/// Dash (or dot gap) length per unit of stroke width, as ChamSys draws `dashed` and `dotted`.
const DASH_PER_WEIGHT: f64 = 3.0;
const DOT_PER_WEIGHT: f64 = 2.0;

fn hex(v: &Value, key: &str) -> Option<Rgb> {
    let t = text(v, key);
    let t = t.strip_prefix('#').filter(|t| t.len() == 6)?;
    u32::from_str_radix(t, 16).ok().map(Rgb::from_u32)
}

/// Draws one shape of the file; `skip` hears why a shape was left out.
/// `dx` shifts the shape right, onto its page's sheet.
fn import_one(s: &mut Session, v: &Value, dx: f64, mut skip: impl FnMut(&'static str)) -> Result<bool> {
    if !v.is_object() {
        skip("shape that is not an object");
        return Ok(false);
    }
    let kind = text(v, "type");
    let coord = |k| number(v, k).unwrap_or(0.0);
    let (Some(a), Some(b)) = (to_drawing(coord("x1") + dx, coord("y1")), to_drawing(coord("x2") + dx, coord("y2"))) else {
        skip("shape outside the drawing limits");
        return Ok(false);
    };
    let (lo, hi) = (Vec2::new(a.x.min(b.x), a.y.min(b.y)), Vec2::new(a.x.max(b.x), a.y.max(b.y)));
    let mut color = hex(v, "color");
    let shape = match kind.as_str() {
        "line" => EntityKind::Line(Line { a: Vec3::new(a.x, a.y, 0.0), b: Vec3::new(b.x, b.y, 0.0) }),
        "ellipse" => {
            let (rx, ry) = ((hi.x - lo.x) / 2.0, (hi.y - lo.y) / 2.0);
            if rx <= 0.0 || ry <= 0.0 {
                skip("ellipse with no size");
                return Ok(false);
            }
            let centre = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, 0.0);
            let (major, ratio) = if rx >= ry { (Vec3::new(rx, 0.0, 0.0), ry / rx) } else { (Vec3::new(0.0, ry, 0.0), rx / ry) };
            EntityKind::Ellipse(Ellipse { center: centre, major, ratio, start: 0.0, end: TAU })
        }
        "text" => {
            color = hex(v, "textColor");
            let height = number(v, "fontSize").filter(|h| *h > 0.0).unwrap_or(DEFAULT_FONT).min(MAX_FONT);
            let contents = mtext_literal(&text(v, "text").chars().take(MAX_TEXT_CHARS).collect::<String>());
            let (ax, code) = match text(v, "textAlign").as_str() {
                "center" => ((lo.x + hi.x) / 2.0, 2),
                "right" => (hi.x, 3),
                _ => (lo.x, 1),
            };
            EntityKind::MText(MText {
                insert: Vec3::new(ax, hi.y, 0.0),
                height,
                width: hi.x - lo.x,
                attach: code,
                rotation: 0.0,
                style: "Standard".into(),
                contents,
                line_spacing: 1.0,
                line_spacing_exact: false,
                columns: None,
            })
        }
        "image" => {
            skip("image shape (not supported yet)");
            return Ok(false);
        }
        _ => {
            let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
            let vertices = corners.into_iter().map(PolyVertex::new).collect();
            EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false })
        }
    };
    let stroke = text(v, "strokeStyle");
    let weight = number(v, "weight").filter(|w| w.is_finite() && *w >= 0.0);
    let line_style = match stroke.as_str() {
        "dashed" => Some(("DASHED", DASH_PER_WEIGHT)),
        "dotted" => Some(("DOT", DOT_PER_WEIGHT)),
        _ => None,
    };
    let ltscale = match line_style {
        Some((name, per)) => linetype_scale(s.doc_mut()?, name, per * weight.unwrap_or(DEFAULT_WEIGHT)),
        None => 1.0,
    };
    let lineweight = weight.map(|w| {
        let want = (w * WEIGHT_UNIT).min(211.0);
        Lineweight::Mm100(
            Lineweight::STANDARD.into_iter().min_by(|a, b| (f64::from(*a) - want).abs().total_cmp(&(f64::from(*b) - want).abs())).unwrap_or(0),
        )
    });
    let fill = hex(v, "fill").filter(|_| !matches!(kind.as_str(), "ellipse" | "text" | "line"));
    if hex(v, "fill").is_some() && kind == "ellipse" {
        skip("fill of an ellipse (only rectangles take a fill)");
    }
    if let Some(c) = fill {
        let opacity = number(v, "fillOpacity").filter(|o| o.is_finite()).map_or(1.0, |o| o.clamp(0.0, 1.0));
        let percent = ((1.0 - opacity) * 100.0).round().min(90.0) as u8;
        let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
        let hatch = EntityKind::Hatch(Hatch {
            pattern: "SOLID".into(),
            solid: true,
            loops: vec![HatchLoop { vertices: corners.into_iter().map(PolyVertex::new).collect(), outer: true, sources: Vec::new() }],
            scale: 1.0,
            angle: 0.0,
            associative: false,
            style: 0,
            elevation: 0.0,
            gradient: None,
            origin: Vec2::ZERO,
            background: None,
            pattern_lines: Vec::new(),
        });
        let f = s.add_entity(hatch)?;
        s.doc_mut()?.modify_entity(f, |e| {
            e.common.layer = SHAPES_LAYER.into();
            e.common.color = Color::True(c);
            if percent > 0 {
                e.common.transparency = Transparency::Percent(percent);
            }
        })?;
    }
    let is_text = kind == "text";
    let h = s.add_entity(shape)?;
    s.doc_mut()?.modify_entity(h, |e| {
        e.common.layer = SHAPES_LAYER.into();
        if let Some(c) = color {
            e.common.color = Color::True(c);
        }
        if !is_text {
            if let Some(lw) = lineweight {
                e.common.lineweight = lw;
            }
            if let Some((name, _)) = line_style {
                e.common.linetype = name.into();
                e.common.ltscale = ltscale;
            }
        }
    })?;
    Ok(true)
}

/// Imports `shapes`; returns how many were drawn.
pub(super) fn import<'a>(s: &mut Session, shapes: impl Iterator<Item = (&'a Value, f64)>, mut skip: impl FnMut(&'static str)) -> Result<usize> {
    let mut n = 0;
    for (v, dx) in shapes {
        if s.doc()?.layer(SHAPES_LAYER).is_none() {
            s.doc_mut()?.layers.push(Layer::new(SHAPES_LAYER));
        }
        n += usize::from(import_one(s, v, dx, &mut skip)?);
    }
    Ok(n)
}

fn corners(a: Vec2, b: Vec2) -> Value {
    let ((x1, y1), (x2, y2)) = (from_drawing(a), from_drawing(b));
    json!({ "x1": x1, "y1": y1, "x2": x2, "y2": y2 })
}

/// A box (low and high corner in the drawing) as ChamSys's top-left and bottom-right corners.
fn boxed((lo, hi): (Vec2, Vec2)) -> Value {
    corners(Vec2::new(lo.x, hi.y), Vec2::new(hi.x, lo.y))
}

fn put_color(shape: &mut Value, key: &str, c: Color) {
    if let Color::True(c) = c {
        shape[key] = json!(format!("#{:06x}", c.to_u32()));
    }
}

/// The entities on [`SHAPES_LAYER`] as `shapes`, and how many of them have no ChamSys kind.
pub(super) fn export(store: &EntityStore) -> (Vec<Value>, usize) {
    let (mut out, mut skipped) = (Vec::new(), 0);
    let ents: Vec<_> = store.iter().filter(|e| e.common.layer == SHAPES_LAYER).collect();
    // Solid rectangular hatches are the fills of the rectangles drawn over them.
    let fills: Vec<_> = ents
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Hatch(h) if h.solid && h.loops.len() == 1 => h.loops.first().and_then(|l| box_of(&l.vertices)).map(|b| (b, *e)),
            _ => None,
        })
        .collect();
    let mut used = vec![false; fills.len()];
    let fill_handles: std::collections::HashSet<_> = fills.iter().map(|(_, f)| f.handle).collect();
    for (n, e) in ents.iter().enumerate() {
        if fill_handles.contains(&e.handle) {
            continue;
        }
        let id = format!("sh{n}");
        let shape = match &e.kind {
            EntityKind::Line(l) => Some(("line", corners(l.a.xy(), l.b.xy()))),
            EntityKind::LwPolyline(p) => rect_of(p).map(|b| {
                let mut r = boxed(b);
                let hit = fills.iter().enumerate().find(|(i, (fb, _))| !used[*i] && fb.0.near(b.0, EPS) && fb.1.near(b.1, EPS));
                if let Some((i, (_, f))) = hit {
                    used[i] = true;
                    put_color(&mut r, "fill", f.common.color);
                    if let Transparency::Percent(p) = f.common.transparency {
                        r["fillOpacity"] = json!(1.0 - f64::from(p) / 100.0);
                    }
                }
                ("rect", r)
            }),
            EntityKind::Ellipse(el) => ellipse_box(el).map(|b| ("ellipse", boxed(b))),
            EntityKind::MText(t) => {
                let w = t.width.max(0.0);
                let (left, align) = match t.attach {
                    2 => (t.insert.x - w / 2.0, "center"),
                    3 => (t.insert.x - w, "right"),
                    _ => (t.insert.x, "left"),
                };
                let mut c = corners(Vec2::new(left, t.insert.y), Vec2::new(left + w, t.insert.y - t.height));
                c["textAlign"] = json!(align);
                c["text"] = json!(cadcraft_fonts::plain_mtext(&t.contents));
                c["fontSize"] = json!(t.height);
                (1..=3).contains(&t.attach).then_some(("text", c))
            }
            _ => None,
        };
        let Some((kind, mut v)) = shape else {
            skipped += 1;
            continue;
        };
        v["id"] = json!(id);
        v["type"] = json!(kind);
        put_color(&mut v, if kind == "text" { "textColor" } else { "color" }, e.common.color);
        if kind != "text" {
            if let Lineweight::Mm100(w) = e.common.lineweight {
                v["weight"] = json!(f64::from(w) / WEIGHT_UNIT);
            }
            match e.common.linetype.as_str() {
                "DASHED" => v["strokeStyle"] = json!("dashed"),
                "DOT" => v["strokeStyle"] = json!("dotted"),
                _ => {}
            }
        }
        out.push(v);
    }
    skipped += used.iter().filter(|u| !**u).count();
    (out, skipped)
}

/// The corners of a closed, axis-aligned, four-corner polyline.
fn rect_of(p: &LwPolyline) -> Option<(Vec2, Vec2)> {
    p.closed.then(|| box_of(&p.vertices)).flatten()
}

/// The corners of four axis-aligned, straight-sided vertices that sit on the corners of their box.
fn box_of(vertices: &[PolyVertex]) -> Option<(Vec2, Vec2)> {
    if vertices.len() != 4 || vertices.iter().any(|v| v.bulge != 0.0) {
        return None;
    }
    let xs = vertices.iter().map(|v| v.p.x);
    let ys = vertices.iter().map(|v| v.p.y);
    let (lo, hi) = (
        Vec2::new(xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min)),
        Vec2::new(xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)),
    );
    let on_corner =
        |v: &PolyVertex| ((v.p.x - lo.x).abs() < EPS || (v.p.x - hi.x).abs() < EPS) && ((v.p.y - lo.y).abs() < EPS || (v.p.y - hi.y).abs() < EPS);
    vertices.iter().all(on_corner).then_some((lo, hi))
}

/// The bounding box of a whole ellipse whose axes run along x and y.
fn ellipse_box(e: &Ellipse) -> Option<(Vec2, Vec2)> {
    if (e.end - e.start - TAU).abs() > EPS || e.ratio.is_nan() || e.ratio <= 0.0 {
        return None;
    }
    let (rx, ry) = if e.major.y.abs() < EPS {
        (e.major.x.abs(), e.major.x.abs() * e.ratio)
    } else if e.major.x.abs() < EPS {
        (e.major.y.abs() * e.ratio, e.major.y.abs())
    } else {
        return None;
    };
    let c = e.center.xy();
    Some((Vec2::new(c.x - rx, c.y - ry), Vec2::new(c.x + rx, c.y + ry)))
}
