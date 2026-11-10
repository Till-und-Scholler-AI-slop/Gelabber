//! `ConnectionNotice` (components/ConnectionNotice.tsx, connection.css,
//! ws/connectionNotice.ts): a pill under the top bar while the gateway is
//! gone, and a moment after it is back. A quick reconnect stays invisible.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::GatewayStatus;

use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner, motion, space};

/// `NOTICE_DELAY_MS`: a dropped socket is usually back well within this.
pub const NOTICE_DELAY: f64 = 3.0;
/// `RESTORED_MS`: how long "Verbindung wiederhergestellt" stays.
pub const RESTORED: f64 = 3.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Offline,
    Reconnecting,
    /// The server or TLS turned the upgrade down (`State::gateway_refused`):
    /// retrying is slow, so it shows at once with why.
    Refused(String),
    Restored,
}

impl Notice {
    pub fn text(&self) -> String {
        match self {
            Self::Offline => "Keine Verbindung – dein Gerät ist offline".into(),
            Self::Reconnecting => "Keine Verbindung – wird wiederhergestellt…".into(),
            Self::Refused(reason) if reason.starts_with("TLS") => {
                "Keine Verbindung – das Zertifikat des Servers wird nicht akzeptiert".into()
            }
            Self::Refused(reason) if reason.starts_with("HTTP ") => {
                format!("Keine Verbindung – vom Server abgelehnt ({reason})")
            }
            Self::Refused(_) => "Keine Verbindung – vom Server abgelehnt".into(),
            Self::Restored => "Verbindung wiederhergestellt".into(),
        }
    }
}

/// `connectionNotices`, on the frame clock instead of timers.
#[derive(Clone, Debug, Default)]
pub struct Notices {
    notice: Option<Notice>,
    delay_until: Option<f64>,
    restored_until: Option<f64>,
    /// When the pill (as lost or as restored) appeared, for its fade-in.
    since: f64,
}

impl Notices {
    fn show(&mut self, next: Option<Notice>, now: f64) {
        if next == self.notice {
            return;
        }
        // The pill's key: lost ↔ restored starts it again.
        let restored = |n: &Option<Notice>| *n == Some(Notice::Restored);
        if self.notice.is_none() || restored(&next) != restored(&self.notice) {
            self.since = now;
        }
        self.notice = next;
    }

    fn troubled(&self) -> bool {
        matches!(
            self.notice,
            Some(Notice::Offline | Notice::Reconnecting | Notice::Refused(_))
        )
    }

    /// `update({status, online})` plus the timers that ran out by `now`;
    /// `refused` is why the gateway turns the upgrade down, if it does.
    pub fn update(
        &mut self,
        status: GatewayStatus,
        online: bool,
        refused: Option<&str>,
        now: f64,
    ) -> Option<Notice> {
        if self.delay_until.is_some_and(|at| now >= at) {
            self.delay_until = None;
            self.show(Some(Notice::Reconnecting), now);
        }
        if self.restored_until.is_some_and(|at| now >= at) {
            self.restored_until = None;
            self.show(None, now);
        }
        match status {
            // Signed out: there is no connection to speak of.
            GatewayStatus::Idle => {
                self.delay_until = None;
                self.restored_until = None;
                self.show(None, now);
            }
            _ if !online => {
                self.delay_until = None;
                self.restored_until = None;
                self.show(Some(Notice::Offline), now);
            }
            GatewayStatus::Connecting | GatewayStatus::Reconnecting if refused.is_some() => {
                self.delay_until = None;
                self.restored_until = None;
                let reason = refused.unwrap_or_default().to_owned();
                self.show(Some(Notice::Refused(reason)), now);
            }
            GatewayStatus::Open => {
                self.delay_until = None;
                if self.troubled() {
                    self.show(Some(Notice::Restored), now);
                    self.restored_until = Some(now + RESTORED);
                }
            }
            GatewayStatus::Connecting | GatewayStatus::Reconnecting => {
                if self.troubled() {
                    self.show(Some(Notice::Reconnecting), now);
                } else if status == GatewayStatus::Reconnecting && self.delay_until.is_none() {
                    self.restored_until = None;
                    self.show(None, now);
                    self.delay_until = Some(now + NOTICE_DELAY);
                }
            }
        }
        self.notice.clone()
    }

    /// The pill's opacity: `@starting-style` fades it in over `--lr-dur`.
    pub fn opacity(&self, now: f64) -> f32 {
        ((now - self.since) as f32 / motion::STANDARD).clamp(0.0, 1.0)
    }

    /// When the next timer runs out, to wake the frame loop for it.
    fn next_timer(&self) -> Option<f64> {
        match (self.delay_until, self.restored_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

fn state_id() -> Id {
    Id::unique("gelabber-connection-notice")
}

/// The notice's state in `ctx` (tests read it).
pub fn notices(ctx: &egui::Context) -> Notices {
    ctx.data(|d| d.get_temp::<Notices>(state_id()))
        .unwrap_or_default()
}

/// Updates the notice from the gateway and paints the pill over the page.
pub fn show(ui: &mut Ui, status: GatewayStatus, refused: Option<&str>) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let mut notices = notices(&ctx);
    // The desktop has no navigator.onLine; a lost network shows as a
    // socket that does not come back.
    let notice = notices.update(status, true, refused, now);
    if let Some(at) = notices.next_timer() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
    }
    let opacity = notices.opacity(now);
    if opacity < 1.0 {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), notices));
    let Some(notice) = notice else { return };
    paint(ui, &notice, opacity);
}

fn paint(ui: &mut Ui, notice: &Notice, opacity: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let restored = *notice == Notice::Restored;
    let text = notice.text();
    let color = if restored { c.online_text } else { c.text };
    let block = Block::single(ui, &text, text::UI, color);
    // padding 6px 14px 6px 12px, a 1px border, 8px gap after the mark.
    let mark = if restored { 14.0 } else { 8.0 };
    let width = 1.0 + 12.0 + mark + space::S2 + block.width() + 14.0 + 1.0;
    let height = 1.0 + 6.0 + block.height() + 6.0 + 1.0;
    let screen = ui.ctx().content_rect();
    // top: 60px, centred between 16px insets; it slides down 6px as it fades in.
    let top = screen.top() + 60.0 - 6.0 * (1.0 - opacity);
    let left = screen.center().x - width / 2.0;
    let rect = Rect::from_min_size(Pos2::new(left.max(16.0), top), Vec2::new(width, height));
    let fade = |col: Color32| col.gamma_multiply(opacity);
    egui::Area::new(Id::unique("gelabber-connection-pill"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(rect.min)
        .show(ui.ctx(), |ui| {
            let painter = ui.painter();
            let mut shadow = theme.shadow_1;
            shadow.color = fade(shadow.color);
            painter.add(shadow.as_shape(rect, corner(theme.radius.full)));
            painter.rect(
                rect,
                corner(theme.radius.full),
                fade(c.elevated),
                Stroke::new(1.0, fade(c.border)),
                StrokeKind::Inside,
            );
            let x = rect.left() + 13.0;
            let mid = rect.center().y;
            if restored {
                icons::paint(painter, Icon::Check, pos2(x, mid - 7.0), 14.0, fade(color));
            } else {
                painter.circle_filled(pos2(x + 4.0, mid), 4.0, fade(c.warning));
            }
            block.paint_tinted(
                painter,
                pos2(x + mark + space::S2, rect.top() + 7.0),
                Some(fade(color)),
            );
            // role="status": the change is read out.
            let status = ui.interact(rect, Id::unique("gelabber-connection-text"), Sense::hover());
            crate::widgets::announce(&status, egui::Role::Status, &text);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn the_notice_follows_the_gateway_and_fades() {
        let status = Rc::new(Cell::new(GatewayStatus::Open));
        let shared = status.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_step_dt(0.05)
            .with_context_setup(crate::fonts::install)
            .build_ui(move |ui| show(ui, shared.get(), None));
        let lost = "Keine Verbindung – wird wiederhergestellt…";
        let back = "Verbindung wiederhergestellt";
        harness.run_steps(2);
        assert!(harness.query_by_label(lost).is_none());
        // A drop shows only after NOTICE_DELAY.
        status.set(GatewayStatus::Reconnecting);
        harness.run_steps(40);
        assert!(harness.query_by_label(lost).is_none());
        harness.run_steps(22);
        assert!(harness.query_by_label(lost).is_some());
        let ctx = harness.ctx.clone();
        let now = ctx.input(|i| i.time);
        // Fading in: less than opaque on its first frames, opaque after --lr-dur.
        let start = notices(&ctx);
        assert!(start.opacity(start.since) < 0.01);
        assert!(start.opacity(now + 0.2) >= 1.0);
        // Back: the restored pill, then nothing after RESTORED.
        status.set(GatewayStatus::Open);
        harness.run_steps(2);
        assert!(harness.query_by_label(back).is_some());
        assert!(harness.query_by_label(lost).is_none());
        assert!(notices(&ctx).opacity(ctx.input(|i| i.time)) < 1.0);
        harness.run_steps(70);
        assert!(harness.query_by_label(back).is_none());
        // A quick reconnect stays invisible.
        status.set(GatewayStatus::Reconnecting);
        harness.run_steps(20);
        status.set(GatewayStatus::Open);
        harness.run_steps(60);
        assert!(harness.query_by_label(lost).is_none());
        assert!(harness.query_by_label(back).is_none());
        // Signed out: nothing.
        status.set(GatewayStatus::Reconnecting);
        harness.run_steps(70);
        assert!(harness.query_by_label(lost).is_some());
        status.set(GatewayStatus::Idle);
        harness.run_steps(2);
        assert!(harness.query_by_label(lost).is_none());
    }

    #[test]
    fn offline_shows_at_once() {
        let mut n = Notices::default();
        assert_eq!(
            n.update(GatewayStatus::Reconnecting, false, None, 0.0),
            Some(Notice::Offline)
        );
        assert_eq!(
            n.update(GatewayStatus::Open, true, None, 1.0),
            Some(Notice::Restored)
        );
        assert_eq!(n.update(GatewayStatus::Open, true, None, 4.1), None);
    }

    #[test]
    fn a_refused_upgrade_shows_at_once_with_why() {
        let mut n = Notices::default();
        let refused = n.update(GatewayStatus::Reconnecting, true, Some("HTTP 403"), 0.0);
        assert_eq!(refused, Some(Notice::Refused("HTTP 403".into())));
        assert_eq!(
            refused.unwrap().text(),
            "Keine Verbindung – vom Server abgelehnt (HTTP 403)"
        );
        // It stays past the reconnect delay while the slow retry runs.
        assert!(matches!(
            n.update(GatewayStatus::Connecting, true, Some("HTTP 403"), 10.0),
            Some(Notice::Refused(_))
        ));
        let tls = n.update(
            GatewayStatus::Reconnecting,
            true,
            Some("TLS: bad cert"),
            11.0,
        );
        assert_eq!(
            tls.unwrap().text(),
            "Keine Verbindung – das Zertifikat des Servers wird nicht akzeptiert"
        );
        // Accepted again: restored, like any other comeback.
        assert_eq!(
            n.update(GatewayStatus::Open, true, None, 12.0),
            Some(Notice::Restored)
        );
    }
}
