//! The sign-in card: covers the app until the account service signs in, and Sign Out brings it back.

use std::sync::{Arc, Mutex};

use cadcraft_engine::Session;
use cadcraft_ui_egui::{Account, AccountError, CadApp, Services, SignedIn};
use egui::{Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

#[derive(Default)]
struct Fake {
    resume: Option<SignedIn>,
    sign_in: Option<Result<SignedIn, AccountError>>,
    request: Option<Result<(), AccountError>>,
    signed_out: Mutex<bool>,
}

fn me() -> SignedIn {
    let product = json!({"id": "p1", "name": "Spot 1", "code": "S1", "cat": "Moving Head", "brand": "Acme"});
    SignedIn { email: "a@b.c".into(), products: vec![product] }
}

impl Account for Fake {
    fn resume(&self) -> Result<Option<SignedIn>, AccountError> {
        Ok(self.resume.clone())
    }
    fn sign_in(&self, _: &str, _: &str) -> Result<SignedIn, AccountError> {
        self.sign_in.clone().unwrap_or(Err(AccountError::Invalid))
    }
    fn request_access(&self, _: &str, _: &str) -> Result<(), AccountError> {
        self.request.clone().unwrap_or(Ok(()))
    }
    fn sign_out(&self) {
        *self.signed_out.lock().unwrap() = true;
    }
}

type H = Harness<'static, CadApp>;

fn harness(account: Option<Fake>) -> H {
    let services = Services { account: account.map(|a| Arc::new(a) as Arc<dyn Account>), ..Services::default() };
    let mut app = CadApp::new(Session::default(), services);
    app.ui.start_tab = false;
    Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut CadApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    )
}

/// Step frames until `done`, so a worker thread's answer has time to arrive.
fn settle(h: &mut H, done: impl Fn(&H) -> bool) {
    for _ in 0..200 {
        h.step();
        if done(h) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn has_card(h: &H) -> bool {
    h.query_by_label("Sign in with your ChamSys Designer account").is_some()
}

fn form_shown(h: &H) -> bool {
    h.query_by_role_and_label(Role::PasswordInput, "PASSWORD").is_some()
}

fn fill(h: &mut H) {
    h.get_by_role_and_label(Role::TextInput, "EMAIL").click();
    h.get_by_role_and_label(Role::TextInput, "EMAIL").type_text("a@b.c");
    h.get_by_role_and_label(Role::PasswordInput, "PASSWORD").click();
    h.get_by_role_and_label(Role::PasswordInput, "PASSWORD").type_text("pw");
    h.step();
}

fn products(h: &mut H) -> usize {
    h.state_mut().run("csd.library.list", json!({})).unwrap()["products"].as_array().map_or(0, Vec::len)
}

#[test]
fn without_an_account_there_is_no_card() {
    let mut h = harness(None);
    h.run_steps(5);
    assert!(!has_card(&h));
}

fn file_menu_opens(account: Option<Fake>) -> bool {
    let mut h = harness(account);
    h.state_mut().ui.in_window_menu = true;
    h.run_steps(3);
    if let Some(file) = h.query_by_label("File") {
        file.click();
    }
    h.run_steps(3);
    h.query_by_label("New from Template...").is_some()
}

#[test]
fn the_card_covers_the_app_and_blocks_it() {
    assert!(file_menu_opens(None), "the menu opens without the card");
    let mut h = harness(Some(Fake::default()));
    settle(&mut h, form_shown);
    assert!(has_card(&h));
    assert!(!file_menu_opens(Some(Fake::default())));
}

#[test]
fn a_stored_session_goes_straight_in() {
    let mut h = harness(Some(Fake { resume: Some(me()), ..Fake::default() }));
    settle(&mut h, |h| !has_card(h));
    assert!(!has_card(&h));
    assert_eq!(products(&mut h), 1);
}

#[test]
fn signing_in_loads_the_catalogue_and_removes_the_card() {
    let mut h = harness(Some(Fake { sign_in: Some(Ok(me())), ..Fake::default() }));
    settle(&mut h, form_shown);
    fill(&mut h);
    h.get_by_label("Sign in").click();
    settle(&mut h, |h| !has_card(h));
    assert!(!has_card(&h));
    assert_eq!(products(&mut h), 1);
}

#[test]
fn a_wrong_password_says_so_and_keeps_the_card() {
    let mut h = harness(Some(Fake::default()));
    settle(&mut h, form_shown);
    fill(&mut h);
    h.get_by_label("Sign in").click();
    settle(&mut h, |h| h.query_by_label("Email or password is incorrect.").is_some());
    assert!(h.query_by_label("Email or password is incorrect.").is_some());
    assert!(has_card(&h));
}

#[test]
fn request_access_says_the_request_was_sent() {
    let mut h = harness(Some(Fake::default()));
    settle(&mut h, form_shown);
    fill(&mut h);
    h.get_by_label("Request access").click();
    settle(&mut h, |h| h.query_by_label("Request sent. An admin will approve it.").is_some());
    assert!(h.query_by_label("Request sent. An admin will approve it.").is_some());
}

#[test]
fn sign_out_brings_the_card_back() {
    let mut h = harness(Some(Fake { resume: Some(me()), ..Fake::default() }));
    settle(&mut h, |h| !has_card(h));
    h.state_mut().run("ui.signout", json!(null)).unwrap();
    settle(&mut h, form_shown);
    assert!(has_card(&h));
}

#[test]
fn sign_out_is_refused_when_nobody_is_signed_in() {
    let mut h = harness(Some(Fake::default()));
    settle(&mut h, form_shown);
    assert!(h.state_mut().run("ui.signout", json!(null)).is_err());
    let mut h = harness(None);
    assert!(h.state_mut().run("ui.signout", json!(null)).is_err());
}
