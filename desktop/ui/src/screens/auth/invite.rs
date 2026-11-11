//! `/invite/{code}` (pages/InvitePage.tsx): the server an invite opens,
//! and one click to join it.

use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2, pos2};
use gelabber_i18n::intl::{Unit, format_relative};
use gelabber_i18n::{auth, common};
use reqwest::Method;
use serde_json::{Value, json};

use super::{Request, layout, result_lines, submit, wide_button};
use crate::app::AppCtx;
use crate::images::ImageState;
use crate::initials::{initials, tile_tone};
use crate::rules::{Failure, error_message};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, space};
use crate::widgets::{Button, FormError};

pub struct State {
    code: String,
    preview: Option<Result<Value, Failure>>,
    load: Request,
    join: Request,
    join_failure: Option<Failure>,
}

impl State {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            preview: None,
            load: None,
            join: None,
            join_failure: None,
        }
    }
}

/// Answers that say the link itself is no good, not that loading failed.
const DEAD: [&str; 3] = ["not_found", "invite_invalid", "banned"];

fn load(app: &AppCtx, code: &str) -> Request {
    let http = app.client.http().clone();
    let path = format!("/invites/{code}");
    Some(Pending::spawn(app.client, async move {
        http.get::<Value>(&path).await
    }))
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    if s.preview.is_none() && s.load.is_none() {
        s.load = load(app, &s.code);
    }
    if let Some(result) = poll(&mut s.load) {
        s.preview = Some(result.map_err(|e| Failure::from_api(&e)));
    }
    if let Some(result) = poll(&mut s.join) {
        match result {
            Ok(server) => {
                if let Some(id) = server.get("id").and_then(Value::as_str) {
                    app.nav.replace(&format!("/s/{id}"));
                    return;
                }
            }
            Err(error) => s.join_failure = Some(Failure::from_api(&error)),
        }
    }

    let dead_join = s
        .join_failure
        .as_ref()
        .filter(|f| DEAD.contains(&f.code.as_str()))
        .cloned();
    let dead_load = match &s.preview {
        Some(Err(failure)) if DEAD.contains(&failure.code.as_str()) => Some(failure.clone()),
        _ => None,
    };
    if let Some(failure) = dead_join.or(dead_load).filter(|_| s.preview.is_some()) {
        dead(ui, app, &failure.code);
        return;
    }
    match &s.preview {
        None => skeleton(ui, app),
        Some(Err(failure)) => {
            let code = failure.code.clone();
            let reloading = s.load.is_some();
            let mut retry = false;
            layout::frame(ui, app, "invite-error", |ui, app| {
                layout::heading(ui, auth::invite_load_failed());
                layout::lead(ui, error_message(&code));
                ui.add_space(space::S6);
                retry = ui
                    .add(
                        Button::new(common::try_again())
                            .primary()
                            .large()
                            .fill()
                            .pending(reloading.then_some(common::loading())),
                    )
                    .clicked();
                ui.add_space(space::S2);
                if wide_button(ui, auth::invite_to_servers(), true) {
                    app.nav.navigate("/");
                }
            });
            if retry && !reloading {
                s.load = load(app, &s.code);
            }
        }
        Some(Ok(preview)) => {
            let preview = preview.clone();
            loaded(ui, app, s, &preview);
        }
    }
}

fn loaded(ui: &mut Ui, app: &mut AppCtx, s: &mut State, preview: &Value) {
    let server = &preview["server"];
    let id = server["id"].as_str().unwrap_or_default().to_owned();
    let member = preview["member"].as_bool().unwrap_or(false);
    let pending = s.join.is_some();
    let join_message = s.join_failure.as_ref().map(|f| error_message(&f.code));
    let mut join = false;
    layout::frame(ui, app, "invite", |ui, app| {
        invite_server(ui, app, preview);
        ui.add_space(space::S6);
        if member {
            if ui
                .add(
                    Button::new(auth::invite_to_server())
                        .primary()
                        .large()
                        .fill(),
                )
                .clicked()
            {
                app.nav.navigate(&format!("/s/{id}"));
            }
        } else {
            if let Some(message) = join_message {
                let theme = Theme::of(ui.ctx());
                ui.add(FormError::new(message).on(theme.c.panel));
                ui.add_space(space::S3);
            }
            join = submit(ui, auth::invite_join(), auth::invite_joining(), pending);
        }
        ui.add_space(space::S2);
        if wide_button(ui, common::cancel(), true) {
            app.nav.navigate("/");
        }
    });
    if join && !pending {
        s.join_failure = None;
        let http = app.client.http().clone();
        let path = format!("/invites/{}/join", s.code);
        s.join = Some(Pending::spawn(app.client, async move {
            http.send::<Value, Value>(Method::POST, &path, &json!({}))
                .await
        }));
    }
}

/// `InviteServer`: tile, "Einladung zu", the name, members and expiry.
fn invite_server(ui: &mut Ui, app: &AppCtx, preview: &Value) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let server = &preview["server"];
    let name = server["name"].as_str().unwrap_or_default();
    let id = server["id"].as_str().unwrap_or_default();
    let member = preview["member"].as_bool().unwrap_or(false);
    let width = ui.available_width();
    let (tile, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), Sense::hover());
    let tile = Rect::from_center_size(tile.center(), Vec2::splat(56.0));
    let icon = server["icon_url"]
        .as_str()
        .map(|url| app.images.get(ui.ctx(), url));
    match icon {
        Some(ImageState::Ready(texture)) => {
            ui.painter().add(
                egui::epaint::RectShape::filled(tile, corner(theme.radius.lg), Color32::WHITE)
                    .with_texture(
                        texture.id(),
                        crate::images::cover_uv(texture.size_vec2(), tile.size()),
                    ),
            );
        }
        _ => {
            ui.painter()
                .rect_filled(tile, corner(theme.radius.lg), theme.tile(tile_tone(id)));
            let style = TextStyle::new(20.0, 600.0, 1.0).tracking(0.02);
            let letters = Block::single(ui, &initials(name), style, c.text);
            letters.paint(
                ui.painter(),
                Pos2::new(
                    tile.center().x - letters.width() / 2.0,
                    tile.center().y - letters.height() / 2.0,
                ),
            );
        }
    }
    // margin-bottom 8 on the tile, then the column gap of 8.
    ui.add_space(space::S2 + space::S2);
    centered(
        ui,
        if member {
            auth::invite_already_member()
        } else {
            auth::invite_invited_to()
        },
        text::UI,
        c.muted,
    );
    ui.add_space(space::S2);
    centered(ui, name, layout::HEADING, c.text);
    ui.add_space(space::S2);
    let count = server["member_count"].as_i64().unwrap_or(0);
    let people = auth::invite_members(count.max(0) as u64);
    let expiry = if member {
        None
    } else {
        preview["expires_at"].as_str().and_then(invite_expiry)
    };
    let line = match expiry {
        Some(expiry) => format!("{people} · {expiry}"),
        None => people,
    };
    centered(ui, &line, text::UI, c.muted);
}

fn centered(ui: &mut Ui, line: &str, style: TextStyle, color: Color32) {
    let width = ui.available_width();
    let mut job = text::job(line, style, color, Some(width));
    job.halign = egui::Align::Center;
    let block = Block::new(ui, job, style);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, block.height()), Sense::hover());
    block.paint(ui.painter(), pos2(rect.center().x, rect.top()));
}

/// `inviteExpiry`: "läuft morgen ab", "läuft in 6 Tagen ab".
pub fn invite_expiry(expires_at: &str) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(expires_at).ok()?;
    let left = (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_milliseconds();
    Some(expiry_text(left))
}

fn expiry_text(left_ms: i64) -> String {
    if left_ms <= 0 {
        return auth::invite_expiry_expired().into();
    }
    let (unit, size) = [(0, 86_400_000i64), (1, 3_600_000), (2, 60_000)]
        .into_iter()
        .find(|(_, ms)| left_ms >= *ms)
        .unwrap_or((2, 60_000));
    let n = ((left_ms as f64 / size as f64).round() as i64).max(1);
    let relative = match unit {
        0 => auth::invite_expiry_days(n),
        1 => format_relative(n, Unit::Hour),
        _ => format_relative(n, Unit::Minute),
    };
    auth::invite_expiry_in(&relative)
}

/// `DeadInvite`.
fn dead(ui: &mut Ui, app: &mut AppCtx, code: &str) {
    let banned = code == "banned";
    let message = if code == "not_found" {
        auth::invite_not_found()
    } else {
        error_message(code)
    };
    layout::frame(ui, app, "invite-dead", |ui, app| {
        layout::heading(
            ui,
            if banned {
                auth::invite_banned_title()
            } else {
                auth::invite_dead_title()
            },
        );
        let mut lines = vec![(message, false)];
        if !banned {
            lines.push((auth::invite_ask_for_new(), true));
        }
        result_lines(ui, &lines);
        ui.add_space(space::S6);
        if wide_button(ui, auth::invite_to_servers(), false) {
            app.nav.navigate("/");
        }
    });
}

/// `SkeletonCard` (min-height 300) while the preview loads.
fn skeleton(ui: &mut Ui, app: &mut AppCtx) {
    layout::frame(ui, app, "invite-loading", |ui, _app| {
        let theme = Theme::of(ui.ctx());
        let width = ui.available_width();
        // The card padding is 32 here as well; lines of 45/90/70% × 14.
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 300.0 - 66.0), Sense::hover());
        let mut y = rect.center().y - (14.0 * 3.0 + 24.0) / 2.0;
        for share in [0.45, 0.9, 0.7] {
            let line = Rect::from_min_size(pos2(rect.left(), y), Vec2::new(width * share, 14.0));
            ui.painter()
                .rect_filled(line, corner(theme.radius.sm), theme.c.surface);
            y += 14.0 + 12.0;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::expiry_text;

    #[test]
    fn expiry_like_intl() {
        assert_eq!(expiry_text(86_400_000 * 6), "läuft in 6 Tagen ab");
        assert_eq!(expiry_text(86_400_000), "läuft morgen ab");
        assert_eq!(expiry_text(3 * 3_600_000), "läuft in 3 Stunden ab");
        assert_eq!(expiry_text(0), "abgelaufen");
    }

    #[test]
    fn expiry_in_english() {
        let _locale = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
        assert_eq!(expiry_text(86_400_000 * 6), "expires in 6 days");
        assert_eq!(expiry_text(86_400_000), "expires tomorrow");
        assert_eq!(expiry_text(2 * 86_400_000), "expires in 2 days");
        assert_eq!(expiry_text(3_600_000), "expires in 1 hour");
        assert_eq!(expiry_text(0), "expired");
    }
}
