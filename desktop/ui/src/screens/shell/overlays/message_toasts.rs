//! New-message toasts at the top right (components/MessageToasts.tsx,
//! messages/toasts.ts, notify.ts, useMessageToasts.ts): a message by
//! someone else in a conversation you are not looking at, when its level
//! and mute allow it. A burst in one channel stacks onto one card. Also
//! hands the client's other notices (kicked, banned, errors) to the toasts.

use egui::{Id, Order, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{Message, Notice, NotificationLevel, SettingScope};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::icons::Icon;
use crate::route::Route;
use crate::screens::settings::prefs;
use crate::screens::shell::common::{self, line};
use crate::text::{self, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::IconButton;

/// `TOAST_HOLD_MS`, `TOAST_STACK_MS`, `TOAST_MAX`, `RELEASE_MIN_MS`.
const HOLD: f64 = 6.0;
const STACK: f64 = 1.6;
const MAX: usize = 4;
const RELEASE_MIN: f64 = 1.5;

#[derive(Clone, Debug)]
struct Toast {
    id: u64,
    channel: Uuid,
    server: Uuid,
    dm: bool,
    label: String,
    author: String,
    avatar: Option<String>,
    mentioned: bool,
    preview: String,
    count: u32,
    /// When the last message of the burst came.
    at: f64,
    /// Running: when it goes. Held (hover): the time it has left.
    until: f64,
    held: Option<f64>,
}

#[derive(Default)]
pub struct MessageToasts {
    items: Vec<Toast>,
    next: u64,
    /// The last Live start (`State::live_started`) handled.
    live_seen: u64,
}

/// The parts of a toast before it is stacked.
struct Incoming {
    channel: Uuid,
    server: Uuid,
    dm: bool,
    label: String,
    author: String,
    avatar: Option<String>,
    mentioned: bool,
    preview: String,
}

impl MessageToasts {
    fn push(&mut self, input: Incoming, now: f64) {
        // Same channel within the stack window: bump the count.
        if let Some(t) = self
            .items
            .iter_mut()
            .find(|t| t.channel == input.channel && now - t.at <= STACK)
        {
            t.count += 1;
            t.at = now;
            t.label = input.label;
            t.author = input.author;
            t.avatar = input.avatar;
            t.mentioned = input.mentioned;
            t.preview = input.preview;
            match t.held.as_mut() {
                Some(left) => *left = left.max(HOLD),
                None => t.until = now + HOLD,
            }
            return;
        }
        // A new burst replaces the channel's old card.
        self.items.retain(|t| t.channel != input.channel);
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            channel: input.channel,
            server: input.server,
            dm: input.dm,
            label: input.label,
            author: input.author,
            avatar: input.avatar,
            mentioned: input.mentioned,
            preview: input.preview,
            count: 1,
            at: now,
            until: now + HOLD,
            held: None,
        });
        let over = self.items.len().saturating_sub(MAX);
        self.items.drain(..over);
    }
}

/// `previewText`: whitespace collapsed, at most 140 characters.
pub fn preview_text(content: &str, has_attachment: bool, gif: bool) -> String {
    let trimmed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if !trimmed.is_empty() {
        let chars: Vec<char> = trimmed.chars().collect();
        return if chars.len() > 140 {
            chars[..137].iter().collect::<String>() + "…"
        } else {
            trimmed
        };
    }
    if gif {
        "GIF".into()
    } else if has_attachment {
        "Datei".into()
    } else {
        "Neue Nachricht".into()
    }
}

/// `readableMentions`: `<@id>` as `@Name`, unknown people as `@Unbekannt`.
fn readable(content: &str, names: &[(Uuid, String)]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        if tail.len() > 36
            && tail.as_bytes()[36] == b'>'
            && let Ok(id) = Uuid::parse_str(&tail[..36])
        {
            out.push('@');
            match names.iter().find(|(u, _)| *u == id) {
                Some((_, name)) => out.push_str(name),
                None => out.push_str("Unbekannt"),
            }
            rest = &tail[37..];
            continue;
        }
        out.push_str("<@");
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn system_text(message: &Message) -> Option<String> {
    let system = message.system.as_ref()?;
    Some(if system.kind == "live_started" {
        let voice = system
            .params
            .get("voice_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        format!("{} ist live in {voice}.", message.author.name)
    } else {
        format!("Systemnachricht von {}", message.author.name)
    })
}

/// Takes the client's notices: messages become toasts here, the rest the
/// app's toasts.
pub fn collect(ctx: &egui::Context, app: &mut AppCtx, toasts: &mut MessageToasts) {
    let now = ctx.input(|i| i.time);
    let viewing = match app.nav.route() {
        Route::Channel(_, c) | Route::Dm(c) => Some(*c),
        _ => None,
    };
    for notice in app.client.take_notices() {
        match notice {
            Notice::Kicked { server, text } | Notice::Banned { server, text } => {
                app.toasts.notify(text);
                let here = match app.nav.route() {
                    Route::Server(s) | Route::Channel(s, _) | Route::ServerSettings(s) => {
                        *s == server
                    }
                    _ => false,
                };
                if here {
                    app.nav.replace("/");
                }
            }
            Notice::Error(text) => app.toasts.error(text),
            Notice::Failed { failed, error } => app.toasts.failure(failed, &error),
            Notice::Message { server, message } => {
                if !prefs::get(ctx).message_toasts || viewing == Some(message.channel_id) {
                    continue;
                }
                if let Some(incoming) = incoming(app, server, &message) {
                    toasts.push(incoming, now);
                }
            }
        }
    }
    for incoming in live_toasts(ctx, app, &mut toasts.live_seen, viewing) {
        toasts.push(incoming, now);
    }
}

/// Whether I sit in that voice channel, here or on another device.
pub fn sits_in(app: &AppCtx, server: Uuid, channel: Uuid) -> bool {
    let joined = app.call.with_state(crate::screens::voice::session::joined);
    joined == Some((server, channel))
        || app.client.with_state(|s| {
            let me = s.me().map(|m| m.id);
            me.and_then(|me| s.voice.get(&server).and_then(|r| r.get(&me)))
                .is_some_and(|seat| seat.channel_id == channel)
        })
}

/// The new Lives since `seen` that show as a toast (useLiveNotices.ts):
/// "Ada ist live in Lounge." under the server's name.
fn live_toasts(
    ctx: &egui::Context,
    app: &AppCtx,
    seen: &mut u64,
    viewing: Option<Uuid>,
) -> Vec<Incoming> {
    let starts: Vec<gelabber_client::store::LiveStart> = app.client.with_state(|s| {
        // A new account starts its count anew.
        if s.live_started.last().is_none_or(|l| l.seq < *seen) {
            *seen = 0;
        }
        s.live_started
            .iter()
            .filter(|l| l.seq > *seen)
            .cloned()
            .collect()
    });
    let Some(last) = starts.last() else {
        return Vec::new();
    };
    *seen = last.seq;
    let device = crate::platform::notify::rules::LiveDevice {
        toasts: prefs::get(ctx).message_toasts,
        desktop: false,
        in_front: ctx.input(|i| i.focused),
        viewing,
    };
    let now = app.client.http().server_now();
    let mut out = Vec::new();
    for start in starts {
        let here = sits_in(app, start.server, start.channel);
        let incoming = app.client.with_state(|s| {
            let (toast, _) =
                crate::platform::notify::rules::live_decision(s, &start, here, device, now);
            if !toast {
                return None;
            }
            let detail = s.detail(start.server)?;
            let voice = detail.channels.iter().find(|c| c.id == start.channel)?;
            let member = detail.members.iter().find(|m| m.user_id == start.user);
            let author = member.map_or_else(|| "Jemand".to_owned(), |m| m.name.clone());
            Some(Incoming {
                channel: start.channel,
                server: start.server,
                dm: false,
                label: detail.server.name.clone(),
                preview: crate::platform::notify::rules::live_text(&author, &voice.name),
                author,
                avatar: member.and_then(|m| m.avatar_url.clone()),
                mentioned: false,
            })
        });
        out.extend(incoming);
    }
    out
}

/// Whether and how a message shows (`wantsNotification`, `channelLabel`).
fn incoming(app: &AppCtx, server: Uuid, message: &Message) -> Option<Incoming> {
    let channel = message.channel_id;
    let at = app.client.http().server_now();
    app.client.with_state(|s| {
        let me = s.me()?.id;
        if message.author.id == me {
            return None;
        }
        let dm = s.dm(channel);
        let mentioned = message.mention_everyone || message.mentions.contains(&me);
        let (level, muted) = match s.read.get(&channel) {
            Some(row) => (row.level, row.is_muted(at)),
            None => {
                // A conversation too new for a row: its setting and its server's.
                let find = |scope, id| {
                    s.notification_settings
                        .iter()
                        .find(move |r| r.scope == scope && r.id == id)
                };
                let own = find(SettingScope::Channel, channel);
                let srv = dm
                    .is_none()
                    .then(|| find(SettingScope::Server, server))
                    .flatten();
                let holds = |r: Option<&gelabber_client::NotificationSetting>| {
                    r.is_some_and(|r| r.muted && r.muted_until.is_none_or(|u| u > at))
                };
                (
                    own.and_then(|r| r.level)
                        .or(srv.and_then(|r| r.level))
                        .unwrap_or_default(),
                    holds(own) || holds(srv),
                )
            }
        };
        let wanted = !muted
            && match level {
                NotificationLevel::None => false,
                NotificationLevel::Mentions => dm.is_some() || mentioned,
                NotificationLevel::All => true,
            };
        if !wanted {
            return None;
        }
        let (label, names) = match dm {
            Some(dm) => (
                // The author already names a DM: the place says what it is.
                "Direktnachricht".to_owned(),
                vec![
                    (me, s.me().map(|m| m.name.clone()).unwrap_or_default()),
                    (dm.peer.id, dm.peer.name.clone()),
                ],
            ),
            None => {
                let detail = s.detail(server);
                let label = detail
                    .and_then(|d| d.channels.iter().find(|c| c.id == channel))
                    .map_or_else(|| "Kanal".to_owned(), |c| format!("#{}", c.name));
                let names = detail
                    .map(|d| {
                        d.members
                            .iter()
                            .map(|m| (m.user_id, m.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                (label, names)
            }
        };
        let preview = preview_text(
            &system_text(message).unwrap_or_else(|| {
                readable(
                    &crate::screens::chat::markdown::plain_text(&message.content),
                    &names,
                )
            }),
            !message.attachments.is_empty(),
            message.gif.is_some(),
        );
        Some(Incoming {
            channel,
            server,
            dm: dm.is_some(),
            label,
            author: message.author.name.clone(),
            avatar: message.author.avatar_url.clone(),
            mentioned,
            preview,
        })
    })
}

const WHO: TextStyle = text::UI.weight(600.0);

/// Paints the stack: top 64, right 16, 352 wide, newest at the bottom.
pub fn show(ui: &Ui, app: &mut AppCtx, toasts: &mut MessageToasts) {
    if toasts.items.is_empty() {
        return;
    }
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    toasts.items.retain(|t| t.held.is_some() || t.until > now);
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let screen = ctx.content_rect();
    let width = 352.0_f32.min(screen.width() - 32.0);
    let right = screen.right() - 16.0;
    let mut closed = None;
    let mut opened = None;
    let mut hovered_ids = Vec::new();
    egui::Area::new(Id::unique("gelabber-message-toasts"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(&ctx, |ui| {
            let mut y = screen.top() + 64.0;
            for toast in &toasts.items {
                let height = 2.0 + 24.0 + 16.8 + 2.0 + 17.55 + 2.0 + 21.0;
                let rect = Rect::from_min_size(pos2(right - width, y), Vec2::new(width, height));
                y = rect.bottom() + 8.0;
                let card = ui.interact(
                    rect,
                    Id::unique(("message-toast", toast.id)),
                    Sense::click(),
                );
                let where_ = if toast.count > 1 {
                    format!("{} · {} neu", toast.label, toast.count)
                } else {
                    toast.label.clone()
                };
                card.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        format!(
                            "Öffnen: {where_}, {}{}: {}",
                            toast.author,
                            if toast.mentioned {
                                " hat dich erwähnt"
                            } else {
                                ""
                            },
                            toast.preview
                        ),
                    )
                });
                let hover = ui.rect_contains_pointer(rect);
                if hover {
                    hovered_ids.push(toast.id);
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let fill = if hover {
                    mix(c.elevated, c.text, 0.08)
                } else {
                    c.elevated
                };
                let painter = ui.painter();
                painter.add(theme.shadow_2.as_shape(rect, corner(theme.radius.lg)));
                painter.rect(
                    rect,
                    corner(theme.radius.lg),
                    fill,
                    Stroke::new(1.0, c.border),
                    StrokeKind::Inside,
                );
                let avatar = Rect::from_min_size(rect.min + Vec2::splat(13.0), Vec2::splat(32.0));
                common::avatar(
                    ui,
                    app.images,
                    avatar,
                    &toast.author,
                    toast.avatar.as_deref(),
                    12.0,
                    1.0,
                    fill,
                );
                let left = avatar.right() + 12.0;
                let max = rect.right() - 41.0 - left;
                let mut ty = rect.top() + 13.0;
                let w = line(ui, &where_, text::META, c.muted, max);
                w.paint(painter, pos2(left, ty));
                ty += text::META.line_px() + 2.0;
                let who = line(ui, &toast.author, WHO, c.text, max);
                who.paint(painter, pos2(left, ty));
                if toast.mentioned {
                    let small = line(
                        ui,
                        " hat dich erwähnt",
                        text::UI.weight(400.0),
                        c.muted,
                        (max - who.width()).max(0.0),
                    );
                    small.paint(painter, pos2(left + who.width(), ty));
                }
                ty += WHO.line_px() + 2.0;
                line(ui, &toast.preview, text::BODY, c.muted, max).paint(painter, pos2(left, ty));
                let close = Rect::from_min_size(
                    pos2(rect.right() - 6.0 - 28.0, rect.top() + 6.0),
                    Vec2::splat(28.0),
                );
                if ui
                    .add(
                        IconButton::new(Icon::Close, "Schließen")
                            .size(28.0)
                            .icon_size(14.0)
                            .at(close),
                    )
                    .clicked()
                {
                    closed = Some(toast.id);
                } else if card.clicked() {
                    opened = Some(toast.clone());
                }
            }
        });
    // Hover holds a toast; back from it there is always a moment left.
    for t in &mut toasts.items {
        let held = hovered_ids.contains(&t.id);
        match (held, t.held) {
            (true, None) => t.held = Some(t.until - now),
            (false, Some(left)) => {
                t.until = now + left.max(RELEASE_MIN);
                t.held = None;
            }
            _ => {}
        }
    }
    if let Some(id) = closed {
        toasts.items.retain(|t| t.id != id);
    }
    if let Some(t) = opened {
        toasts.items.retain(|x| x.id != t.id);
        if t.dm {
            app.nav.navigate(&format!("/d/{}", t.channel));
        } else {
            app.nav
                .navigate(&format!("/s/{}/c/{}", t.server, t.channel));
        }
    }
    if let Some(next) = toasts
        .items
        .iter()
        .filter(|t| t.held.is_none())
        .map(|t| t.until)
        .reduce(f64::min)
    {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming(channel: Uuid) -> Incoming {
        Incoming {
            channel,
            server: Uuid::nil(),
            dm: false,
            label: "#allgemein".into(),
            author: "Jonas".into(),
            avatar: None,
            mentioned: false,
            preview: "Hallo".into(),
        }
    }

    #[test]
    fn bursts_stack_and_the_stack_holds_four() {
        let mut t = MessageToasts::default();
        let a = Uuid::from_u128(1);
        t.push(incoming(a), 0.0);
        t.push(incoming(a), 1.0);
        assert_eq!(t.items.len(), 1);
        assert_eq!(t.items[0].count, 2);
        // After the window a new burst replaces the card.
        t.push(incoming(a), 5.0);
        assert_eq!(t.items.len(), 1);
        assert_eq!(t.items[0].count, 1);
        for n in 2..=6 {
            t.push(incoming(Uuid::from_u128(n)), 6.0);
        }
        assert_eq!(t.items.len(), 4);
    }

    #[test]
    fn previews_like_the_web() {
        assert_eq!(preview_text("  a \n b ", false, false), "a b");
        assert_eq!(preview_text("", true, false), "Datei");
        assert_eq!(preview_text("", false, true), "GIF");
        assert_eq!(preview_text("", false, false), "Neue Nachricht");
        let long = "x".repeat(200);
        assert_eq!(preview_text(&long, false, false).chars().count(), 138);
        let id = Uuid::from_u128(7);
        assert_eq!(
            readable(
                &format!("hi <@{id}> <@{}>", Uuid::from_u128(8)),
                &[(id, "Lena".into())]
            ),
            "hi @Lena @Unbekannt"
        );
    }
}
