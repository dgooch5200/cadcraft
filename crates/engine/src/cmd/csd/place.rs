//! Click-to-place: the armed product goes down at every point until Enter or Esc, as in the web
//! app. With nothing armed it first asks for a product id. Placing goes through the JSON form, so
//! both make the same device.

use cadcraft_csd::{CARD_H, CARD_W};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{bad, run_place, str_param};
use crate::cmd::helpers::{lwpoly, rect_vertices};
use crate::{Accept, EntityKind, Input, Interactive, Prompt, Result, Session, Step};

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
        let name = s.csd_library.iter().find(|x| &x.id == product).map_or(product.as_str(), |x| x.name.as_str());
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
