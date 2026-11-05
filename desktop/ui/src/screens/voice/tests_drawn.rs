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
fn harness(client: Client, call: Call, mut draw: impl FnMut(&mut Ui, &mut AppCtx) + 'static) -> H {
    let images = Images::new(client.clone());
    Harness::builder()
        .with_size(vec2(1280.0, 800.0))
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
        call_bar::dock_notice(&live(LiveSound::Ended), "on", "").as_deref(),
        Some("Der Stream-Ton wurde beendet. Das Video läuft weiter.")
    );
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Off), "on", "").as_deref(),
        Some("Stream-Ton ab dem nächsten Live.")
    );
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Off), "off", ""),
        None
    );
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Sharing), "on", "").as_deref(),
        Some("Dein Stream sendet den Ton aller Anwendungen außer Gelabber.")
    );
    assert_eq!(
        call_bar::dock_notice(&live(LiveSound::Sharing), "on", "Spotify"),
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
