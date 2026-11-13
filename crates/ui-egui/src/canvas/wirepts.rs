//! Waypoints of the selected lighting cable: squares to drag (one `csd.wire.point.move` on
//! release) and double-click to remove. The cable's own vertex grips are hidden, as the
//! post-command pass would re-route any edit made through them.

use serde_json::json;

use super::{CadApp, Handle, Pos2, Rect, Vec2, Xf, vec2};
use crate::context_menu::wire_link;

/// Half the side of a waypoint square, and how far from its centre a click still counts, in points.
const HALF: f32 = 4.0;
const PICK: f32 = 7.0;

/// The one selected cable and its waypoints.
fn selected(app: &CadApp) -> Option<(Handle, Vec<Vec2>)> {
    let [h] = app.session.selection()[..] else { return None };
    Some((h, wire_link(app.session.doc().ok()?.entity(h)?)?.points))
}

/// The waypoint under `hover`: (cable, index).
pub(super) fn point_at(app: &CadApp, xf: &Xf, hover: Option<Pos2>) -> Option<(Handle, usize)> {
    let hp = hover?;
    if app.session.prompting() {
        return None;
    }
    let (h, pts) = selected(app)?;
    pts.iter().position(|p| xf.to_screen(*p).distance(hp) <= PICK).map(|i| (h, i))
}

pub(super) fn draw(app: &CadApp, painter: &egui::Painter, xf: &Xf, rect: Rect, t: &crate::theme::Tokens) {
    let Some((h, pts)) = selected(app) else { return };
    for (i, p) in pts.iter().enumerate() {
        let drag = app.canvas.wire_drag.filter(|d| d.0 == h && d.1 == i);
        let sp = xf.to_screen(drag.map_or(*p, |d| d.2));
        if rect.contains(sp) {
            let r = Rect::from_center_size(sp, vec2(HALF * 2.0, HALF * 2.0));
            painter.rect_filled(r, 0.0, if drag.is_some() { t.grip_hot } else { t.grip });
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.text), egui::StrokeKind::Middle);
        }
    }
}

/// Drag a waypoint, double-click to remove it. Call every frame with where the pointer is.
pub(super) fn interact(app: &mut CadApp, ui: &egui::Ui, resp: &egui::Response, xf: &Xf, hover: Option<Pos2>, double: bool) {
    let primary = egui::PointerButton::Primary;
    if double && let Some((w, i)) = point_at(app, xf, hover) {
        app.canvas.wire_drag = None;
        let _ = app.run("csd.wire.point.remove", json!({ "wire": w.hex(), "index": i }));
        return;
    }
    if resp.drag_started_by(primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some((w, i)) = point_at(app, xf, Some(origin))
        && let Some(p) = selected(app).and_then(|(_, pts)| pts.get(i).copied())
    {
        app.canvas.wire_drag = Some((w, i, p));
    }
    let Some((w, i, _)) = app.canvas.wire_drag else { return };
    let pos = ui.input(|i| i.pointer.interact_pos()).map(|p| xf.to_world(p));
    if resp.drag_stopped_by(primary) {
        app.canvas.wire_drag = None;
        if let Some(to) = pos
            && let Err(e) = app.run("csd.wire.point.move", json!({ "wire": w.hex(), "index": i, "to": [to.x, to.y] }))
        {
            app.session.echo(e);
        }
    } else if let Some(to) = pos {
        app.canvas.wire_drag = Some((w, i, to));
    }
}
