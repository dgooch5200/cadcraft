//! Click-to-place: the armed product goes down at every point until Enter or Esc, as in the web
//! app. With nothing armed it first asks for a product id. Placing goes through the JSON form, so
//! both make the same device.

use cadcraft_csd::{CARD_H, CARD_W, lookup};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{bad, point_req, run_place, str_param};
use crate::cmd::helpers::{lwpoly, rect_vertices};
use crate::{Accept, EntityKind, Input, Interactive, Prompt, Result, Session, Step};

/// `csd.arm {product}`: names the product `csd.place` puts down next.
pub(super) fn run_arm(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "product").ok_or_else(|| bad("csd.arm", "`product` is required"))?;
    if lookup(&s.csd_library, id).is_none() {
        return Err(bad("csd.arm", format!("unknown product \"{id}\"")));
    }
    s.csd_armed = Some(id.to_string());
    Ok(json!({ "product": id }))
}

/// The most devices one `csd.place.grid` puts down.
const MAX_GRID: u64 = 400;
/// The web app's default space between neighbouring cards.
const GRID_GAP: f64 = 24.0;

/// `csd.place.grid`: `at` is the first card's centre; rows run downwards, columns to the right.
pub(super) fn run_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "csd.place.grid";
    let count = |key: &str| {
        p.get(key).and_then(Value::as_u64).filter(|n| *n >= 1).ok_or_else(|| bad(cmd, format!("`{key}` must be a whole number, 1 or more")))
    };
    let (rows, cols) = (count("rows")?, count("cols")?);
    if rows.saturating_mul(cols) > MAX_GRID {
        return Err(bad(cmd, format!("a grid holds at most {MAX_GRID} devices")));
    }
    let step = |key: &str, default: f64| match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number"))),
    };
    let (dx, dy) = (step("dx", CARD_W + GRID_GAP)?, step("dy", CARD_H + GRID_GAP)?);
    let at = point_req(cmd, p, "at")?;
    let product = str_param(p, "product").ok_or_else(|| bad(cmd, "`product` is required"))?;
    let mut handles = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let (x, y) = (at.x + c as f64 * dx, at.y - r as f64 * dy);
            let placed = run_place(s, &json!({ "product": product, "at": [x, y] }))?;
            handles.extend(placed.get("handle").cloned());
        }
    }
    Ok(json!({ "handles": handles }))
}

pub(super) struct PlaceM {
    product: Option<String>,
}

impl PlaceM {
    pub(super) fn start(s: &Session) -> Result<Box<dyn Interactive>> {
        Ok(Box::new(PlaceM { product: s.csd_armed.clone() }))
    }
}

impl Interactive for PlaceM {
    fn name(&self) -> &'static str {
        "CSD.PLACE"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let Some(product) = &self.product else {
            return Prompt::new("Enter product id", Accept::TEXT);
        };
        let name = lookup(&s.csd_library, product).map_or(product.as_str(), |x| x.name.as_str());
        Prompt::new(format!("Specify insertion point for {name}"), Accept::POINT)
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match (i, &self.product) {
            (Input::Text(id), None) => {
                match run_arm(s, &json!({ "product": id.trim() })) {
                    Ok(_) => self.product = s.csd_armed.clone(),
                    Err(e) => s.echo(e.to_string()),
                }
                Ok(Step::Continue)
            }
            (Input::Point(p), Some(product)) => {
                run_place(s, &json!({ "product": product, "at": [p.x, p.y] }))?;
                Ok(Step::Continue)
            }
            // Esc disarms; what is placed stays.
            (Input::Enter | Input::Cancel, _) => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        let h = Vec2::new(CARD_W / 2.0, CARD_H / 2.0);
        vec![lwpoly(rect_vertices(c - h, c + h), true)]
    }
}
