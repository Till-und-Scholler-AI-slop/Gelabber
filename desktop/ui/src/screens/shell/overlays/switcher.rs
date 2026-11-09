//! The quick switcher (components/QuickSwitcher.tsx, quickSwitch.ts):
//! every server, channel and DM by typing part of its name, unread first;
//! and the shortcut sheet (`ShortcutSheet`, shellShortcuts.ts).

use egui::{Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{Badge, ChannelKind};
use uuid::Uuid;

use super::picker::{self, PickerState, Row, RowIcon};
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::initials::{tile_initials, tile_tone};
use crate::route::Route;
use crate::screens::shell::{channel_sidebar, state};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Modal, hairline_h};

#[derive(Default)]
pub struct Switcher {
    picker: PickerState,
    /// Server details were asked for once.
    loading: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Server,
    Text,
    Voice,
    Dm,
}

#[derive(Clone, Copy)]
enum Target {
    Server(Uuid),
    Channel(Uuid, Uuid),
    Dm(Uuid),
}

struct Entry {
    kind: Kind,
    label: String,
    detail: String,
    target: Target,
    badge: Badge,
    avatar: Option<String>,
}

/// `switchEntries`: DMs, then per server its channels and the server; the
/// conversation open now is left out.
fn entries(app: &AppCtx, current: Option<Uuid>) -> (Vec<Entry>, Vec<Uuid>) {
    let now = chrono::Utc::now();
    app.client.with_state(|s| {
        let mut out = Vec::new();
        for dm in &s.dms {
            if Some(dm.id) == current {
                continue;
            }
            out.push(Entry {
                kind: Kind::Dm,
                label: dm.peer.name.clone(),
                detail: "Direktnachricht".into(),
                target: Target::Dm(dm.id),
                badge: s.channel_badge(dm.id, now),
                avatar: dm.peer.avatar_url.clone(),
            });
        }
        let mut missing = Vec::new();
        for server in &s.servers {
            let mut total = Badge::default();
            match s.detail(server.id) {
                Some(detail) => {
                    for id in channel_sidebar::channel_order(detail) {
                        let Some(ch) = detail.channels.iter().find(|c| c.id == id) else {
                            continue;
                        };
                        let badge = s.channel_badge(id, now);
                        total = total + badge;
                        if Some(id) == current {
                            continue;
                        }
                        out.push(Entry {
                            kind: if ch.kind == ChannelKind::Voice {
                                Kind::Voice
                            } else {
                                Kind::Text
                            },
                            label: ch.name.clone(),
                            detail: server.name.clone(),
                            target: Target::Channel(server.id, id),
                            badge,
                            avatar: None,
                        });
                    }
                }
                None => missing.push(server.id),
            }
            out.push(Entry {
                kind: Kind::Server,
                label: server.name.clone(),
                detail: "Server".into(),
                target: Target::Server(server.id),
                badge: total,
                avatar: server.icon_url.clone(),
            });
        }
        (out, missing)
    })
}

fn unread_rank(b: Badge) -> u8 {
    if b.count > 0 {
        2
    } else if b.dot {
        1
    } else {
        0
    }
}

/// `rankEntries`: best match first, unread leading among equals, at most 50.
fn rank(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let mut scored: Vec<(u8, usize, Entry)> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| picker::match_score(query, &e.label).map(|s| (s, i, e)))
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(unread_rank(b.2.badge).cmp(&unread_rank(a.2.badge)))
            .then(b.2.badge.count.cmp(&a.2.badge.count))
            .then(a.1.cmp(&b.1))
    });
    scored.into_iter().take(50).map(|(_, _, e)| e).collect()
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, s: &mut Switcher) -> bool {
    let ctx = ui.ctx().clone();
    let current = match app.nav.route() {
        Route::Channel(_, c) | Route::Dm(c) => Some(*c),
        _ => None,
    };
    let (all, missing) = entries(app, current);
    if !s.loading {
        s.loading = true;
        for id in missing {
            app.client.refresh_server(id);
        }
    }
    let ranked = rank(all, &s.picker.query);
    let theme = Theme::of(&ctx);
    let rows: Vec<Row> = ranked
        .iter()
        .map(|e| Row {
            label: if e.kind == Kind::Text {
                format!("#{}", e.label)
            } else {
                e.label.clone()
            },
            detail: Some(e.detail.clone()),
            icon: match (e.kind, e.target) {
                (Kind::Dm, _) => RowIcon::Avatar {
                    name: e.label.clone(),
                    url: e.avatar.clone(),
                },
                (Kind::Server, Target::Server(id)) => RowIcon::Tile {
                    initials: tile_initials(&e.label),
                    tint: theme.tile(tile_tone(&id.to_string())),
                    url: e.avatar.clone(),
                },
                (Kind::Voice, _) => RowIcon::Glyph(Icon::Speaker),
                _ => RowIcon::Glyph(Icon::Hash),
            },
            // Voice rooms carry no unread mark.
            badge: if e.kind == Kind::Voice {
                Badge::default()
            } else {
                e.badge
            },
        })
        .collect();
    let empty = format!("Nichts gefunden für „{}“.", s.picker.query.trim());
    let mut picked = None;
    let output = Modal::new("quick-switcher", "Wechseln zu").show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            picked = picker::show(
                ui,
                app.images,
                "quick-switcher",
                &mut s.picker,
                "Server, Kanal oder Unterhaltung",
                "Server, Kanal oder Unterhaltung suchen",
                &rows,
                &empty,
                false,
            );
            // The hint: body gap 20 + margin-top 12.
            ui.add_space(32.0);
            text::label(
                ui,
                "↑ ↓ auswählen · Enter öffnen · Esc schließen",
                text::META,
                theme.c.muted,
            );
        },
        None,
    );
    if let Some(i) = picked {
        go(ui, app, ranked[i].target);
        return false;
    }
    !output.close_requested
}

/// Opens a pick: a server goes to the channel last open there, unless
/// someone is in its voice rooms (`railChannel`).
fn go(ui: &Ui, app: &mut AppCtx, target: Target) {
    match target {
        Target::Dm(c) => app.nav.navigate(&format!("/d/{c}")),
        Target::Channel(s, c) => app.nav.navigate(&format!("/s/{s}/c/{c}")),
        Target::Server(server) => {
            let user = app.client.with_state(|s| s.me().map(|m| m.id));
            let in_voice = app
                .client
                .with_state(|s| s.voice.get(&server).is_some_and(|r| !r.is_empty()));
            let last = (!in_voice)
                .then(|| state::memory(ui.ctx(), user).channels.get(&server).copied())
                .flatten();
            match last {
                Some(c) => app.nav.navigate(&format!("/s/{server}/c/{c}")),
                None => app.nav.navigate(&format!("/s/{server}")),
            }
        }
    }
}

/// One shortcut: its keys and what they do.
pub type Shortcut = (Vec<String>, &'static str);

/// `shortcutGroups` with the key names of a non-Mac keyboard: the
/// workspace's keys, with the desktop app's Strg+Umschalt+S, then the
/// call's (`call_bar::shortcuts`, `kino_key`, the large view's M and
/// Leertaste). `push_to_talk` is the key push-to-talk holds, when on.
pub fn shortcut_groups(push_to_talk: Option<&str>) -> Vec<(&'static str, Vec<Shortcut>)> {
    let keys = |k: &[&str]| k.iter().map(|k| (*k).to_owned()).collect::<Vec<_>>();
    let mut voice = vec![
        (
            keys(&["Strg", "Umschalt", "M"]),
            "Stummschaltung ein oder aus",
        ),
        (
            keys(&["Strg", "Umschalt", "D"]),
            "Taubschaltung ein oder aus",
        ),
    ];
    if let Some(key) = push_to_talk {
        voice.push((vec![key.to_owned()], "Zum Sprechen gedrückt halten"));
    }
    voice.push((keys(&["K"]), "Kino ein oder aus"));
    voice.push((
        keys(&["M / Leertaste"]),
        "Im Vollbild: Stummschaltung ein oder aus",
    ));
    vec![
        (
            "Allgemein",
            vec![
                (
                    keys(&["Strg", "K"]),
                    "Server, Kanal oder Unterhaltung öffnen",
                ),
                (keys(&["Strg", "F"]), "In der Unterhaltung suchen"),
                (keys(&["Alt", "↑ / ↓"]), "Vorheriger oder nächster Kanal"),
                (
                    keys(&["Alt", "Umschalt", "↑ / ↓"]),
                    "Vorheriger oder nächster ungelesener Kanal",
                ),
                (
                    keys(&["Strg", "Alt", "↑ / ↓"]),
                    "Vorheriger oder nächster Server",
                ),
                // Whatever is open takes Esc first (chat `overlay_open`).
                (
                    keys(&["Esc"]),
                    "Schließen, sonst Unterhaltung als gelesen markieren",
                ),
                (
                    keys(&["Umschalt", "Esc"]),
                    "Ganzen Server als gelesen markieren",
                ),
                // The desktop app's own (setup.rs), as "Server wechseln …".
                (
                    keys(&["Strg", "Umschalt", "S"]),
                    "Mit einem anderen Gelabber-Server verbinden",
                ),
                (keys(&["Strg", "/"]), "Diese Übersicht"),
            ],
        ),
        ("Im Sprachkanal", voice),
    ]
}

const KBD: TextStyle = TextStyle::new(12.0, 600.0, 1.4);

/// "Tastenkürzel": each group under its caps heading (`.shortcut-group-title`,
/// gap 20 between groups), description left, keys right, a hairline
/// between rows.
pub fn shortcut_sheet(ui: &mut Ui) -> bool {
    let ctx = ui.ctx().clone();
    let push_to_talk = crate::screens::settings::media::push_to_talk_key_name(&ctx);
    let groups = shortcut_groups(push_to_talk.as_deref());
    let output = Modal::new("shortcuts", "Tastenkürzel").show(
        &ctx,
        |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            let width = ui.available_width();
            for (g, (title, shortcuts)) in groups.iter().enumerate() {
                if g > 0 {
                    ui.add_space(20.0);
                }
                let heading = Block::single(ui, &title.to_uppercase(), text::CAPS, c.muted);
                let (head, response) = ui.allocate_exact_size(
                    Vec2::new(width, heading.height() + 2.0),
                    egui::Sense::hover(),
                );
                heading.paint(ui.painter(), head.min);
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, *title));
                for (i, (keys, label)) in shortcuts.iter().enumerate() {
                    // The keys first: the description gets what is left.
                    let blocks: Vec<Block> = keys
                        .iter()
                        .map(|k| Block::single(ui, k, KBD, c.text))
                        .collect();
                    let kbd_w: Vec<f32> = blocks
                        .iter()
                        .map(|b| (b.width() + 16.0).max(26.0))
                        .collect();
                    let keys_w = kbd_w.iter().sum::<f32>() + 4.0 * (blocks.len() - 1) as f32;
                    let desc = Block::wrapped(ui, label, text::UI, c.text, width - keys_w - 16.0);
                    let kbd_h = KBD.line_px() + 4.0 + 3.0;
                    let h = 20.0 + desc.height().max(kbd_h);
                    let last = i + 1 == shortcuts.len();
                    let (row, _) = ui.allocate_exact_size(
                        Vec2::new(width, h + if last { 0.0 } else { 1.0 }),
                        egui::Sense::hover(),
                    );
                    let mid = row.top() + h / 2.0;
                    let painter = ui.painter();
                    desc.paint(painter, pos2(row.left(), mid - desc.height() / 2.0));
                    let mut x = row.right() - keys_w;
                    for (b, w) in blocks.iter().zip(&kbd_w) {
                        let r =
                            Rect::from_min_size(pos2(x, mid - kbd_h / 2.0), Vec2::new(*w, kbd_h));
                        let r = crate::widgets::snap(ui.ctx(), r);
                        painter.rect(
                            r,
                            corner(theme.radius.xs),
                            c.border_strong,
                            Stroke::NONE,
                            StrokeKind::Inside,
                        );
                        // 1px sides and top, a 2px bottom edge.
                        let face = Rect::from_min_max(
                            r.min + Vec2::splat(1.0),
                            r.max - Vec2::new(1.0, 2.0),
                        );
                        painter.rect_filled(face, corner(theme.radius.xs - 1.0), c.surface);
                        b.paint(painter, pos2(r.center().x - b.width() / 2.0, r.top() + 3.0));
                        x += w + 4.0;
                    }
                    if !last {
                        hairline_h(
                            painter,
                            &theme,
                            row.left()..=row.right(),
                            row.bottom() - 1.0,
                        );
                    }
                }
            }
        },
        None,
    );
    !output.close_requested
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, count: i64, dot: bool) -> Entry {
        Entry {
            kind: Kind::Text,
            label: label.into(),
            detail: String::new(),
            target: Target::Dm(Uuid::nil()),
            badge: Badge { count, dot },
            avatar: None,
        }
    }

    /// The sheet lists what the keys do: the call's in their own group,
    /// push-to-talk only with a key, as `shortcutGroups` on the web.
    #[test]
    fn the_sheet_groups_the_call_keys() {
        let groups = shortcut_groups(None);
        let titles: Vec<&str> = groups.iter().map(|(t, _)| *t).collect();
        assert_eq!(titles, ["Allgemein", "Im Sprachkanal"]);
        let voice: Vec<String> = groups[1].1.iter().map(|(k, _)| k.join("+")).collect();
        assert_eq!(
            voice,
            ["Strg+Umschalt+M", "Strg+Umschalt+D", "K", "M / Leertaste"]
        );
        assert!(
            groups[0]
                .1
                .iter()
                .any(|(k, _)| k.join("+") == "Strg+Umschalt+S")
        );
        let with_ptt = shortcut_groups(Some("F"));
        assert!(
            with_ptt[1]
                .1
                .iter()
                .any(|(k, l)| k == &["F"] && *l == "Zum Sprechen gedrückt halten")
        );
    }

    #[test]
    fn unread_leads_among_equals() {
        let ranked = rank(
            vec![
                entry("allgemein", 0, false),
                entry("spiele", 0, true),
                entry("Jonas", 1, false),
            ],
            "",
        );
        let labels: Vec<&str> = ranked.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["Jonas", "spiele", "allgemein"]);
        let ranked = rank(
            vec![entry("musik", 0, true), entry("spiele", 0, false)],
            "s",
        );
        assert_eq!(ranked[0].label, "spiele");
    }
}
