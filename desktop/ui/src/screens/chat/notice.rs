//! The conversation's own notices: `LoadError` (components/LoadError.tsx,
//! ui.css `.ui-load-error`) for a channel or DM that could not be loaded,
//! and `.lr-inline-notice` (chat.css) for a read mark that was not saved.

use egui::{Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{OpState, State};
use uuid::Uuid;

use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::Button;

/// `ChannelPage`: the server's detail failed and none is loaded. `Some`
/// with whether the retry runs (`isFetching`); a server that is gone is
/// the shell's, which leaves it.
pub fn channel_error(s: &State, server: Uuid) -> Option<bool> {
    let key = format!("server:{server}");
    if s.detail(server).is_some() || s.failures(&key) == 0 {
        return None;
    }
    match s.op(&key) {
        Some(OpState::Failed(e)) if e.is_gone() => None,
        Some(OpState::Pending) => Some(true),
        _ => Some(false),
    }
}

/// `DmChannelPage`: fetching the DM failed, and it is not gone (a gone DM
/// goes back to the list).
pub fn dm_error(s: &State, dm: Uuid) -> Option<bool> {
    let key = format!("dm_load:{dm}");
    if s.failures(&key) == 0 || s.dm(dm).is_some() {
        return None;
    }
    match s.op(&key) {
        Some(OpState::Failed(e)) if e.is_gone() => None,
        Some(OpState::Pending) => Some(true),
        _ => Some(false),
    }
}

/// Whether the DM's fetch says it is gone (`isGoneError`).
pub fn dm_gone(s: &State, dm: Uuid) -> bool {
    matches!(s.op(&format!("dm_load:{dm}")), Some(OpState::Failed(e)) if e.is_gone())
}

/// `LoadError`: a 3px danger bar on the left, the message, then "Erneut
/// versuchen" ("Wird geladen…" while it runs). `width` is the box's, from
/// `at`; true when the retry was pressed.
pub fn load_error(ui: &mut Ui, at: egui::Pos2, width: f32, message: &str, pending: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::wrapped(ui, message, text::BODY, c.text, width - 30.0);
    // Padding 10 12 (with the bar 15), the text, a 12px gap, the button.
    let h = 10.0 + block.height() + 12.0 + 32.0 + 10.0;
    let rect = Rect::from_min_size(at, vec2(width, h));
    ui.painter()
        .rect_filled(rect, corner(theme.radius.sm), mix(c.bg, c.danger_bg, 0.18));
    ui.painter()
        .with_clip_rect(Rect::from_min_max(
            rect.min,
            pos2(rect.left() + 3.0, rect.bottom()),
        ))
        .rect_filled(rect, corner(theme.radius.sm), c.danger);
    block.paint(ui.painter(), pos2(rect.left() + 15.0, rect.top() + 10.0));
    let id = egui::Id::unique(("gelabber-load-error", message));
    ui.interact(
        Rect::from_min_size(pos2(rect.left() + 15.0, rect.top() + 10.0), block.size()),
        id.with("text"),
        Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, message));
    let button = Rect::from_min_size(
        pos2(
            rect.left() + 15.0,
            rect.top() + 10.0 + block.height() + 12.0,
        ),
        vec2(width - 30.0, 32.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(button)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child
        .add(
            Button::new("Erneut versuchen")
                .small()
                .pending(pending.then_some("Wird geladen…")),
        )
        .clicked()
        && !pending
}

/// `.lr-inline-notice`: the panel line with a muted 12px text and the
/// bold action at its end; true when the action was pressed.
pub fn inline_notice(ui: &mut Ui, rect: Rect, message: &str, action: &str) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let style = TextStyle::new(12.0, 400.0, 1.5);
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.panel);
    let block = Block::single(ui, message, style, c.muted);
    block.paint(
        ui.painter(),
        pos2(rect.left() + 10.0, rect.center().y - block.height() / 2.0),
    );
    let label = Block::single(ui, action, style.weight(600.0), c.text);
    let r = Rect::from_min_size(
        pos2(
            rect.right() - 10.0 - label.width(),
            rect.center().y - label.height() / 2.0,
        ),
        label.size(),
    );
    let response = ui.interact(
        r,
        egui::Id::unique(("gelabber-inline-notice", message)),
        Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, action));
    label.paint(ui.painter(), r.min);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().hline(
            r.left()..=r.right(),
            r.top() + style.baseline() + 1.5,
            Stroke::new(1.0, c.text),
        );
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;
    use gelabber_client::ApiError;

    fn api(status: u16, code: &str) -> ApiError {
        ApiError::Api {
            status,
            code: code.into(),
            message: String::new(),
            fields: serde_json::Value::Null,
        }
    }

    /// The server's detail failed: the channel page shows the load error,
    /// its retry asks again and reads "Wird geladen…" until the answer.
    #[test]
    fn a_failed_load_shows_the_error_and_its_retry() {
        let server = Uuid::from_u128(5);
        let dm = Uuid::from_u128(6);
        let client = gelabber_client::Client::new("http://127.0.0.1:9").expect("client");
        assert_eq!(client.with_state(|s| channel_error(s, server)), None);
        client.fail_op(&format!("server:{server}"), api(500, "internal"));
        assert_eq!(client.with_state(|s| channel_error(s, server)), Some(false));
        client.fail_op(&format!("dm_load:{dm}"), api(503, "internal"));
        assert_eq!(client.with_state(|s| dm_error(s, dm)), Some(false));
        let pressed = std::cell::Cell::new(false);
        let c = client.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 300.0))
            .build_ui(|ui| {
                crate::fonts::install(ui.ctx());
                crate::theme::set(ui.ctx(), Theme::dark());
                let pending = c.with_state(|s| channel_error(s, server)).unwrap_or(false);
                if load_error(
                    ui,
                    pos2(24.0, 24.0),
                    700.0,
                    "Dieser Kanal konnte nicht geladen werden.",
                    pending,
                ) {
                    pressed.set(true);
                }
            });
        harness.run_steps(2);
        harness.get_by_label("Dieser Kanal konnte nicht geladen werden.");
        harness.get_by_label("Erneut versuchen").click();
        harness.run_steps(2);
        assert!(pressed.get());
        // Gone (404/403) is no load error: the shell or the DM list takes it.
        client.fail_op(&format!("dm_load:{dm}"), api(404, "not_found"));
        assert_eq!(client.with_state(|s| dm_error(s, dm)), None);
        assert!(client.with_state(|s| dm_gone(s, dm)));
    }
}
