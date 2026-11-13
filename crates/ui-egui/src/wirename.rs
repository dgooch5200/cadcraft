//! The cable rename window behind the cable menu's Rename item (`wirename:<hex handle>`).

use serde_json::json;

use crate::CadApp;

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, spec: &str, open: &mut bool) {
    let hex = spec.strip_prefix(crate::context_menu::WIRE_NAME).unwrap_or_default();
    let sid = egui::Id::unique("wirename_text").with(hex);
    let saved: Option<String> = ctx.data_mut(|d| d.get_temp(sid));
    let first = saved.is_none();
    let mut text: String = saved.unwrap_or_else(|| {
        let link =
            cadcraft_doc::Handle::parse_hex(hex).and_then(|h| app.session.doc().ok()?.entity(h).and_then(|e| crate::context_menu::wire_link(e)));
        link.and_then(|l| l.name).unwrap_or_default()
    });
    let (mut keep, mut commit, mut cancel) = (*open, false, false);
    egui::Window::new(crate::tl!("Rename cable")).id(egui::Id::unique("Rename cable")).open(&mut keep).collapsible(false).resizable(false).show(
        ctx,
        |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(crate::tl!("Name")));
            if first {
                r.request_focus();
            }
            commit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                commit |= ui.button(crate::tl!("OK")).clicked();
                if ui.button(crate::tl!("Cancel")).clicked() {
                    cancel = true;
                }
            });
        },
    );
    keep &= !cancel;
    if commit {
        if let Err(e) = app.run("csd.wire.set", json!({ "wire": hex, "name": text })) {
            app.session.echo(e);
        }
        keep = false;
    }
    *open = keep;
    ctx.data_mut(|d| {
        if keep {
            d.insert_temp(sid, text);
        } else {
            d.remove_temp::<String>(sid);
        }
    });
}
