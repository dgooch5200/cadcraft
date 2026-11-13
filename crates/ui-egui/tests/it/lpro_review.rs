//! Import Luminary's review window: each fixture type with its quantity and product, an unsure
//! match flagged, and Import adding what the rows say to the unplaced list.

use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, Services, dialogs, lproreview};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const PH: &str = r#""vAngles":[0],"hAngles":[0],"candela":[[1]]"#;

#[test]
fn review_lists_types_flags_unsure_ones_and_imports_the_picks() {
    let file = format!(
        r#"{{"fixtureTypes":[{{"id":"a","name":"Drive","photometry":{{{PH},"meta":{{"LUMINAIRE":"EPIX Flex Drive"}}}}}},{{"id":"b","name":"zzz","photometry":{{{PH}}}}}],
        "fixtures":[{{"typeId":"a","x":0,"y":0}},{{"typeId":"a","x":1,"y":0}},{{"typeId":"b","x":1,"y":0}}]}}"#
    );
    let path = std::env::temp_dir().join(format!("cadcraft-lproreview-{}.lpro", std::process::id()));
    std::fs::write(&path, file).unwrap();
    let mut app = CadApp::new(Session::default(), Services::default());
    app.ui.dialog = Some(format!("{}{}", lproreview::DIALOG, path.to_str().unwrap()));
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("Fixture type");
    h.get_by_label("Drive");
    h.get_by_label("\u{26a0} zzz");
    h.get_by_label("Import").click();
    h.run_steps(3);
    let _ = std::fs::remove_file(&path);
    let app = h.state_mut();
    assert!(app.ui.dialog.is_none(), "the window closes after importing");
    let items = app.run("csd.unplaced.list", json!({})).unwrap()["items"].clone();
    let got: Vec<_> = items.as_array().unwrap().iter().map(|i| (i["product"].clone(), i["qty"].clone())).collect();
    assert_eq!(got, vec![(json!("epix-flex-drive"), json!(2))], "{items}");
}
