//! Click-to-connect: pick a port on one device, then a port on another, until Enter or Esc. A wire
//! goes through the JSON form, so both make the same wire.

use cadcraft_csd::{CARD_H, CARD_W, Protocol, Side, Slot, port_point};
use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::Vec2;
use serde_json::json;

use super::{devices, run_connect};
use crate::cmd::helpers::line;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// How far outside a card a click still picks it.
const MARGIN: f64 = 10.0;

type Port = (Handle, Side, Vec2);

/// The device whose card (plus margin) holds `p`, and the card side whose port is nearest.
fn pick(s: &Session, p: Vec2) -> Option<Port> {
    let store = s.doc().ok()?.space(&s.space())?;
    devices(store, &s.csd_library)
        .iter()
        .filter_map(|d| {
            let i = d.insert;
            let (at, scale) = (i.insert.xy(), Vec2::new(i.scale.x, i.scale.y));
            let local = (p - at).rotate(-i.rotation);
            let inside = local.x.abs() <= CARD_W / 2.0 * scale.x.abs() + MARGIN && local.y.abs() <= CARD_H / 2.0 * scale.y.abs() + MARGIN;
            let (side, port, dist) = [Side::Top, Side::Bottom, Side::Left, Side::Right]
                .into_iter()
                .map(|side| {
                    let port = port_point(at, i.rotation, scale, side, Slot::SOLO);
                    (side, port, (port - p).len())
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))?;
            inside.then_some(((d.handle, side, port), dist))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(hit, _)| hit)
}

/// A `CSD_` card under `p` whose product the library lacks.
fn on_unknown_card(s: &Session, p: Vec2) -> bool {
    let Some(store) = s.doc().ok().and_then(|d| d.space(&s.space())) else { return false };
    store.iter().any(|e| match &e.kind {
        EntityKind::Insert(i) => {
            let local = (p - i.insert.xy()).rotate(-i.rotation);
            super::is_device_block(i)
                && local.x.abs() <= CARD_W / 2.0 * i.scale.x.abs() + MARGIN
                && local.y.abs() <= CARD_H / 2.0 * i.scale.y.abs() + MARGIN
        }
        _ => false,
    })
}

pub(super) struct ConnectM {
    first: Option<Port>,
}

impl ConnectM {
    pub(super) fn start(_s: &Session) -> Result<Box<dyn Interactive>> {
        Ok(Box::new(ConnectM { first: None }))
    }
}

impl Interactive for ConnectM {
    fn name(&self) -> &'static str {
        "CSD.CONNECT"
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let which = if self.first.is_some() { "second" } else { "first" };
        let ids: Vec<&str> = Protocol::ALL.iter().map(|p| p.id()).collect();
        let p = Prompt::new(format!("Select {which} device port [Protocol: {}]", s.csd_protocol.label()), Accept::POINT).kw(&ids);
        p.base_opt(self.first.map(|(_, _, port)| port))
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(p) => {
                let Some(hit) = pick(s, p) else {
                    s.echo(if on_unknown_card(s, p) { "That card's product is not in the library." } else { "No device there." });
                    return Ok(Step::Continue);
                };
                let Some((from, from_side, _)) = self.first.take() else {
                    self.first = Some(hit);
                    return Ok(Step::Continue);
                };
                let wire = json!({ "from": from.hex(), "fromSide": from_side.id(), "to": hit.0.hex(), "toSide": hit.1.id(), "protocol": s.csd_protocol.id() });
                if let Err(e) = run_connect(s, &wire) {
                    s.echo(e.to_string());
                }
            }
            Input::Keyword(t) | Input::Text(t) => match Protocol::from_id(&t.trim().to_lowercase()) {
                Some(p) => s.csd_protocol = p,
                None => s.echo("Invalid option keyword."),
            },
            Input::Enter | Input::Cancel => return Ok(Step::Done),
            _ => {}
        }
        Ok(Step::Continue)
    }
    fn preview(&self, _s: &Session, c: Vec2) -> Vec<EntityKind> {
        self.first.iter().map(|(_, _, port)| line(*port, c)).collect()
    }
}
