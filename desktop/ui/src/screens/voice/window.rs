//! A video in a window of its own (VoiceTile.tsx `OwnWindow`, the Tauri
//! app's viewer.rs): for a second monitor or a tiling
//! compositor. An egui deferred viewport titled "<label> – Gelabber" that
//! draws the newest picture contained on black; its tile says "Läuft im
//! eigenen Fenster." meanwhile.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Id, Sense, ViewportBuilder, ViewportClass, ViewportId};
use gelabber_call::{Call, CallStatus, VideoKey};

use super::video::{self, Fit};

/// What the tile says when its window did not open.
pub const FAILED: &str = "Das Videofenster konnte nicht geöffnet werden.";

#[derive(Clone, Default)]
struct Windows {
    /// Open windows by video, with their title.
    open: HashMap<VideoKey, String>,
    /// The video whose window did not open, for its tile's message.
    failed: Option<VideoKey>,
    /// The picture each window drew last (`Frame::serial`).
    drawn: HashMap<VideoKey, u64>,
    /// The call the open windows show, for [`wake`]; only while one is open,
    /// as the windows' own draw callbacks hold it too.
    call: Option<Call>,
}

fn id() -> Id {
    Id::unique("gelabber-voice-windows")
}

fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut Windows) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Windows>(id())))
}

/// The window's viewport.
pub fn viewport_id(key: VideoKey) -> ViewportId {
    ViewportId::from_hash_of(("gelabber-video-window", key.0))
}

/// `${label} – Gelabber`.
pub fn title(label: &str) -> String {
    format!("{label} – Gelabber")
}

pub fn is_open(ctx: &egui::Context, key: VideoKey) -> bool {
    with(ctx, |w| w.open.contains_key(&key))
}

pub fn failed(ctx: &egui::Context, key: VideoKey) -> bool {
    with(ctx, |w| w.failed == Some(key))
}

/// "Eigenes Fenster" / "Fenster schließen".
pub fn toggle(ctx: &egui::Context, key: VideoKey, label: &str) {
    with(ctx, |w| {
        w.failed = None;
        if w.open.remove(&key).is_none() {
            w.open.insert(key, title(label));
        }
    });
}

pub fn close(ctx: &egui::Context, key: VideoKey) {
    with(ctx, |w| {
        w.open.remove(&key);
    });
}

/// The open windows whose video has a picture they did not draw yet.
fn due(w: &Windows, serial: impl Fn(VideoKey) -> Option<u64>) -> Vec<VideoKey> {
    w.open
        .keys()
        .copied()
        .filter(|k| serial(*k).is_some_and(|s| w.drawn.get(k) != Some(&s)))
        .collect()
}

/// Repaints the windows with a new picture; they do not poll. Called from
/// the client's repaint hook on any thread, as each picture arrives: the
/// main window's own passes slow down or stop while it is minimised,
/// hidden or (Wayland) on another workspace, the video's window does not.
pub fn wake(ctx: &egui::Context) {
    let Some(w) = ctx.data(|d| d.get_temp::<Windows>(id()).filter(|w| !w.open.is_empty())) else {
        return;
    };
    let Some(call) = &w.call else { return };
    for key in due(&w, |k| call.frame(k).map(|f| f.serial)) {
        ctx.request_repaint_of(viewport_id(key));
    }
}

/// Shows every open window, once per frame of the main window. Without
/// separate windows (a backend that embeds viewports) the window counts as
/// not opened, as the web's `openNativeViewer` rejecting.
pub fn show(ctx: &egui::Context, call: &Call) {
    // A video that ended (the call, the camera, the Live) takes its window
    // along; a reconnect keeps it.
    let shown: Option<HashSet<VideoKey>> = call.with_state(|s| match s.status {
        CallStatus::Connecting | CallStatus::Reconnecting => None,
        CallStatus::Connected => Some(
            s.participants
                .iter()
                .flat_map(|p| [p.camera, p.live])
                .flatten()
                .collect(),
        ),
        CallStatus::Idle | CallStatus::Failed(_) => Some(HashSet::new()),
    });
    let open: Vec<(VideoKey, String)> = with(ctx, |w| {
        if let Some(shown) = &shown {
            w.open.retain(|k, _| shown.contains(k));
        }
        w.drawn.retain(|k, _| w.open.contains_key(k));
        w.call = (!w.open.is_empty()).then(|| call.clone());
        w.open.clone().into_iter().collect()
    });
    if open.is_empty() {
        return;
    }
    if ctx.embed_viewports() {
        with(ctx, |w| {
            w.failed = open.first().map(|(k, _)| *k);
            w.open.clear();
        });
        return;
    }
    wake(ctx);
    for (key, title) in open {
        let call = call.clone();
        ctx.show_viewport_deferred(
            viewport_id(key),
            ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([960.0, 540.0])
                .with_min_inner_size([320.0, 180.0]),
            move |ctx, class| {
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    close(ctx, key);
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(Color32::BLACK))
                    .show(ctx, |ui| {
                        let rect = ui.max_rect();
                        ui.allocate_rect(rect, Sense::hover());
                        if let Some(frame) = call.frame(key) {
                            with(ui.ctx(), |w| w.drawn.insert(key, frame.serial));
                            // The core scales to the size asked for: the window's, not the tile's.
                            video::request_fit(
                                ui,
                                &call,
                                key,
                                rect,
                                Some(&frame),
                                Fit::Contain,
                                30,
                            );
                            video::paint(
                                ui,
                                Id::unique(("video-window", key.0)),
                                rect,
                                frame,
                                Fit::Contain,
                                false,
                                0.0,
                            );
                        }
                    });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use gelabber_call::{CallState, Participant};
    use gelabber_client::Client;

    /// A call in which Jonas sends `key` as his camera.
    fn sending(call: &Call, key: Option<VideoKey>) {
        call.set_state(CallState {
            status: CallStatus::Connected,
            participants: vec![Participant {
                user: uuid::Uuid::from_u128(0x12),
                camera: key,
                ..Participant::default()
            }],
            ..CallState::default()
        });
    }

    #[test]
    fn the_menu_entry_opens_a_titled_viewport() {
        let call = Call::new(Client::new("http://127.0.0.1:9").expect("client"));
        let key = VideoKey(42);
        sending(&call, Some(key));
        let mut harness = Harness::new_ui(move |ui| {
            let ctx = ui.ctx();
            ctx.set_embed_viewports(false);
            show(ctx, &call);
        });
        toggle(&harness.ctx, key, "Jonas — Live");
        harness.step();
        let viewports = &harness.output().viewport_output;
        let output = viewports
            .get(&viewport_id(key))
            .expect("a viewport for the window");
        assert_eq!(
            output.builder.title.as_deref(),
            Some("Jonas — Live – Gelabber")
        );
        assert!(is_open(&harness.ctx, key));
        // "Fenster schließen" takes it away again.
        toggle(&harness.ctx, key, "Jonas — Live");
        harness.step();
        assert!(
            !harness
                .output()
                .viewport_output
                .contains_key(&viewport_id(key))
        );
    }

    #[test]
    fn without_windows_the_tile_says_so() {
        let call = Call::new(Client::new("http://127.0.0.1:9").expect("client"));
        let key = VideoKey(7);
        sending(&call, Some(key));
        let mut harness = Harness::new_ui(move |ui| {
            let ctx = ui.ctx();
            ctx.set_embed_viewports(true);
            show(ctx, &call);
        });
        toggle(&harness.ctx, key, "Jonas");
        harness.step();
        assert!(failed(&harness.ctx, key));
        assert!(!is_open(&harness.ctx, key));
        assert_eq!(FAILED, "Das Videofenster konnte nicht geöffnet werden.");
    }

    /// The camera going off, then the call ending, close the window: it
    /// does not stay black, nor come back with the next call.
    #[test]
    fn a_window_goes_with_its_video() {
        let call = Call::new(Client::new("http://127.0.0.1:9").expect("client"));
        let key = VideoKey(42);
        sending(&call, Some(key));
        let shown = call.clone();
        let mut harness = Harness::new_ui(move |ui| {
            ui.ctx().set_embed_viewports(false);
            show(ui.ctx(), &shown);
        });
        toggle(&harness.ctx, key, "Jonas");
        harness.step();
        assert!(is_open(&harness.ctx, key));
        // A reconnect keeps it.
        call.set_state(CallState {
            status: CallStatus::Reconnecting,
            ..CallState::default()
        });
        harness.step();
        assert!(is_open(&harness.ctx, key));
        sending(&call, None);
        harness.step();
        assert!(!is_open(&harness.ctx, key), "the camera went off");
        sending(&call, Some(key));
        toggle(&harness.ctx, key, "Jonas");
        harness.step();
        assert!(is_open(&harness.ctx, key));
        call.set_state(CallState::default());
        harness.step();
        assert!(!is_open(&harness.ctx, key), "the call ended");
        assert!(
            !harness
                .output()
                .viewport_output
                .contains_key(&viewport_id(key))
        );
    }

    /// The picture's arrival, not the main window's pass, wakes a window:
    /// [`wake`] runs from the client's repaint hook. Only a window whose
    /// video has a picture it did not draw yet is due.
    #[test]
    fn only_a_new_picture_wakes_a_window() {
        let (a, b, c) = (VideoKey(1), VideoKey(2), VideoKey(3));
        let mut w = Windows::default();
        for key in [a, b, c] {
            w.open.insert(key, String::new());
        }
        w.drawn.insert(a, 5);
        w.drawn.insert(b, 5);
        let serial = |k: VideoKey| match k.0 {
            1 => Some(5),
            2 => Some(6),
            _ => None,
        };
        assert_eq!(due(&w, serial), vec![b], "a drew it, c has none yet");
        // No window open: nothing to wake, and no call kept for it.
        let ctx = egui::Context::default();
        wake(&ctx);
    }
}
