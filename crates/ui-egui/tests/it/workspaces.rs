//! WSCURRENT: Simple and Advanced keep their own panel layouts and every command works in both.

use cadcraft_engine::Session;
use cadcraft_ui_egui::docking::{Panel, WorkspaceName};
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::json;

fn app() -> CadApp {
    CadApp::new(Session::default(), Services::default())
}

fn advanced() -> CadApp {
    let mut app = app();
    app.run("wscurrent", json!({"name": "Advanced"})).unwrap();
    app
}

#[test]
fn a_fresh_profile_starts_in_simple_with_tool_sets_and_no_palettes() {
    let mut app = app();
    assert_eq!(app.ui.workspace, WorkspaceName::default());
    assert_eq!(app.ui.workspace, WorkspaceName::Simple);
    let layout = &app.ui.docking.layout;
    assert!(layout.contains(&Panel::Canvas) && layout.contains(&Panel::ToolSets));
    assert!(!layout.contains(&Panel::Layers) && !layout.contains(&Panel::Properties) && !layout.contains(&Panel::ExternalRefs));
    assert!(app.ui.show_toolsets && !app.ui.show_palettes);
    // Commands still run.
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
    // The palettes are one command away.
    app.run("ui.panel.layers", json!(null)).unwrap();
    assert!(app.ui.docking.layout.contains(&Panel::Layers));
}

#[test]
fn advanced_has_the_palettes() {
    let app = advanced();
    assert!(app.ui.docking.layout.contains(&Panel::Layers) && app.ui.docking.layout.contains(&Panel::Properties));
    assert!(app.ui.show_palettes);
}

#[test]
fn switching_back_restores_the_rearranged_layout() {
    let mut app = advanced();
    app.run("ui.dock", json!({"operation":"float", "panel":"layers", "rect":[120,90,340,410]})).unwrap();
    let rearranged = app.ui.docking.clone();
    app.run("wscurrent", json!({"name": "simple"})).unwrap();
    assert!(!app.ui.docking.layout.contains(&Panel::Layers));
    app.run("wscurrent", json!({"name": "ADVANCED"})).unwrap();
    assert_eq!(app.ui.docking, rearranged);
    assert!(app.ui.show_palettes);
}

#[test]
fn unknown_name_errors_and_changes_nothing() {
    let mut app = advanced();
    let before = app.ui.docking.clone();
    assert!(app.run("wscurrent", json!({"name": "Expert"})).is_err());
    assert_eq!((app.ui.workspace, &app.ui.docking), (WorkspaceName::Advanced, &before));
}

#[test]
fn typed_command_switches_and_reports() {
    let mut app = advanced();
    assert_eq!(app.cmdline("wscurrent Simple"), None);
    assert_eq!(app.ui.workspace, WorkspaceName::Simple);
    assert_eq!(app.cmdline("wscurrent"), None);
    assert_eq!(app.ui.workspace, WorkspaceName::Simple);
    app.cmdline("wscurrent nope");
    assert_eq!(app.ui.workspace, WorkspaceName::Simple);
    assert_eq!(app.run("ui.workspace.advanced", json!(null)).unwrap(), json!({"workspace": "Advanced"}));
}

#[test]
fn reset_panel_layout_resets_the_current_workspace() {
    let mut app = advanced();
    app.run("ui.dock.reset", json!(null)).unwrap();
    assert_eq!(app.ui.docking, WorkspaceName::Advanced.default_layout());
    app.run("wscurrent", json!({"name": "Simple"})).unwrap();
    app.run("ui.dock.reset", json!(null)).unwrap();
    assert_eq!(app.ui.docking, WorkspaceName::Simple.default_layout());
}

#[test]
fn prefs_round_trip_the_workspace_name() {
    let mut a = advanced();
    let mut b = app();
    b.load_prefs(&a.prefs_json());
    assert_eq!(b.ui.workspace, WorkspaceName::Advanced);
    b.load_prefs(r#"{"workspace":"bogus"}"#);
    assert_eq!(b.ui.workspace, WorkspaceName::Advanced);
    a.run("wscurrent", json!({"name": "Simple"})).unwrap();
    b.load_prefs(&a.prefs_json());
    assert_eq!(b.ui.workspace, WorkspaceName::Simple);
}
