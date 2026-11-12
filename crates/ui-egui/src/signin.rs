//! The ChamSys Designer sign-in card. Where the host gave an [`Account`](crate::Account) the card
//! covers the whole window until someone is signed in, and the product catalogue it returns goes
//! into the library. Account calls block on the network, so each runs on its own thread and
//! answers through a channel polled every frame.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use egui::{Color32, Frame, Key, RichText, Stroke, TextEdit, vec2};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{AccountError, CadApp, SignedIn};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// The stored session has not been tried yet.
    #[default]
    Start,
    Resuming,
    Form,
    SignedIn,
}

enum Reply {
    Session(Result<Option<SignedIn>, AccountError>),
    Requested(Result<(), AccountError>),
}

/// What the card says under the form.
enum Note {
    Error(AccountError),
    /// The library refused the catalogue.
    Load(String),
    Requested,
}

#[derive(Default)]
pub struct State {
    phase: Phase,
    email: String,
    password: String,
    note: Option<Note>,
    pending: Option<Receiver<Reply>>,
    focus_email: bool,
}

/// Whether the card is up.
pub fn gated(app: &CadApp) -> bool {
    app.services.account.is_some() && app.signin.phase != Phase::SignedIn
}

/// Whether someone is signed in (what Sign Out needs).
pub fn signed_in(app: &CadApp) -> bool {
    app.services.account.is_some() && app.signin.phase == Phase::SignedIn
}

fn start(st: &mut State, work: impl FnOnce() -> Reply + Send + 'static) {
    let (tx, rx) = channel();
    // A thread that cannot start drops `tx`, which `tick` reports as a failed request.
    let _ = std::thread::Builder::new().spawn(move || {
        let _ = tx.send(work());
    });
    st.pending = Some(rx);
}

/// Start the resume on the first frame and take in an answer when it arrives. Runs every frame.
pub fn tick(app: &mut CadApp, ctx: &egui::Context) {
    let Some(account) = app.services.account.clone() else { return };
    if app.signin.phase == Phase::Start {
        app.signin.phase = Phase::Resuming;
        start(&mut app.signin, move || Reply::Session(account.resume()));
    }
    let Some(rx) = &app.signin.pending else { return };
    let reply = match rx.try_recv() {
        Ok(reply) => reply,
        Err(TryRecvError::Empty) => return ctx.request_repaint_after(Duration::from_millis(100)),
        Err(TryRecvError::Disconnected) => Reply::Session(Err(AccountError::Server("the request did not finish".into()))),
    };
    app.signin.pending = None;
    match reply {
        Reply::Session(Ok(Some(signed_in))) => load(app, signed_in),
        Reply::Session(Ok(None)) => form(&mut app.signin, None),
        Reply::Session(Err(e)) | Reply::Requested(Err(e)) => form(&mut app.signin, Some(Note::Error(e))),
        Reply::Requested(Ok(())) => form(&mut app.signin, Some(Note::Requested)),
    }
    ctx.request_repaint();
}

/// Show the form, with the email field focused when it comes up fresh.
fn form(st: &mut State, note: Option<Note>) {
    st.focus_email |= st.phase == Phase::Resuming;
    st.phase = Phase::Form;
    st.note = note;
}

/// Hand the catalogue to the library; the card goes once it is in.
fn load(app: &mut CadApp, signed_in: SignedIn) {
    match app.run("csd.library.set", json!({ "products": signed_in.products })) {
        Ok(_) => {
            app.signin.phase = Phase::SignedIn;
            app.signin.note = None;
            app.signin.password.clear();
        }
        Err(e) => form(&mut app.signin, Some(Note::Load(e))),
    }
}

/// `ui.signout`: end the session and bring the card back.
pub fn sign_out(app: &mut CadApp) -> Result<Value, String> {
    let Some(account) = app.services.account.clone().filter(|_| signed_in(app)) else {
        return Err("Not signed in.".into());
    };
    let _ = std::thread::Builder::new().spawn(move || account.sign_out());
    app.signin.password.clear();
    app.signin.focus_email = true;
    form(&mut app.signin, None);
    Ok(Value::Null)
}

fn message(note: &Note) -> String {
    match note {
        Note::Error(AccountError::Invalid) => crate::tl!("Email or password is incorrect.").into(),
        Note::Error(AccountError::Pending) => crate::tl!("Your account is waiting for an admin to approve it.").into(),
        Note::Error(AccountError::Disabled) => crate::tl!("This account is disabled.").into(),
        Note::Error(AccountError::Exists) => crate::tl!("That email already has an account.").into(),
        Note::Error(AccountError::Server(m)) => crate::tf!("Couldn't reach the server: {m}", m = m),
        Note::Load(e) => e.clone(),
        Note::Requested => crate::tl!("Request sent. An admin will approve it.").into(),
    }
}

/// The card and the dimmed backdrop that blocks the app behind it.
pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let Some(account) = app.services.account.clone().filter(|_| gated(app)) else { return };
    let t = Tokens::get();
    let st = &mut app.signin;
    let busy = st.pending.is_some();
    let card = Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(24).shadow(egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(90),
    });
    let (mut request, mut submit) = (false, false);
    crate::modal::instant(ctx, egui::Id::unique("signin_card")).frame(card).backdrop_color(Color32::from_black_alpha(150)).show(ctx, |ui| {
        ui.set_width(272.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        // The light theme's fields are as pale as the card.
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
        ui.label(RichText::new(crate::tl!("CADCraft")).size(18.0).strong());
        ui.label(RichText::new(crate::tl!("Sign in with your ChamSys Designer account")).size(12.0).color(t.text_dim));
        ui.add_space(16.0);
        if st.phase == Phase::Resuming {
            ui.label(RichText::new(crate::tl!("Signing in…")).color(t.text_dim));
            return;
        }
        let label = |ui: &mut egui::Ui, text: &str| ui.label(RichText::new(text).size(10.5).color(t.text_dim));
        let mut enter = false;
        ui.add_enabled_ui(!busy, |ui| {
            label(ui, crate::tl!("EMAIL"));
            let email =
                ui.add(TextEdit::singleline(&mut st.email).desired_width(f32::INFINITY).margin(vec2(6.0, 5.0))).accessible_name(crate::tl!("EMAIL"));
            if std::mem::take(&mut st.focus_email) {
                email.request_focus();
            }
            ui.add_space(6.0);
            label(ui, crate::tl!("PASSWORD"));
            let password = ui
                .add(TextEdit::singleline(&mut st.password).password(true).desired_width(f32::INFINITY).margin(vec2(6.0, 5.0)))
                .accessible_name(crate::tl!("PASSWORD"));
            enter = (email.lost_focus() || password.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
        });
        ui.add_space(8.0);
        if busy {
            ui.label(RichText::new(crate::tl!("Signing in…")).color(t.text_dim));
        } else if let Some(note) = &st.note {
            let color = if matches!(note, Note::Requested) { t.text } else { t.danger };
            ui.label(RichText::new(message(note)).size(12.0).color(color));
        }
        ui.add_space(8.0);
        let wide = vec2(ui.available_width(), 28.0);
        ui.add_enabled_ui(!busy, |ui| {
            let primary = egui::Button::new(RichText::new(crate::tl!("Sign in")).color(Color32::WHITE)).fill(t.accent).min_size(wide);
            submit = ui.add(primary).clicked() || enter;
            request = ui.add(egui::Button::new(crate::tl!("Request access")).min_size(wide)).clicked();
        });
    });
    let (email, password) = (st.email.trim().to_string(), st.password.clone());
    if submit {
        st.note = None;
        start(st, move || Reply::Session(account.sign_in(&email, &password).map(Some)));
    } else if request {
        st.note = None;
        start(st, move || Reply::Requested(account.request_access(&email, &password)));
    }
}
