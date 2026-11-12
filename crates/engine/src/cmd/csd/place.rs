//! Click-to-place: the armed product goes down at every point until Enter or Esc, as in the web
//! app. Placing goes through the JSON form, so both make the same device.

use cadcraft_csd::{CARD_H, CARD_W};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{bad, run_place, str_param};
use crate::cmd::helpers::{lwpoly, rect_vertices};
use crate::{Accept, EngineError, EntityKind, Input, Interactive, Prompt, Result, Session, Step};

/// `csd.arm {product}`: names the product `csd.place` puts down next.
pub(super) fn run_arm(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "product").ok_or_else(|| bad("csd.arm", "`product` is required"))?;
    if !s.csd_library.iter().any(|x| x.id == id) {
        return Err(bad("csd.arm", format!("unknown product \"{id}\"")));
    }
    s.csd_armed = Some(id.to_string());
    Ok(json!({ "product": id }))
}

pub(super) struct PlaceM {
    product: String,
}

impl PlaceM {
    pub(super) fn new(s: &Session) -> Result<Box<dyn Interactive>> {
        let product = s.csd_armed.clone().ok_or_else(|| EngineError::Other("no product armed: run csd.arm first".into()))?;
        Ok(Box::new(PlaceM { product }))
    }
}

impl Interactive for PlaceM {
    fn name(&self) -> &'static str {
        "CSD.PLACE"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let name = s.csd_library.iter().find(|x| x.id == self.product).map_or(self.product.as_str(), |x| x.name.as_str());
        Prompt::new(format!("Specify insertion point for {name}"), Accept::POINT)
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(p) => {
                run_place(s, &json!({ "product": self.product, "at": [p.x, p.y] }))?;
                Ok(Step::Continue)
            }
            // Esc disarms; what is placed stays.
            Input::Enter | Input::Cancel => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        let h = Vec2::new(CARD_W / 2.0, CARD_H / 2.0);
        vec![lwpoly(rect_vertices(c - h, c + h), true)]
    }
}
