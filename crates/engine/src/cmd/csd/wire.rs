//! Cable waypoints, straightening, label position and names. Each command edits the wire's
//! [`WireLink`]; the post-command pass re-routes the polyline and moves the pill.

use cadcraft_csd::{MAX_WIRE_POINTS, WireLink, fraction_near, nearest_leg};
use cadcraft_doc::Handle;
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{Laid, layout, wires};
use crate::cmd::curves::handle_param;
use crate::cmd::{CommandSpec, bad, point_param, point_req, str_param};
use crate::{Result, Session};

/// How close (drawing units) a click must be to a waypoint to pick or remove it.
const PICK: f64 = 9.0;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("csd.wire.hit", "Find Wire Point", run_hit)
            .params("{at: [x,y], tolerance? (default 9)} → {wire: handle|null, point: index|null, points: count}; a waypoint under `at` wins over a wire's line")
            .noundo(),
        CommandSpec::new("csd.wire.point.add", "Add Wire Point", run_add)
            .params("{wire: handle, at: [x,y]} → {index}; inserted on the nearest leg, on the snap grid if SNAPMODE is on"),
        CommandSpec::new("csd.wire.point.remove", "Remove Wire Point", run_remove)
            .params("{wire: handle, index? | at?: [x,y] (nearest waypoint within 9)} → {points: count}"),
        CommandSpec::new("csd.wire.point.move", "Move Wire Point", run_move).params("{wire: handle, index, to: [x,y]} → {index}"),
        CommandSpec::new("csd.wire.straighten", "Straighten Wires", run_straighten)
            .params("{wires?: [handle]} (default every bent wire) → {count}; clears the waypoints"),
        CommandSpec::new("csd.wire.bent", "Bent Wires", run_bent).params("{} → {count}; wires that have waypoints").noundo(),
        CommandSpec::new("csd.wire.label", "Wire Label Position", run_label)
            .params("{wire: handle, at?: [x,y], mode?: \"segment\"} → {labelT: number|null}; `at` puts the label at the nearest point of the path, with mode segment at the middle of the nearest leg; neither resets it to the default spot"),
        CommandSpec::new("csd.wire.set", "Rename Wire", run_set).params("{wire: handle, name: string} → {name}; an empty name clears it"),
    ]
}

fn wire_param(s: &Session, cmd: &str, p: &Value) -> Result<(Handle, WireLink)> {
    let h = handle_param(p, "wire").ok_or_else(|| bad(cmd, "`wire` (object handle) is required"))?;
    let store = s.doc()?.space(&s.space()).ok_or_else(|| bad(cmd, "no such space"))?;
    wires(store).into_iter().find(|(w, _)| *w == h).ok_or_else(|| bad(cmd, "`wire` is not a CSD wire"))
}

fn store_link(s: &mut Session, h: Handle, link: &WireLink) -> Result<()> {
    Ok(s.doc_mut()?.modify_entity(h, |e| e.common.xdata = link.to_xdata())?)
}

/// The wire's routed path, ports included.
fn path_of(s: &Session, h: Handle) -> Result<Vec<Vec2>> {
    let store = s.doc()?.space(&s.space()).ok_or_else(|| bad("csd.wire", "no such space"))?;
    Ok(layout(store, s.settings.orthomode).into_iter().find(|l| l.wire == h).map(|l| l.path).unwrap_or_default())
}

fn snapped(s: &Session, at: Vec2) -> Vec2 {
    let unit = s.settings.snapunit;
    let round = |v: f64, u: f64| if u.is_finite() && u > 0.0 { (v / u).round() * u } else { v };
    if s.settings.snapmode { Vec2::new(round(at.x, unit.x), round(at.y, unit.y)) } else { at }
}

fn index_param(cmd: &str, p: &Value, link: &WireLink) -> Result<usize> {
    let i = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    i.filter(|i| *i < link.points.len()).ok_or_else(|| bad(cmd, "`index` is not a waypoint of this wire"))
}

fn run_hit(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_req("csd.wire.hit", p, "at")?;
    let tol = p.get("tolerance").and_then(Value::as_f64).filter(|t| t.is_finite() && *t >= 0.0).unwrap_or(PICK);
    let store = s.doc()?.space(&s.space()).ok_or_else(|| bad("csd.wire.hit", "no such space"))?;
    let (mut point, mut line): (Option<(f64, Handle, usize, usize)>, Option<(f64, Handle, usize)>) = (None, None);
    for Laid { wire, link, path, .. } in layout(store, s.settings.orthomode) {
        for (i, pt) in link.points.iter().enumerate() {
            let d = pt.dist(at);
            if d <= tol && point.is_none_or(|b| d < b.0) {
                point = Some((d, wire, i, link.points.len()));
            }
        }
        if let Some((_, _, d)) = nearest_leg(&path, at)
            && d <= tol
            && line.is_none_or(|b| d < b.0)
        {
            line = Some((d, wire, link.points.len()));
        }
    }
    Ok(match (point, line) {
        (Some((_, w, i, n)), _) => json!({ "wire": w.hex(), "point": i, "points": n }),
        (None, Some((_, w, n))) => json!({ "wire": w.hex(), "point": null, "points": n }),
        _ => json!({ "wire": null, "point": null, "points": 0 }),
    })
}

fn run_add(s: &mut Session, p: &Value) -> Result<Value> {
    let (h, mut link) = wire_param(s, "csd.wire.point.add", p)?;
    let at = snapped(s, point_req("csd.wire.point.add", p, "at")?);
    if link.points.len() >= MAX_WIRE_POINTS {
        return Err(bad("csd.wire.point.add", format!("a wire keeps at most {MAX_WIRE_POINTS} waypoints")));
    }
    let path = path_of(s, h)?;
    // Legs run between the ports and the waypoints, not along the routed path's elbows.
    let chain: Vec<Vec2> = path.first().into_iter().chain(&link.points).chain(path.last()).copied().collect();
    let index = nearest_leg(&chain, at).map_or(link.points.len(), |(i, ..)| i);
    link.points.insert(index, at);
    store_link(s, h, &link)?;
    Ok(json!({ "index": index }))
}

fn run_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let (h, mut link) = wire_param(s, "csd.wire.point.remove", p)?;
    let index = match point_param(p, "at") {
        Some(at) if p.get("index").is_none() => link
            .points
            .iter()
            .enumerate()
            .map(|(i, pt)| (i, pt.dist(at)))
            .filter(|(_, d)| *d <= PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
            .ok_or_else(|| bad("csd.wire.point.remove", "no waypoint near `at`"))?,
        _ => index_param("csd.wire.point.remove", p, &link)?,
    };
    link.points.remove(index);
    store_link(s, h, &link)?;
    Ok(json!({ "points": link.points.len() }))
}

fn run_move(s: &mut Session, p: &Value) -> Result<Value> {
    let (h, mut link) = wire_param(s, "csd.wire.point.move", p)?;
    let index = index_param("csd.wire.point.move", p, &link)?;
    let to = point_req("csd.wire.point.move", p, "to")?;
    if let Some(pt) = link.points.get_mut(index) {
        *pt = to;
    }
    store_link(s, h, &link)?;
    Ok(json!({ "index": index }))
}

fn run_straighten(s: &mut Session, p: &Value) -> Result<Value> {
    let all = {
        let store = s.doc()?.space(&s.space()).ok_or_else(|| bad("csd.wire.straighten", "no such space"))?;
        wires(store)
    };
    let only: Option<Vec<Handle>> = match p.get("wires") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_array()
                .ok_or_else(|| bad("csd.wire.straighten", "`wires` must be an array of handles"))?
                .iter()
                .map(|v| v.as_str().and_then(Handle::parse_hex).ok_or_else(|| bad("csd.wire.straighten", format!("{v} is not an object handle"))))
                .collect::<Result<_>>()?,
        ),
    };
    let mut count = 0;
    for (h, mut link) in all {
        if link.points.is_empty() || only.as_ref().is_some_and(|o| !o.contains(&h)) {
            continue;
        }
        link.points.clear();
        store_link(s, h, &link)?;
        count += 1;
    }
    Ok(json!({ "count": count }))
}

fn run_bent(s: &mut Session, _p: &Value) -> Result<Value> {
    let store = s.doc()?.space(&s.space()).ok_or_else(|| bad("csd.wire.bent", "no such space"))?;
    Ok(json!({ "count": wires(store).iter().filter(|(_, l)| !l.points.is_empty()).count() }))
}

fn run_label(s: &mut Session, p: &Value) -> Result<Value> {
    let (h, mut link) = wire_param(s, "csd.wire.label", p)?;
    let segment = match str_param(p, "mode") {
        None => false,
        Some("segment") => true,
        Some(m) => return Err(bad("csd.wire.label", format!("unknown mode \"{m}\""))),
    };
    link.label_t = match point_param(p, "at") {
        Some(at) => fraction_near(&path_of(s, h)?, at, segment),
        None if segment => return Err(bad("csd.wire.label", "mode segment needs `at`")),
        None => None,
    };
    store_link(s, h, &link)?;
    Ok(json!({ "labelT": link.label_t }))
}

fn run_set(s: &mut Session, p: &Value) -> Result<Value> {
    let (h, mut link) = wire_param(s, "csd.wire.set", p)?;
    let name = str_param(p, "name").ok_or_else(|| bad("csd.wire.set", "`name` (string) is required"))?;
    link.name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
    store_link(s, h, &link)?;
    Ok(json!({ "name": link.name }))
}
