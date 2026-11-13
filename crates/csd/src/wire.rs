//! Geometry of a wire's hand-set shape: smooth routing through waypoints and positions along a path.

use cadcraft_geom::Vec2;

/// Segments per leg of a smooth wire through waypoints.
const LEG_SEGMENTS: usize = 16;

/// A curve from `from` (leaving along `d_out`) to `to` (arriving against `d_in`) through every
/// point of `via`: a Catmull-Rom spline as cubic Hermite legs, flattened.
pub(crate) fn smooth(from: Vec2, d_out: Vec2, via: &[Vec2], to: Vec2, d_in: Vec2) -> Vec<Vec2> {
    let mut p = Vec::with_capacity(via.len() + 2);
    p.push(from);
    p.extend_from_slice(via);
    p.push(to);
    let at = |i: usize| p.get(i).copied().unwrap_or(to);
    let last = p.len() - 1;
    let tangent = |i: usize| match i {
        0 => d_out * (at(1).dist(from) * 1.2),
        i if i == last => d_in * (-at(i).dist(at(i - 1)) * 1.2),
        i => (at(i + 1) - at(i - 1)) * 0.5,
    };
    let mut out = Vec::with_capacity(last * LEG_SEGMENTS + 1);
    for i in 0..last {
        let (a, b) = (at(i), at(i + 1));
        let (c1, c2) = (a + tangent(i) * (1.0 / 3.0), b - tangent(i + 1) * (1.0 / 3.0));
        for k in 0..LEG_SEGMENTS {
            let t = k as f64 / LEG_SEGMENTS as f64;
            let u = 1.0 - t;
            out.push(a * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + b * (t * t * t));
        }
    }
    out.push(to);
    out
}

fn path_len(path: &[Vec2]) -> f64 {
    path.windows(2).map(|w| w[0].dist(w[1])).sum()
}

/// The point `t` (0..=1) of the way along `path` by length.
pub fn point_at(path: &[Vec2], t: f64) -> Option<Vec2> {
    let mut left = path_len(path) * t.clamp(0.0, 1.0);
    for w in path.windows(2) {
        let l = w[0].dist(w[1]);
        if left <= l && l > 0.0 {
            return Some(w[0].lerp(w[1], left / l));
        }
        left -= l;
    }
    path.last().copied()
}

/// The point of the segment `a`-`b` nearest `at`.
fn foot(a: Vec2, b: Vec2, at: Vec2) -> Vec2 {
    let d = b - a;
    let l2 = d.len2();
    if l2 < 1e-18 { a } else { a.lerp(b, ((at - a).dot(d) / l2).clamp(0.0, 1.0)) }
}

/// The leg of `path` nearest `at`: its index, the nearest point on it and the distance to it.
pub fn nearest_leg(path: &[Vec2], at: Vec2) -> Option<(usize, Vec2, f64)> {
    path.windows(2)
        .enumerate()
        .map(|(i, w)| {
            let f = foot(w[0], w[1], at);
            (i, f, f.dist(at))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// The fraction of `path`'s length at the point nearest `at`; with `whole_leg`, at the middle of
/// that leg instead.
pub fn fraction_near(path: &[Vec2], at: Vec2, whole_leg: bool) -> Option<f64> {
    let (i, f, _) = nearest_leg(path, at)?;
    let before = path_len(path.get(..=i)?);
    let (a, b) = (*path.get(i)?, *path.get(i + 1)?);
    let along = if whole_leg { a.dist(b) / 2.0 } else { a.dist(f) };
    let total = path_len(path);
    Some(if total > 0.0 { ((before + along) / total).clamp(0.0, 1.0) } else { 0.0 })
}
