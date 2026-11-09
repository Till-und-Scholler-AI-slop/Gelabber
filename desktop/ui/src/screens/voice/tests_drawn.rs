//! The call bar, the stage's tiles and the overview drawn in egui_kittest
//! against hand-made call states (gelabber_call's `testing` setter) and an
//! unreachable server.

use egui::{Rect, Ui, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gelabber_call::{Call, CallState, CallStatus, LiveSound, MicStatus, Participant, VideoKey};
use gelabber_client::Client;

use super::{JONAS, LENA, SERVER, SOFA};
use crate::app::{AppCtx, Nav, Session};
use crate::images::Images;
use crate::screens::voice::session::{Person, Picture};
use crate::screens::voice::stage::{Kind, PicId};
use crate::screens::voice::{call_bar, tile, window};
use crate::toasts::Toasts;

fn offline() -> (Client, Call) {
    let client = Client::new("http://127.0.0.1:9").expect("client");
    let call = Call::new(client.clone());
    (client, call)
}

/// Draws a closure as the app would: its fonts and theme are in before
/// the first frame (as `GelabberApp::new` installs them).
pub struct Probe(Box<dyn FnMut(&mut Ui)>);

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        (self.0)(ui);
    }
}

type H = Harness<'static, Probe>;

/// `draw` with an `AppCtx` each frame.
fn harness(client: Client, call: Call, draw: impl FnMut(&mut Ui, &mut AppCtx) + 'static) -> H {
    harness_at(1.0, client, call, draw)
}

/// The same at `ppp` device pixels per point (desktop scale × text size).
fn harness_at(
    ppp: f32,
    client: Client,
    call: Call,
    mut draw: impl FnMut(&mut Ui, &mut AppCtx) + 'static,
) -> H {
    let images = Images::new(client.clone());
    Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(ppp)
        .build_eframe(move |cc| {
            crate::fonts::install(&cc.egui_ctx);
            crate::theme::set(&cc.egui_ctx, crate::theme::Theme::dark());
            Probe(Box::new(move |ui| {
                let mut nav = Nav::new("/");
                let mut session = Session::new();
                let mut toasts = Toasts::default();
                let mut app = AppCtx {
                    client: &client,
                    call: &call,
                    images: &images,
                    nav: &mut nav,
                    session: &mut session,
                    account_mail: None,
                    toasts: &mut toasts,
                };
                draw(ui, &mut app);
            }))
        })
}

fn bar(ui: &mut Ui, app: &mut AppCtx) {
    call_bar::show(
        ui,
        app,
        Rect::from_min_size(pos2(288.0, 744.0), vec2(992.0, 56.0)),
    );
}

fn in_sofa(mic: MicStatus) -> CallState {
    CallState {
        status: CallStatus::Connected,
        server: Some(SERVER),
        channel: Some(SOFA),
        mic,
        participants: vec![Participant {
            user: LENA,
            me: true,
            ..Participant::default()
        }],
        ..CallState::default()
    }
}

#[test]
fn without_a_microphone_the_bar_says_why_and_retries() {
    let (client, call) = offline();
    call.set_state(in_sofa(MicStatus::Unavailable));
    let mut h = harness(client, call.clone(), bar);
    h.run_steps(6);
    assert!(
        h.query_by_label("Stummschalten").is_none(),
        "no mute for no microphone"
    );
    h.get_by_label("Kein Mikrofon").click();
    h.run_steps(6);
    h.get_by_label("Mikrofon nicht gefunden oder belegt.");
    h.get_by_label("Mikrofon wählen …");
    h.get_by_label("Erneut versuchen").click();
    h.run_steps(6);
    assert_eq!(call.microphone_retries(), 1);
    // A refused microphone points to the system's settings.
    call.set_state(in_sofa(MicStatus::Denied));
    h.get_by_label("Kein Mikrofon").click();
    h.run_steps(6);
    h.get_by_label("Kein Zugriff auf das Mikrofon. Erlaube ihn in den Systemeinstellungen.");
    // With one, the toggle is back.
    call.set_state(in_sofa(MicStatus::Live));
    h.run_steps(6);
    h.get_by_label("Stummschalten");
    assert!(h.query_by_label("Kein Mikrofon").is_none());
}

#[test]
fn a_lost_call_offers_to_rejoin_and_goes_away() {
    let (client, call) = offline();
    call.set_state(in_sofa(MicStatus::Live));
    call.simulate_media_loss();
    assert_eq!(call.state().lost_call, Some((SERVER, SOFA)));
    let mut h = harness(client, call.clone(), bar);
    h.run_steps(6);
    h.get_by_label("Verbindung verloren");
    h.get_by_label("Erneut versuchen").click();
    h.run_steps(6);
    assert_eq!(call.join_attempts(), 1);
    h.get_by_label("Schließen").click();
    h.run_steps(6);
    assert!(call.state().lost_call.is_none());
    assert!(h.query_by_label("Verbindung verloren").is_none());
}

fn person(id: uuid::Uuid, name: &str, me: bool) -> Person {
    Person {
        id,
        name: name.into(),
        avatar: None,
        me,
        member: true,
        muted: false,
        deafened: false,
        speaking: false,
        live: me,
        camera: !me,
    }
}

fn picture(user: uuid::Uuid, kind: Kind, label: &str, own: bool) -> Picture {
    Picture {
        id: PicId { user, kind },
        key: Some(match kind {
            Kind::Live => VideoKey::live(user),
            Kind::Camera => VideoKey::camera(user),
        }),
        label: label.into(),
        mirror: false,
        speaking: false,
        watching: false,
        own,
    }
}

fn lena() -> Person {
    person(LENA, "Lena", true)
}

fn own_live() -> Picture {
    picture(LENA, Kind::Live, "Lena (du) — Live", true)
}

fn stage_rect() -> Rect {
    Rect::from_min_size(pos2(312.0, 116.0), vec2(944.0, 556.0))
}

#[test]
fn the_own_live_says_what_becomes_of_its_sound() {
    let notes = [
        "Ton wird mitgesendet",
        "Kein Ton von „Spotify“",
        "Stream-Ton nicht aufgenommen · Video läuft weiter",
        "Stream-Ton beendet · Video läuft weiter",
    ];
    for (sound, words) in [
        (LiveSound::Sharing, Some(notes[0])),
        (LiveSound::Silent("Spotify".into()), Some(notes[1])),
        (
            LiveSound::Unavailable(Some("kein PipeWire".into())),
            Some(notes[2]),
        ),
        (LiveSound::Ended, Some(notes[3])),
        (LiveSound::Off, None),
    ] {
        let (client, call) = offline();
        call.set_state(CallState {
            live: true,
            live_sound: sound.clone(),
            ..in_sofa(MicStatus::Live)
        });
        let mut h = harness(client, call, |ui, app| {
            tile::picture(
                ui,
                app,
                stage_rect(),
                &own_live(),
                Some(&lena()),
                tile::Place::Grid,
                false,
                false,
                1.0,
            );
        });
        h.run_steps(6);
        for note in notes {
            assert_eq!(
                h.query_by_label(note).is_some(),
                words == Some(note),
                "{sound:?}: {note}"
            );
        }
    }
    // The bar's notices for the same statuses.
    let live = |sound| CallState {
        live: true,
        live_sound: sound,
        ..CallState::default()
    };
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Ended), "").as_deref(),
        Some("Der Stream-Ton wurde beendet. Das Video läuft weiter.")
    );
    // Switched on during a Live, the sound joins it: nothing waits for
    // the next one, as the desktop text "Gilt auch während du live bist" says.
    assert_eq!(call_bar::dock_notice(&live(LiveSound::Off), ""), None);
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Sharing), "").as_deref(),
        Some("Dein Stream sendet den Ton aller Anwendungen außer Gelabber.")
    );
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Sharing), "Spotify"),
        None
    );
}

#[test]
fn a_limited_picture_says_why() {
    let (label, reason) = tile::limit_badge(tile::VideoLimit::Renderer);
    assert_eq!(label, "Geringe Bildqualität");
    assert_eq!(
        reason,
        "WebGL ist hier nicht verfügbar. Das Bild bleibt deshalb klein."
    );
    let (_, reason) = tile::limit_badge(tile::VideoLimit::Transport);
    assert!(reason.contains("Content-Security-Policy"));
}

fn jonas_camera() -> (Person, Picture) {
    (
        person(JONAS, "Jonas", false),
        picture(JONAS, Kind::Camera, "Jonas", false),
    )
}

/// Jonas' camera on the stage, with the voice dialogs and windows drawn
/// as the shell does; `embed` stands for a backend without windows.
fn camera_stage(embed: bool) -> H {
    let (client, call) = offline();
    call.set_state(in_sofa(MicStatus::Live));
    let (jonas, camera) = jonas_camera();
    harness(client, call, move |ui, app| {
        ui.ctx().set_embed_viewports(embed);
        tile::picture(
            ui,
            app,
            stage_rect(),
            &camera,
            Some(&jonas),
            tile::Place::Grid,
            false,
            false,
            1.0,
        );
        crate::screens::voice::dialogs::show(ui, app);
    })
}

#[test]
fn a_video_opens_in_a_window_of_its_own() {
    let key = VideoKey::camera(JONAS);
    let mut h = camera_stage(false);
    h.run_steps(6);
    h.get_by_label("Weitere Aktionen").click();
    h.run_steps(6);
    h.get_by_label("Eigenes Fenster").click();
    h.run_steps(6);
    assert!(window::is_open(&h.ctx, key));
    let viewport = h
        .output()
        .viewport_output
        .get(&window::viewport_id(key))
        .expect("a viewport for the window");
    assert_eq!(viewport.builder.title.as_deref(), Some("Jonas – Gelabber"));
    // The tile makes way for it, with the way back.
    h.get_by_label("Läuft im eigenen Fenster.");
    h.get_by_label("Fenster schließen").click();
    h.run_steps(6);
    assert!(!window::is_open(&h.ctx, key));

    // A backend without windows: the tile says the window did not open.
    let mut h = camera_stage(true);
    h.run_steps(6);
    h.get_by_label("Weitere Aktionen").click();
    h.run_steps(6);
    h.get_by_label("Eigenes Fenster").click();
    h.run_steps(6);
    assert!(!window::is_open(&h.ctx, key));
    h.get_by_label(window::FAILED);
}

#[test]
fn a_server_that_did_not_load_offers_a_retry() {
    let (client, call) = offline();
    client.open_server(SERVER);
    let failed = |client: &Client| {
        client.with_state(|s| {
            matches!(
                s.op(&format!("server:{SERVER}")),
                Some(gelabber_client::OpState::Failed(_))
            )
        })
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !failed(&client) {
        assert!(std::time::Instant::now() < until, "the load did not fail");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut h = harness(client, call, |ui, app| {
        let rect = Rect::from_min_size(pos2(288.0, 0.0), vec2(744.0, 744.0));
        crate::screens::shell::overview::show(ui, app, rect, SERVER);
    });
    h.run_steps(6);
    h.get_by_label("Der Server konnte nicht geladen werden.");
    h.get_by_label("Erneut versuchen").click();
    h.step();
    // The retry runs with the error in place, then fails again.
    h.get_by_label("Der Server konnte nicht geladen werden.");
}

#[test]
fn an_unread_row_marks_a_draft() {
    let (client, call) = offline();
    let mut h = harness(client, call, |ui, _app| {
        let rect = Rect::from_min_size(pos2(380.0, 300.0), vec2(120.0, 52.0));
        crate::screens::shell::overview::unread_name(ui, rect, "musik", true);
        let other = Rect::from_min_size(pos2(380.0, 360.0), vec2(120.0, 52.0));
        crate::screens::shell::overview::unread_name(ui, other, "spiele", false);
    });
    h.run_steps(6);
    assert_eq!(h.query_all_by_label("Entwurf").count(), 1);
}

#[test]
fn the_voice_dialog_opens_on_its_tab_and_moves_with_the_arrows() {
    use crate::screens::voice::dialogs::{self, Tab};
    use egui_kittest::kittest::NodeT;
    let (client, call) = offline();
    let mut h = harness(client, call, dialogs::show);
    dialogs::open_settings(&h.ctx, Tab::Diagnose);
    h.run_steps(6);
    let selected = |h: &H, label: &str| {
        h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    };
    assert!(selected(&h, "Diagnose"), "opens on the asked tab");
    assert!(h.get_by_label("Diagnose").accesskit_node().is_focused());
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(6);
    assert!(selected(&h, "Audio"), "the arrows wrap round");
    h.key_press(egui::Key::End);
    h.run_steps(6);
    assert!(selected(&h, "Diagnose"));
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(6);
    assert!(selected(&h, "Video & Stream"));
    h.get_by_label("Fertig").click();
    h.run_steps(6);
    assert!(!dialogs::open(&h.ctx));
}

#[test]
fn the_microphone_test_speaks_as_the_desktop_app() {
    use crate::screens::voice::dialogs::{self, Tab};
    let (client, call) = offline();
    let mut h = harness(client, call, dialogs::show);
    dialogs::open_settings(&h.ctx, Tab::Audio);
    h.run_steps(6);
    // The native test only meters: nothing to record and compare.
    assert_eq!(h.query_all_by_label("Mikrofon testen").count(), 1);
    assert!(
        h.query_by_label("Mikrofon testen und vergleichen")
            .is_none()
    );
    dialogs::open_mic_test(&h.ctx);
    h.run_steps(6);
    assert_eq!(h.query_all_by_label("WebRTC-Filter").count(), 1);
    assert!(h.query_by_label("Browserfilter").is_none());
}

/// The bar with the window's width, as the shell lays it out.
fn wide_bar(ui: &mut Ui, app: &mut AppCtx) {
    let screen = ui.ctx().content_rect();
    let height = call_bar::height(ui, app, screen.width() - 288.0);
    call_bar::show(
        ui,
        app,
        Rect::from_min_max(pos2(288.0, screen.bottom() - height), screen.max),
    );
}

#[test]
fn under_1024_the_bar_keeps_its_words_for_tooltips_only() {
    let (client, call) = offline();
    call.set_state(in_sofa(MicStatus::Unavailable));
    let mut h = harness(client, call.clone(), wide_bar);
    h.run_steps(4);
    let widths = |h: &H| {
        ["Verlassen", "Kein Mikrofon", "Live gehen"].map(|l| h.get_by_label(l).rect().width())
    };
    let [leave, mic, live] = widths(&h);
    assert!(
        leave > 80.0 && mic > 100.0 && live > 70.0,
        "{leave} {mic} {live}"
    );
    h.set_size(vec2(1000.0, 800.0));
    h.run_steps(4);
    // Icon (and the Live menu's caret) in an 8px padding, the names kept.
    assert_eq!(widths(&h), [36.0, 36.0, 51.0]);
    h.set_size(vec2(1024.0, 800.0));
    h.run_steps(4);
    assert_eq!(widths(&h)[0], leave);
}

#[test]
fn the_notice_is_a_row_of_the_bar() {
    let (client, call) = offline();
    let mut state = in_sofa(MicStatus::Live);
    state.live = true;
    state.live_sound = LiveSound::Ended;
    call.set_state(state);
    let mut h = harness(client, call.clone(), wide_bar);
    h.run_steps(4);
    let text = "Der Stream-Ton wurde beendet. Das Video läuft weiter.";
    let notice = h.get_by_label(text).rect();
    let leave = h.get_by_label("Verlassen").rect();
    // Under the controls, 6px apart, inside the bar that grew by its row.
    assert!(notice.top() >= leave.bottom() + 6.0, "{notice:?} {leave:?}");
    assert!(notice.bottom() <= 800.0 - 8.0);
    assert_eq!(leave.top(), 800.0 - 87.0 + 1.0 + 8.0);
    h.get_by_label("Hinweis schließen").click();
    h.run_steps(4);
    assert!(h.query_by_label(text).is_none());
    assert_eq!(
        h.get_by_label("Verlassen").rect().top(),
        800.0 - 56.0 + 1.0 + 8.0 + 1.5
    );
}

#[test]
fn the_media_form_lists_devices_as_it_opens_and_on_focus() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (client, call) = offline();
    let shown = Arc::new(AtomicBool::new(true));
    let mut form = crate::screens::settings::media::State::default();
    let on = shown.clone();
    let mut h = harness(client, call.clone(), move |ui, app| {
        if on.load(Ordering::Relaxed) {
            crate::screens::settings::media::video(ui, app, &mut form);
        }
    });
    h.run_steps(3);
    assert_eq!(call.device_refreshes(), 1, "as it opens");
    h.run_steps(3);
    assert_eq!(call.device_refreshes(), 1, "not every frame");
    let focus = |h: &mut H, on: bool| {
        h.input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .focused = Some(on);
    };
    focus(&mut h, false);
    h.run_steps(2);
    focus(&mut h, true);
    h.run_steps(2);
    assert_eq!(call.device_refreshes(), 2, "the window back in front");
    shown.store(false, Ordering::Relaxed);
    h.run_steps(2);
    shown.store(true, Ordering::Relaxed);
    h.run_steps(2);
    assert_eq!(call.device_refreshes(), 3, "opened again");
}

/// The label of the node with the focus.
fn focused(h: &H) -> Option<String> {
    use egui_kittest::kittest::NodeT;
    h.query_all(egui_kittest::kittest::By::new().predicate(|n| n.is_focused()))
        .next()
        .map(|n| n.accesskit_node().label().unwrap_or_default())
}

#[test]
fn popovers_open_with_the_focus_where_the_web_puts_it() {
    let (client, call) = offline();
    call.set_state(in_sofa(MicStatus::Live));
    let mut h = harness(client, call.clone(), bar);
    h.run_steps(4);
    // The connection's details: "Alle Details" (`autoFocus`), not the first button.
    h.get_by_label("Verbindung: wird gemessen … – Details anzeigen")
        .click();
    h.run_steps(4);
    assert_eq!(focused(&h).as_deref(), Some("Alle Details"));
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    // Without one, the first control: "Erneut versuchen".
    call.set_state(in_sofa(MicStatus::Unavailable));
    h.run_steps(2);
    h.get_by_label("Kein Mikrofon").click();
    h.run_steps(4);
    assert_eq!(focused(&h).as_deref(), Some("Erneut versuchen"));
    // Drawn again, the popover leaves the focus where it went.
    h.get_by_label("Mikrofon wählen …").focus();
    h.run_steps(4);
    assert_eq!(focused(&h).as_deref(), Some("Mikrofon wählen …"));
}

/// The sidebar column's right border runs on down between the user panel
/// and the call bar (`.user-panel` sits inside `.channel-sidebar`), and
/// every separator is one whole device pixel, as Chromium draws `1px`, at
/// the standard text size (1.125) and at 150 % too.
#[test]
fn separators_run_through_the_bottom_strip_one_device_pixel_wide() {
    let border = crate::theme::Theme::dark().c.border;
    for ppp in [1.0f32, 1.125, 1.5] {
        let (client, call) = offline();
        call.set_state(in_sofa(MicStatus::Live));
        let mut h = harness_at(ppp, client, call, crate::screens::shell::show);
        h.run_steps(6);
        let image = h.render().expect("render");
        let is_border = |x: u32, y: u32| image.get_pixel(x, y).0[..3] == border.to_array()[..3];
        let device = |v: f32| (v * ppp).round() as u32;
        // The column 287: in the sidebar, the user panel's strip, and only there.
        let x = device(287.0);
        for y in [device(400.0), device(772.0), device(799.0)] {
            assert!(is_border(x, y), "{ppp}: border at {x},{y}");
            assert!(
                !is_border(x - 1, y) && !is_border(x + 1, y),
                "{ppp}: one pixel at {x},{y}"
            );
        }
        // The bottom strip's top line, over the user panel and the call bar.
        let y = device(744.0);
        for x in [device(150.0), device(600.0)] {
            assert!(is_border(x, y), "{ppp}: line at {x},{y}");
            assert!(
                !is_border(x, y - 1) && !is_border(x, y + 1),
                "{ppp}: one pixel at {x},{y}"
            );
        }
    }
}

/// "Zuschauen" from the hint or a live toast (`joinToWatch`): outside a
/// call it joins; in that call it does not join again; in another call it
/// only opens the channel, and the button says so.
#[test]
fn watching_from_outside_joins_watches_or_only_opens() {
    use crate::screens::voice::{hint, session};
    let kino = uuid::Uuid::from_u128(0x21);
    let (client, call) = offline();
    let images = Images::new(client.clone());
    let mut nav = Nav::new("/");
    let mut signed_in = Session::new();
    let mut toasts = Toasts::default();
    let mut app = AppCtx {
        client: &client,
        call: &call,
        images: &images,
        nav: &mut nav,
        session: &mut signed_in,
        account_mail: None,
        toasts: &mut toasts,
    };
    let label = |channel| hint::watch_label(call.with_state(session::joined), SERVER, channel);
    let sofa_page = format!("/s/{SERVER}/c/{SOFA}");

    assert_eq!(label(SOFA), "Zuschauen");
    hint::join_to_watch(&mut app, SERVER, SOFA, &[JONAS]);
    assert_eq!(call.join_attempts(), 1);
    assert_eq!(app.nav.location().path, sofa_page);

    call.set_state(in_sofa(MicStatus::Live));
    app.nav.navigate("/");
    assert_eq!(label(SOFA), "Zuschauen");
    hint::join_to_watch(&mut app, SERVER, SOFA, &[JONAS]);
    assert_eq!(call.join_attempts(), 1, "already there: watch, no join");
    assert_eq!(app.nav.location().path, sofa_page);

    assert_eq!(label(kino), "Zum Kanal");
    hint::join_to_watch(&mut app, SERVER, kino, &[JONAS]);
    assert_eq!(call.join_attempts(), 1, "the room asks before it switches");
    assert_eq!(app.nav.location().path, format!("/s/{SERVER}/c/{kino}"));
    assert_eq!(call.with_state(session::joined), Some((SERVER, SOFA)));
}
