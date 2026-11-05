//! Toasts (components/Toasts.tsx, toasts.ts, ui.css `.gel-toast`): a stack
//! at the bottom right, info for 4s and errors for 10s, newest at the
//! bottom. The pointer on a toast or the focus in it holds its countdown.

use egui::{Id, Order, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::ApiError;

use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::IconButton;

/// `INFO_MS`, `ERROR_MS`.
const INFO: f64 = 4.0;
const ERROR: f64 = 10.0;
/// Back from a hold, there is always a moment left to finish reading.
const RELEASE_MIN: f64 = 1.5;

#[derive(Clone, Debug, PartialEq)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    /// Seconds still to show while the countdown runs.
    left: f64,
    /// When `left` was last counted down; `None` before the first frame.
    counted: Option<f64>,
    hover: bool,
    focus: bool,
}

impl Toast {
    fn held(&self) -> bool {
        self.hover || self.focus
    }
}

/// The composer drawn this frame (`anchor-name: --lr-composer`).
pub fn set_anchor(ctx: &egui::Context, rect: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(Id::unique("gelabber-toast-anchor"), (rect, frame)));
}

fn space_id() -> Id {
    Id::unique("gelabber-toast-space")
}

fn set_space(ctx: &egui::Context, space: f32) {
    if space != self::space(ctx) {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(space_id(), space));
}

/// How tall the stack over the composer was last frame (`--lr-toast-space`),
/// 0 without one: the message list ends that much plus 12px higher
/// (`.lr-toast-space`), so no toast covers the newest message (#354).
pub fn space(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(space_id())).unwrap_or(0.0)
}

fn anchor(ctx: &egui::Context) -> Option<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(Rect, u64)>(Id::unique("gelabber-toast-anchor")))
        .filter(|(_, f)| *f == frame)
        .map(|(r, _)| r)
}

/// `toastText`: what an error toast says about `error`; `None` for a lost
/// session (the app is on its way to the login page).
pub fn error_text(error: &ApiError) -> Option<String> {
    let code = error.code();
    if code == "unauthenticated" {
        return None;
    }
    if code == "validation_failed"
        && let ApiError::Api { fields, .. } = error
        && let Some(fields) = fields.as_object()
    {
        let detail: Vec<String> = fields
            .iter()
            .filter_map(|(field, code)| Some(crate::rules::field_message(field, code.as_str()?)))
            .filter(|m| !m.is_empty())
            .collect();
        if !detail.is_empty() {
            return Some(detail.join(" "));
        }
    }
    Some(crate::rules::error_message(code).to_owned())
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// `notify(text)`: an info toast.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.push(text.into(), false);
    }

    /// `notifyError`: an error toast with the warning icon.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), true);
    }

    /// `notifyError(error, failed)`: `failed` names the change that did not
    /// happen ("Nachricht nicht gelöscht."), the error says why.
    pub fn failure(&mut self, failed: &str, error: &ApiError) {
        if let Some(text) = error_text(error) {
            self.error(format!("{failed} {text}"));
        }
    }

    fn push(&mut self, text: String, error: bool) {
        // A burst of the same notice shows once; a fourth error is dropped.
        if self
            .items
            .iter()
            .any(|t| t.text == text && t.error == error)
        {
            return;
        }
        if error && self.items.iter().filter(|t| t.error).count() >= 3 {
            return;
        }
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            text,
            error,
            left: if error { ERROR } else { INFO },
            counted: None,
            hover: false,
            focus: false,
        });
    }

    /// The texts shown now, oldest first.
    pub fn texts(&self) -> Vec<&str> {
        self.items.iter().map(|t| t.text.as_str()).collect()
    }

    /// Counts the running toasts down to `now` and drops the expired ones;
    /// a held toast keeps its time (`hold`/`release`).
    fn advance(&mut self, now: f64) {
        for toast in &mut self.items {
            let since = toast.counted.unwrap_or(now);
            if !toast.held() {
                toast.left -= now - since;
            }
            toast.counted = Some(now);
        }
        self.items.retain(|t| t.held() || t.left > 0.0);
    }

    /// Sets what holds toast `id`; a release leaves it at least 1.5s.
    fn hold(&mut self, id: u64, hover: bool, focus: bool) {
        if let Some(toast) = self.items.iter_mut().find(|t| t.id == id) {
            let was = toast.held();
            toast.hover = hover;
            toast.focus = focus;
            if was && !toast.held() {
                toast.left = toast.left.max(RELEASE_MIN);
            }
        }
    }

    /// Paints the stack; `outside_workspace` puts it 16px over the bottom
    /// (auth pages), else 12px.
    pub fn show(&mut self, ui: &Ui, outside_workspace: bool) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.advance(now);
        if self.items.is_empty() {
            set_space(&ctx, 0.0);
            return;
        }
        let theme = Theme::of(&ctx);
        let c = theme.c;
        let screen = ctx.content_rect();
        let width = 380.0_f32.min(screen.width() - 32.0);
        // In the workspace 12px over the composer, 16px in from its right
        // edge; without one, 12px over the bottom.
        let composer = (!outside_workspace).then(|| anchor(&ctx)).flatten();
        let bottom = match composer {
            Some(r) => r.top() - 12.0,
            None => screen.bottom() - if outside_workspace { 16.0 } else { 12.0 },
        };
        let right = composer.map_or(screen.right(), |r| r.right()) - 16.0;
        let mut closed = None;
        let mut holds = Vec::new();
        let mut span: Option<(f32, f32)> = None;
        // Info toasts sit above the error toasts.
        let mut order: Vec<&Toast> = self.items.iter().filter(|t| !t.error).collect();
        order.extend(self.items.iter().filter(|t| t.error));
        let any_error = self.items.iter().any(|t| t.error);
        egui::Area::new(Id::unique("gelabber-toasts"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(&ctx, |ui| {
                // Two regions, info over errors, 8px apart: an empty error
                // region still keeps its gap.
                let mut y = bottom;
                if !any_error {
                    y -= 8.0;
                }
                for toast in order.iter().rev() {
                    let left_pad = if toast.error {
                        12.0 + 3.0 + 16.0 + 12.0
                    } else {
                        13.0
                    };
                    let text_w = width - left_pad - 12.0 - 32.0 - 9.0;
                    let block = Block::wrapped(ui, &toast.text, text::BODY, c.text, text_w);
                    // The 32px close button overhangs by 6px top and bottom.
                    let height = block.height().max(32.0 - 12.0) + 24.0 + 2.0;
                    let rect = Rect::from_min_max(pos2(right - width, y - height), pos2(right, y));
                    let painter = ui.painter();
                    painter.add(theme.shadow_2.as_shape(rect, corner(theme.radius.lg)));
                    painter.rect(
                        rect,
                        corner(theme.radius.lg),
                        c.elevated,
                        Stroke::new(1.0, c.border),
                        StrokeKind::Inside,
                    );
                    if toast.error {
                        painter
                            .with_clip_rect(Rect::from_min_max(
                                rect.min,
                                pos2(rect.left() + 3.0, rect.bottom()),
                            ))
                            .rect_filled(rect, corner(theme.radius.lg), c.danger);
                        icons::paint(
                            painter,
                            Icon::Warning,
                            Pos2::new(rect.left() + 15.0, rect.center().y - 8.0),
                            16.0,
                            c.danger,
                        );
                    }
                    block.paint(
                        painter,
                        Pos2::new(
                            rect.left() + left_pad,
                            rect.center().y - block.height() / 2.0,
                        ),
                    );
                    // role="status": the message is read out (and found by tests).
                    let card = ui.interact(
                        rect,
                        Id::unique(("gelabber-toast", toast.id)),
                        egui::Sense::hover(),
                    );
                    card.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Label, true, &toast.text)
                    });
                    let close = Rect::from_min_size(
                        pos2(rect.right() - 9.0 - 32.0, rect.center().y - 16.0),
                        Vec2::splat(32.0),
                    );
                    let button = ui.add(
                        IconButton::new(Icon::Close, "Schließen")
                            .size(32.0)
                            .at(close),
                    );
                    if button.clicked() {
                        closed = Some(toast.id);
                    }
                    // The close button covers part of the card: either holds it.
                    let hover = card.contains_pointer() || button.hovered();
                    holds.push((toast.id, hover, button.has_focus()));
                    span = Some(span.map_or((rect.top(), rect.bottom()), |(t, b)| {
                        (t.min(rect.top()), b.max(rect.bottom()))
                    }));
                    y = rect.top() - 8.0;
                }
            });
        // `--lr-toast-space`: above a composer the list makes room for them.
        let space = match (composer, span) {
            (Some(_), Some((top, bottom))) => (bottom - top).ceil(),
            _ => 0.0,
        };
        set_space(&ctx, space);
        for (id, hover, focus) in holds {
            self.hold(id, hover, focus);
        }
        if let Some(id) = closed {
            self.items.retain(|t| t.id != id);
        }
        if let Some(next) = self
            .items
            .iter()
            .filter(|t| !t.held())
            .map(|t| t.left)
            .reduce(f64::min)
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: &str, fields: serde_json::Value) -> ApiError {
        ApiError::Api {
            status: 400,
            code: code.into(),
            message: String::new(),
            fields,
        }
    }

    #[test]
    fn a_held_toast_keeps_its_time() {
        let mut t = Toasts::default();
        t.notify("Hallo");
        t.advance(0.0);
        t.advance(3.0);
        let id = t.items[0].id;
        // The pointer comes at 3s: the countdown stops.
        t.hold(id, true, false);
        t.advance(30.0);
        assert_eq!(t.texts(), ["Hallo"]);
        // Focus takes over from the pointer: still held.
        t.hold(id, false, true);
        t.advance(60.0);
        assert_eq!(t.texts(), ["Hallo"]);
        // Released: the one second left becomes 1.5s, then it goes.
        t.hold(id, false, false);
        t.advance(61.4);
        assert_eq!(t.texts(), ["Hallo"]);
        t.advance(61.6);
        assert!(t.texts().is_empty());
    }

    #[test]
    fn a_released_toast_runs_out_from_where_it_stopped() {
        let mut t = Toasts::default();
        t.error("Fehler");
        t.advance(0.0);
        t.advance(2.0);
        let id = t.items[0].id;
        t.hold(id, true, false);
        t.advance(100.0);
        t.hold(id, false, false);
        // Eight of the ten seconds are left after the hold.
        t.advance(107.9);
        assert_eq!(t.texts(), ["Fehler"]);
        t.advance(108.1);
        assert!(t.texts().is_empty());
    }

    /// The pointer on the card holds it on screen; once it leaves, the
    /// toast runs out.
    #[test]
    fn the_pointer_holds_a_drawn_toast() {
        use egui_kittest::kittest::Queryable as _;
        let toasts = std::cell::RefCell::new(Toasts::default());
        toasts
            .borrow_mut()
            .notify("Server nicht gefunden oder kein Zugriff.");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .with_step_dt(0.1)
            .build_ui(|ui| {
                crate::fonts::install(ui.ctx());
                crate::theme::set(ui.ctx(), Theme::dark());
                toasts.borrow_mut().show(ui, true);
            });
        harness.run_steps(5);
        let close = harness.get_by_label("Schließen").rect().center();
        harness.hover_at(close);
        harness.run_steps(100);
        assert_eq!(toasts.borrow().texts().len(), 1, "held for 10s");
        // Held after half a second: 3.5 of the 4 seconds are left.
        harness.hover_at(egui::pos2(1.0, 1.0));
        harness.run_steps(30);
        assert_eq!(toasts.borrow().texts().len(), 1, "runs on where it stopped");
        harness.run_steps(10);
        assert!(toasts.borrow().texts().is_empty());
    }

    #[test]
    fn duplicates_and_a_fourth_error_are_dropped() {
        let mut t = Toasts::default();
        t.error("a");
        t.error("a");
        t.error("b");
        t.error("c");
        t.error("d");
        t.notify("a");
        assert_eq!(t.texts(), ["a", "b", "c", "a"]);
    }

    #[test]
    fn error_texts_follow_the_web() {
        assert_eq!(
            error_text(&api("forbidden", serde_json::Value::Null)).as_deref(),
            Some("Dafür fehlt dir die Berechtigung.")
        );
        assert_eq!(
            error_text(&api("unauthenticated", serde_json::Value::Null)),
            None
        );
        assert_eq!(
            error_text(&api(
                "validation_failed",
                serde_json::json!({"name": "required"})
            ))
            .as_deref(),
            Some("Name fehlt.")
        );
        assert_eq!(
            error_text(&ApiError::Transport("reset".into())).as_deref(),
            Some("Keine Verbindung zum Server.")
        );
    }

    /// Every optimistic write: its op fails in the store, the notice
    /// becomes the web's toast with the API error appended.
    #[test]
    fn failed_writes_show_their_toast() {
        use egui_kittest::kittest::Queryable as _;
        let cases = [
            ("server_update:1", "Server nicht gespeichert."),
            ("delete_server:1", "Server nicht gelöscht."),
            ("leave:1", "Server nicht verlassen."),
            ("create_category", "Kategorie nicht erstellt."),
            ("category:1", "Kategorie nicht gespeichert."),
            ("delete_category:1", "Kategorie nicht gelöscht."),
            ("create_channel", "Kanal nicht erstellt."),
            ("channel:1", "Kanal nicht gespeichert."),
            ("delete_channel:1", "Kanal nicht gelöscht."),
            ("revoke:abc", "Einladung nicht widerrufen."),
            ("kick:1", "Mitglied nicht entfernt."),
            ("ban:1", "Mitglied nicht gesperrt."),
            ("unban:1", "Mitglied nicht entsperrt."),
            ("edit:1", "Änderung nicht gespeichert."),
            ("delete:1", "Nachricht nicht gelöscht."),
            ("react:1", "Reaktion nicht gespeichert."),
            ("dm", "Unterhaltung nicht geöffnet."),
        ];
        let client = gelabber_client::Client::new("http://127.0.0.1:9").expect("client");
        for (key, failed) in cases {
            client.fail_op(key, api("forbidden", serde_json::Value::Null));
            assert!(
                client
                    .with_state(|s| matches!(s.op(key), Some(gelabber_client::OpState::Failed(_))))
            );
            let toasts = std::cell::RefCell::new(Toasts::default());
            let c = client.clone();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(800.0, 400.0))
                .build_ui(|ui| {
                    crate::fonts::install(ui.ctx());
                    crate::theme::set(ui.ctx(), Theme::dark());
                    let mut toasts = toasts.borrow_mut();
                    for notice in c.take_notices() {
                        if let gelabber_client::Notice::Failed { failed, error } = notice {
                            toasts.failure(failed, &error);
                        }
                    }
                    toasts.show(ui, true);
                });
            harness.run_steps(3);
            let text = format!("{failed} Dafür fehlt dir die Berechtigung.");
            harness.get_by_label(&text);
            drop(harness);
            assert_eq!(toasts.borrow().texts(), [text.as_str()]);
        }
        // A lost session shows no toast: the app goes to the login page.
        client.fail_op("delete:2", api("unauthenticated", serde_json::Value::Null));
        let mut t = Toasts::default();
        for notice in client.take_notices() {
            if let gelabber_client::Notice::Failed { failed, error } = notice {
                t.failure(failed, &error);
            }
        }
        assert!(t.texts().is_empty());
    }
}
