//! Lighting content is laid out in ChamSys Designer's pixels (96 to the inch) and drawn at
//! `px` drawing units per pixel, from the drawing's `$INSUNITS`. The factor is stored in the
//! drawing the first time lighting content goes in, so changing the units later doesn't move it.

use cadcraft_doc::{Drawing, EntityKind};

const VAR: &str = "CSDPX";
const PX_PER_IN: f64 = 96.0;

/// Inches in one `$INSUNITS` unit; unitless counts as inches, as AutoCAD's default does.
fn inches_per_unit(insunits: i64) -> f64 {
    match insunits {
        2 => 12.0,
        3 => 63_360.0,
        4 => 1.0 / 25.4,
        5 => 1.0 / 2.54,
        6 => 1000.0 / 25.4,
        7 => 1.0e6 / 25.4,
        8 => 1.0e-6,
        9 => 1.0e-3,
        10 => 36.0,
        11 => 1.0e-7 / 25.4,
        12 => 1.0e-6 / 25.4,
        13 => 1.0e-3 / 25.4,
        14 => 100.0 / 25.4,
        15 => 10_000.0 / 25.4,
        16 => 1.0e5 / 25.4,
        17 => 1.0e12 / 25.4,
        18 => 1.495_978_707e14 / 25.4,
        19 => 9.460_730_472_580_8e18 / 25.4,
        20 => 3.085_677_581_49e19 / 25.4,
        _ => 1.0,
    }
}

/// Drawing units per pixel: the stored factor, else 1 for a drawing that already holds lighting
/// content from before the factor existed, else from `$INSUNITS`.
pub(crate) fn px(d: &Drawing) -> f64 {
    if let Some(k) = Some(d.header.f64(VAR, 0.0)).filter(|k| k.is_finite() && *k > 0.0) {
        return k;
    }
    let legacy = d.model.iter().any(|e| matches!(&e.kind, EntityKind::Insert(i) if i.block.starts_with("CSD_")));
    if legacy { 1.0 } else { 1.0 / (PX_PER_IN * inches_per_unit(d.header.i64("INSUNITS", 1))) }
}

/// [`px`], stored in the drawing so later content uses the same factor.
pub(crate) fn pin_px(d: &mut Drawing) -> f64 {
    let k = px(d);
    d.header.set_f64(VAR, k);
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_follows_the_units_and_stays_once_pinned() {
        let mut d = Drawing::new_imperial();
        assert!((px(&d) - 1.0 / 96.0).abs() < 1e-12);
        d.header.set_i64("INSUNITS", 4);
        assert!((px(&d) - 25.4 / 96.0).abs() < 1e-12);
        assert!((pin_px(&mut d) - 25.4 / 96.0).abs() < 1e-12);
        d.header.set_i64("INSUNITS", 1);
        assert!((px(&d) - 25.4 / 96.0).abs() < 1e-12, "pinned factor survives a units change");
        d.header.set_i64("INSUNITS", 6);
        d.header.set_f64(VAR, f64::NAN);
        assert!((px(&d) - 0.0254 / 96.0).abs() < 1e-15, "a bad stored factor is ignored");
    }
}
