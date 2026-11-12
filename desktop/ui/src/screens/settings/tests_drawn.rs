//! The settings pages drawn in egui_kittest with a signed-in session and
//! an unreachable server.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use egui::{Ui, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gelabber_call::Call;
use gelabber_client::Client;

use crate::app::{AppCtx, Nav, Session};
use crate::images::Images;
use crate::toasts::Toasts;

pub struct Probe(Box<dyn FnMut(&mut Ui)>);

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        (self.0)(ui);
    }
}

pub type H = Harness<'static, Probe>;

pub const USER: &str = "6f1c1d2e-3a4b-4c5d-8e9f-0a1b2c3d4e5f";

/// The app's route and session, kept across frames.
pub struct World {
    pub nav: Nav,
    pub session: Session,
    pub toasts: Toasts,
}

/// `draw` each frame with an `AppCtx` whose session is signed in.
pub fn harness(
    href: &str,
    mut draw: impl FnMut(&mut Ui, &mut AppCtx) + 'static,
) -> (H, Rc<RefCell<World>>, Client, Call) {
    let client = Client::new("http://127.0.0.1:9").expect("client");
    let call = Call::new(client.clone());
    let images = Images::new(client.clone());
    let mut session = Session::new();
    session.adopt(&serde_json::json!({ "user": {
        "id": USER,
        "email": "lena@example.org",
        "name": "Lena",
        "avatar_url": null,
        "created_at": "2025-01-01T00:00:00Z",
        "bio": null,
        "pronouns": null,
        "status_text": null,
        "accent_color": null,
        "banner_color": null,
        "banner_url": null,
        "avatar_decoration": null,
    }}));
    let world = Rc::new(RefCell::new(World {
        nav: Nav::new(href),
        session,
        toasts: Toasts::default(),
    }));
    let shared = world.clone();
    let (c, k) = (client.clone(), call.clone());
    let h = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            crate::fonts::install(&cc.egui_ctx);
            crate::theme::set(&cc.egui_ctx, crate::theme::Theme::dark());
            Probe(Box::new(move |ui| {
                let mut w = shared.borrow_mut();
                let World {
                    nav,
                    session,
                    toasts,
                } = &mut *w;
                let mut app = AppCtx {
                    client: &c,
                    call: &k,
                    images: &images,
                    nav,
                    session,
                    account_mail: None,
                    toasts,
                    public_pages: None,
                };
                draw(ui, &mut app);
            }))
        });
    (h, world, client, call)
}

/// Runs `test` on its own thread and fails if it has not finished in
/// time: a hung UI thread fails the test instead of hanging the run.
pub fn within(secs: u64, test: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        test();
        let _ = tx.send(());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(()) => handle.join().expect("test thread"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(panic) = handle.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("the UI thread hung for {secs} s")
        }
    }
}

fn settings_page(area: &'static str) -> impl FnMut(&mut Ui, &mut AppCtx) {
    move |ui, app| {
        let a = app.nav.location().path.clone();
        let area_now = a.strip_prefix("/settings/").unwrap_or(area).to_owned();
        super::page(ui, app, Some(&area_now));
    }
}

/// "Weitere Themes" under Darstellung once locked the page's state twice.
#[test]
fn more_themes_leads_to_the_themes_area() {
    within(30, || {
        let _config = crate::platform::paths::testing::TempConfig::new("more-themes");
        let (mut h, world, _client, _call) =
            harness("/settings/darstellung", settings_page("darstellung"));
        h.run_steps(4);
        h.get_by_label("Weitere Themes").click();
        h.run_steps(4);
        assert_eq!(world.borrow().nav.location().path, "/settings/themes");
    });
}

/// While "Taste festlegen" records, the window's back and forward leave the
/// side mouse buttons alone (app.rs asks `recording_key`).
#[test]
fn recording_a_key_claims_the_side_buttons() {
    within(30, || {
        let _config = crate::platform::paths::testing::TempConfig::new("ptt-record");
        let (mut h, _world, _client, _call) = harness("/settings/audio", settings_page("audio"));
        super::prefs::update(&h.ctx, |p| p.voice_input = "ptt".into());
        h.run_steps(4);
        assert!(!super::media::recording_key(&h.ctx));
        h.get_by_label("Taste festlegen").click();
        h.run_steps(2);
        assert!(super::media::recording_key(&h.ctx));
        h.event(egui::Event::PointerButton {
            pos: egui::pos2(640.0, 400.0),
            button: egui::PointerButton::Extra1,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        // The frame that takes the button still counts as recording.
        assert!(super::media::recording_key(&h.ctx));
        h.run_steps(3);
        assert!(!super::media::recording_key(&h.ctx));
        assert_eq!(super::prefs::get(&h.ctx).ptt_key, "Mouse4");
    });
}

/// Audio or Video on the page with the call's "Alle Einstellungen …" over
/// it: two copies of one form in a frame, each with ids of its own (debug
/// builds panicked on the shared ones, release ones mixed the input).
#[test]
fn the_media_dialog_over_its_own_page_keeps_its_controls_apart() {
    use crate::screens::voice::dialogs::{self, Tab};
    for (area, tab) in [("audio", Tab::Audio), ("video", Tab::Video)] {
        within(30, move || {
            let _config = crate::platform::paths::testing::TempConfig::new("media-twice");
            let mut page = settings_page(area);
            let (mut h, _world, _client, _call) =
                harness(&format!("/settings/{area}"), move |ui, app| {
                    page(ui, app);
                    dialogs::show(ui, app);
                });
            super::prefs::update(&h.ctx, |p| {
                p.voice_input = "ptt".into();
                p.video_upload_limit = 10_000_000.0;
            });
            h.run_steps(3);
            dialogs::open_settings(&h.ctx, tab);
            h.run_steps(6);
            assert!(dialogs::open(&h.ctx));
        });
    }
}

/// Darstellung in English: the settings navigation names the area
/// "Appearance", and the language section names itself and its options in
/// the interface's language, each option in its own.
#[test]
fn the_language_section_follows_the_interface_language() {
    within(30, || {
        use gelabber_i18n::Locale;
        let _config = crate::platform::paths::testing::TempConfig::new("language-english");
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        // The navigation column beside the page, as the shell lays them out.
        let mut page = settings_page("darstellung");
        let (mut h, world, _client, _call) = harness("/settings/darstellung", move |ui, app| {
            let column = ui.max_rect();
            let (nav, rest) = column.split_left_right_at_x(column.left() + 240.0);
            ui.scope_builder(egui::UiBuilder::new().max_rect(nav), |ui| {
                super::sidebar(ui, app, Some("darstellung"));
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(rest), |ui| page(ui, app));
        });
        {
            let mut world = world.borrow_mut();
            let mut user = world.session.user().cloned().unwrap();
            user["locale"] = "de".into();
            world.session.adopt(&serde_json::json!({ "user": user }));
        }
        h.run_steps(4);
        assert!(h.query_by_label("Darstellung").is_some());
        assert!(h.query_by_label("Sprache").is_some());
        assert!(h.query_by_label("Language").is_none());
        gelabber_i18n::set(Locale::En);
        h.run_steps(4);
        assert!(h.query_by_label("Appearance").is_some());
        assert!(h.query_by_label("Darstellung").is_none());
        assert!(h.query_by_label("Language").is_some());
        assert!(h.query_by_label("Sprache").is_none());
        // The area's controls speak English too.
        assert!(h.query_by_label("More themes").is_some());
        assert!(h.query_by_label("Weitere Themes").is_none());
        // Each language is named in its own language on its option.
        assert!(h.query_by_label("Deutsch").is_some());
        assert!(h.query_by_label("English").is_some());
    });
}

// ------------------------------------------------- Sicherheit: Anmeldung

fn sign_in_methods(password: bool, connected: bool) -> gelabber_client::SignInMethods {
    serde_json::from_value(serde_json::json!({
        "password": password,
        "providers": [{
            "id": "google", "name": "Google", "configured": true,
            "connected": connected.then(|| serde_json::json!({
                "email": "lena@example.test", "since": "2026-11-10T10:00:00Z"
            }))
        }]
    }))
    .expect("methods")
}

/// Sicherheit with these sign-in methods.
fn security_with(methods: gelabber_client::SignInMethods) -> H {
    let (mut h, _world, _client, _call) = harness("/settings/konto", settings_page("konto"));
    h.run_steps(2);
    super::page_state(&h.ctx).lock().security.sign_in =
        super::sign_in::SignInState::with_methods(methods);
    h.run_steps(4);
    h
}

fn is_disabled(h: &H, label: &str) -> bool {
    use egui_kittest::kittest::NodeT;
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

/// The rows of hosted-security.png: Passwort with "Ändern", Google
/// connected with "Trennen"; Ändern opens today's form in a dialog.
#[test]
fn anmeldung_lists_the_password_and_a_connected_provider() {
    within(30, || {
        let _config = crate::platform::paths::testing::TempConfig::new("sign-in-connected");
        let mut h = security_with(sign_in_methods(true, true));
        assert!(h.query_by_label("Anmeldung").is_some());
        assert!(h.query_by_label("Passwort").is_some());
        assert!(
            h.query_by_label("Zum Anmelden mit deiner E-Mail-Adresse.")
                .is_some()
        );
        assert!(h.query_by_label("Google").is_some());
        assert!(
            h.query_by_label(
                "Verbunden mit lena@example.test. Trennen geht nur, solange ein Passwort gesetzt ist."
            )
            .is_some()
        );
        assert!(!is_disabled(&h, "Trennen"));
        // Today's form is gone from the page; it is in the dialog.
        assert!(h.query_by_label("Aktuelles Passwort").is_none());
        assert!(h.query_by_label("Auf allen Geräten abmelden").is_some());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Ändern")
            .click();
        h.run_steps(4);
        // The dialog's title and its submit button.
        assert_eq!(h.query_all_by_label("Passwort ändern").count(), 2);
        assert!(h.query_by_label("Aktuelles Passwort").is_some());
        assert!(h.query_by_label("Neues Passwort").is_some());
        // "Trennen" asks first.
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Abbrechen")
            .click();
        h.run_steps(4);
        assert!(h.query_by_label("Aktuelles Passwort").is_none());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Trennen")
            .click();
        h.run_steps(4);
        assert!(h.query_by_label("Google trennen?").is_some());
    });
}

/// Not connected: "Verbinden", which with a password asks for it first.
#[test]
fn connecting_with_a_password_asks_for_it() {
    within(30, || {
        let _config = crate::platform::paths::testing::TempConfig::new("sign-in-open");
        let mut h = security_with(sign_in_methods(true, false));
        assert!(h.query_by_label("Nicht verbunden.").is_some());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Verbinden")
            .click();
        h.run_steps(4);
        assert!(h.query_by_label("Google verbinden").is_some());
        assert!(h.query_by_label("Aktuelles Passwort").is_some());
        // Without it the dialog says what is missing and goes nowhere.
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Weiter zu Google")
            .click();
        h.run_steps(4);
        assert!(h.query_by_label("Google verbinden").is_some());
        assert!(h.query_by_label("Weiter zu Google…").is_none());
    });
}

/// Google only: "Festlegen" names the provider, and "Trennen" is off
/// until a password is set.
#[test]
fn without_a_password_trennen_waits_for_festlegen() {
    within(30, || {
        let _config = crate::platform::paths::testing::TempConfig::new("sign-in-google-only");
        let mut h = security_with(sign_in_methods(false, true));
        assert!(
            h.query_by_label("Noch keins gesetzt. Du meldest dich mit Google an.")
                .is_some()
        );
        assert!(is_disabled(&h, "Trennen"));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Festlegen")
            .click();
        h.run_steps(4);
        // The dialog's title and its submit button.
        assert_eq!(h.query_all_by_label("Passwort festlegen").count(), 2);
        assert!(h.query_by_label("Aktuelles Passwort").is_none());
        assert!(h.query_by_label("Neues Passwort").is_some());
    });
}

/// Against the stand-in API: the methods load, "Trennen" confirmed
/// disconnects, and the row says so.
#[test]
fn disconnecting_a_provider_after_the_question() {
    use crate::app::testkit::{FakeApi, Reply, launcher, wait_for_label, workspace};
    let _config = crate::platform::paths::testing::TempConfig::new("sign-in-disconnect");
    let api = FakeApi::start(true, |method, path, _| match (method, path) {
        ("GET", "/auth/methods") => Some(Reply::json(
            serde_json::to_value(sign_in_methods(true, true)).expect("json"),
        )),
        ("DELETE", "/auth/oidc/google") => Some(Reply::json(
            serde_json::to_value(sign_in_methods(true, false)).expect("json"),
        )),
        _ => workspace(method, path),
    });
    let mut h = launcher(&api, "/settings/konto");
    wait_for_label(&mut h, "Trennen");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Trennen")
        .click();
    wait_for_label(&mut h, "Google trennen?");
    super::controls::answer_for_test(&h.ctx, "disconnect-provider-google", true);
    wait_for_label(&mut h, "Getrennt.");
    assert!(api.saw("DELETE /auth/oidc/google"));
    wait_for_label(&mut h, "Nicht verbunden.");
    wait_for_label(&mut h, "Verbinden");
}
