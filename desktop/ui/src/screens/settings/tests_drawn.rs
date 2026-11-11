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

/// Darstellung in English: the language section names itself and its
/// options in the interface's language, each option in its own.
#[test]
fn the_language_section_follows_the_interface_language() {
    within(30, || {
        use gelabber_i18n::Locale;
        let _config = crate::platform::paths::testing::TempConfig::new("language-english");
        let _locale = gelabber_i18n::on_this_thread(Locale::De);
        let (mut h, world, _client, _call) =
            harness("/settings/darstellung", settings_page("darstellung"));
        {
            let mut world = world.borrow_mut();
            let mut user = world.session.user().cloned().unwrap();
            user["locale"] = "de".into();
            world.session.adopt(&serde_json::json!({ "user": user }));
        }
        h.run_steps(4);
        assert!(h.query_by_label("Sprache").is_some());
        assert!(h.query_by_label("Language").is_none());
        gelabber_i18n::set(Locale::En);
        h.run_steps(4);
        assert!(h.query_by_label("Language").is_some());
        assert!(h.query_by_label("Sprache").is_none());
        // Each language is named in its own language on its option.
        assert!(h.query_by_label("Deutsch").is_some());
        assert!(h.query_by_label("English").is_some());
    });
}
