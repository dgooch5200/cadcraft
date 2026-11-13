//! Per-field checks for the library worksheet. The web app imports leniently (a bad mode entry or
//! colour is dropped or defaulted) and refuses on a blank or shared Model ID; this reports the same
//! faults, plus the mode entries it would drop, so a row can be fixed before it is applied.

use std::collections::HashSet;

use crate::{Product, product_key};

/// One fault: the row index, the record key of the field (`id`, `name`, `modes`, `poeWatts`,
/// `poePortWatts`) and what is wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct Invalid {
    pub row: usize,
    pub field: &'static str,
    pub message: String,
}

/// Every fault in `rows`: an empty or shared (by [`product_key`]) id, an empty name, a `modes` entry
/// that is not `name:channels` with a positive whole channel count, and watts that are negative or
/// not finite.
pub fn validate_library(rows: &[Product]) -> Vec<Invalid> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (row, p) in rows.iter().enumerate() {
        let mut bad = |field: &'static str, message: String| out.push(Invalid { row, field, message });
        let id = p.id.trim();
        if id.is_empty() {
            bad("id", "The Model ID is empty.".into());
        } else if !seen.insert(product_key(&p.id)) {
            bad("id", format!("The Model ID \"{id}\" is used by another row."));
        }
        if p.name.trim().is_empty() {
            bad("name", "The product name is empty.".into());
        }
        if let Some(entry) = p.modes.split(',').map(str::trim).filter(|e| !e.is_empty()).find(|e| !mode_ok(e)) {
            bad("modes", format!("\"{entry}\" is not a mode: write name:channels, with a whole number above 0."));
        }
        for (field, watts) in [("poeWatts", p.poe_watts), ("poePortWatts", p.poe_port_watts)] {
            if !watts.is_finite() || watts < 0.0 {
                bad(field, "Watts must be 0 or more.".into());
            }
        }
    }
    out
}

/// `name:channels` split at the last colon, as the patch sheet reads it.
fn mode_ok(entry: &str) -> bool {
    entry.rsplit_once(':').is_some_and(|(name, n)| !name.trim().is_empty() && n.trim().parse::<u32>().is_ok_and(|n| n > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(id: &str) -> Product {
        Product { id: id.into(), name: "Name".into(), ..Product::default() }
    }

    fn faults(rows: &[Product]) -> Vec<(usize, &'static str)> {
        validate_library(rows).iter().map(|i| (i.row, i.field)).collect()
    }

    #[test]
    fn a_good_library_is_clean() {
        let mut p = product("a");
        p.modes = "17ch:17, 34ch:34, Pixel: 1:1 map:720,".into();
        assert!(faults(&[p, product("b")]).is_empty());
    }

    #[test]
    fn id_and_name_must_be_present_and_ids_unique_after_whitespace_normalising() {
        let mut noname = product("c");
        noname.name = "  ".into();
        let rows = [product(" "), product("a b"), product("a  b"), noname, product("A B")];
        assert_eq!(faults(&rows), [(0, "id"), (2, "id"), (3, "name")]);
    }

    #[test]
    fn modes_need_a_name_and_a_positive_whole_channel_count() {
        for bad in ["17", ":17", "Std:0", "Std:-3", "Std:1.5", "Std:x", "ok:1, bad"] {
            let mut p = product("a");
            p.modes = bad.into();
            assert_eq!(faults(&[p]), [(0, "modes")], "{bad}");
        }
    }

    #[test]
    fn watts_must_be_finite_and_not_negative() {
        let mut p = product("a");
        p.poe_watts = -1.0;
        p.poe_port_watts = f64::NAN;
        assert_eq!(faults(&[p]), [(0, "poeWatts"), (0, "poePortWatts")]);
    }
}
