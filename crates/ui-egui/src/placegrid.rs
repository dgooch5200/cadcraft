//! The Place Grid window behind the canvas menu's "Place grid here..." (`placegrid:<x>,<y>`).

use serde_json::json;

use crate::CadApp;

/// Rows and columns last asked for, kept for the next time the window opens.
type Size = (u32, u32);

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, spec: &str, open: &mut bool) {
    let at = spec
        .strip_prefix(crate::context_menu::PLACE_GRID)
        .and_then(|s| s.split_once(','))
        .and_then(|(x, y)| Some([x.parse::<f64>().ok()?, y.parse::<f64>().ok()?]));
    let sid = egui::Id::unique("placegrid_size");
    let (mut rows, mut cols): Size = ctx.data_mut(|d| d.get_temp(sid)).unwrap_or((2, 2));
    let (mut keep, mut commit, mut cancel) = (*open, false, false);
    egui::Window::new(crate::tl!("Place grid")).id(egui::Id::unique("Place grid")).open(&mut keep).collapsible(false).resizable(false).show(
        ctx,
        |ui| {
            egui::Grid::new("placegrid_fields").show(ui, |ui| {
                ui.label(crate::tl!("Rows"));
                ui.add(egui::DragValue::new(&mut rows).range(1..=400));
                ui.end_row();
                ui.label(crate::tl!("Columns"));
                ui.add(egui::DragValue::new(&mut cols).range(1..=400));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                commit = ui.button(crate::tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.button(crate::tl!("Cancel")).clicked();
            });
        },
    );
    keep &= !cancel;
    if commit {
        if let (Some(at), Some(product)) = (at, app.session.csd_armed.clone())
            && let Err(e) = app.run("csd.place.grid", json!({ "product": product, "at": at, "rows": rows, "cols": cols }))
        {
            app.session.echo(e);
        }
        keep = false;
    }
    ctx.data_mut(|d| d.insert_temp(sid, (rows, cols)));
    *open = keep;
}
