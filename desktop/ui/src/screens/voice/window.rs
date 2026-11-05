//! A video in a window of its own (VoiceTile.tsx `OwnWindow`, the Tauri
//! app's desktop/app/src/viewer.rs): for a second monitor or a tiling
//! compositor. An egui deferred viewport titled "<label> – Gelabber" that
//! draws the newest picture contained on black; its tile says "Läuft im
//! eigenen Fenster." meanwhile.

use std::collections::HashMap;

use egui::{Color32, Id, Sense, ViewportBuilder, ViewportClass, ViewportId};
use gelabber_call::{Call, VideoKey};

use super::video::{self, Fit};

/// What the tile says when its window did not open.
pub const FAILED: &str = "Das Videofenster konnte nicht geöffnet werden.";

#[derive(Clone, Default)]
struct Windows {
    /// Open windows by video, with their title.
    open: HashMap<VideoKey, String>,
    /// The video whose window did not open, for its tile's message.
    failed: Option<VideoKey>,
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

/// Shows every open window, once per frame of the main window. Without
/// separate windows (a backend that embeds viewports) the window counts as
/// not opened, as the web's `openNativeViewer` rejecting.
pub fn show(ctx: &egui::Context, call: &Call) {
    let open: Vec<(VideoKey, String)> = with(ctx, |w| w.open.clone().into_iter().collect());
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
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use gelabber_client::Client;

    #[test]
    fn the_menu_entry_opens_a_titled_viewport() {
        let call = Call::new(Client::new("http://127.0.0.1:9").expect("client"));
        let key = VideoKey(42);
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
}
