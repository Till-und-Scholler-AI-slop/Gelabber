//! The member panel beside channels and the overview
//! (components/MemberPanel.tsx, index.css `.member-panel`): Online,
//! Abwesend and Offline groups with counts, one row per member with
//! presence, status line, voice line and voice state icons.

use egui::{Id, Rect, Ui, Vec2, pos2};
use gelabber_client::PresenceStatus;
use uuid::Uuid;

use super::common::{self, baseline_top, line, paint_mid};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block, TextStyle};
use crate::theme::Theme;
use crate::widgets::{hairline_h, hairline_v};

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.panel);
    hairline_v(&painter, &theme, rect.left(), rect.top()..=rect.bottom());

    struct Entry {
        id: Uuid,
        name: String,
        avatar: Option<String>,
        status: Option<String>,
        voice: Option<(String, bool, bool)>,
        live: bool,
    }
    let data = app.client.with_state(|s| {
        let detail = s.detail(server)?;
        let me = s.me();
        let names = |channel: Uuid| {
            detail
                .channels
                .iter()
                .find(|c| c.id == channel)
                .map(|c| c.name.clone())
        };
        let groups: Vec<(PresenceStatus, Vec<Entry>)> = s
            .member_groups(server)
            .into_iter()
            .map(|(status, members)| {
                let entries = members
                    .into_iter()
                    .map(|m| {
                        let own = me.filter(|u| u.id == m.user_id);
                        let occupant = if own.is_some() {
                            (s.voice_self.server == Some(server))
                                .then_some(s.voice_self.channel)
                                .flatten()
                                .map(|ch| (ch, s.voice_self.muted, s.voice_self.deafened))
                        } else {
                            s.voice
                                .get(&server)
                                .and_then(|r| r.get(&m.user_id))
                                .map(|o| (o.channel_id, o.muted, o.deafened))
                        };
                        let live = s
                            .live
                            .get(&server)
                            .is_some_and(|l| l.values().any(|users| users.contains(&m.user_id)));
                        Entry {
                            id: m.user_id,
                            name: own.map_or_else(|| m.name.clone(), |u| u.name.clone()),
                            avatar: own
                                .map_or_else(|| m.avatar_url.clone(), |u| u.avatar_url.clone()),
                            status: own
                                .map_or_else(|| m.status_text.clone(), |u| u.status_text.clone())
                                .filter(|t| !t.is_empty()),
                            voice: occupant.map(|(ch, muted, deaf)| {
                                (names(ch).unwrap_or_default(), muted, deaf)
                            }),
                            live,
                        }
                    })
                    .collect();
                (status, entries)
            })
            .collect();
        Some((detail.members.len(), groups, me.map(|m| m.id)))
    });
    let Some((count, groups, me)) = data else {
        return;
    };

    // Heading: "Mitglieder" and the count.
    let title = Block::single(
        ui,
        "Mitglieder",
        TextStyle::new(15.0, 650.0, 1.5).tracking(-0.2 / 15.0),
        c.text,
    );
    paint_mid(
        &painter,
        &title,
        rect.left() + 1.0 + 16.0,
        rect.top() + 25.5,
    );
    let n = Block::single(
        ui,
        &count.to_string(),
        TextStyle::new(12.0, 400.0, 1.5),
        c.muted,
    );
    paint_mid(
        &painter,
        &n,
        rect.right() - 16.0 - n.width(),
        rect.top() + 25.5,
    );
    hairline_h(
        &painter,
        &theme,
        rect.left() + 1.0..=rect.right(),
        rect.top() + 51.0,
    );

    let list = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 52.0), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let heading_style = TextStyle::new(11.0, 650.0, 1.5).tracking(0.6 / 11.0);
    let row_h = |e: &Entry| {
        let mut h = 4.0 + 21.0 + 4.0;
        if e.status.is_some() {
            h += 2.0 + 18.0;
        }
        if e.live || e.voice.is_some() {
            h += 3.0 + 15.4;
        }
        f32::max(h, 42.0)
    };
    let content_h: f32 = 14.0
        + groups
            .iter()
            .map(|(_, e)| 8.0 + 16.5 + 6.0 + e.iter().map(|x| row_h(x) + 2.0).sum::<f32>())
            .sum::<f32>()
        + 18.0 * groups.len().saturating_sub(1) as f32
        + 16.0;
    egui::ScrollArea::vertical()
        .id_salt(("members-scroll", server))
        .max_height(list.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let (area, _) =
                ui.allocate_exact_size(Vec2::new(list.width(), content_h), egui::Sense::hover());
            let left = area.left() + 10.0;
            let right = area.right() - 10.0;
            let mut y = area.top() + 14.0;
            for (gi, (status, entries)) in groups.iter().enumerate() {
                y += if gi == 0 { 8.0 } else { 18.0 - 2.0 };
                let label = match status {
                    PresenceStatus::Online => "Online",
                    PresenceStatus::Idle => "Abwesend",
                    PresenceStatus::Offline => "Offline",
                };
                let heading = Block::single(
                    ui,
                    &format!("{} — {}", label, entries.len()).to_uppercase(),
                    heading_style,
                    c.muted,
                );
                heading.paint(ui.painter(), pos2(left + 6.0, y));
                y += 16.5 + 6.0;
                let offline = *status == PresenceStatus::Offline;
                for e in entries {
                    let h = row_h(e);
                    let row = Rect::from_min_size(pos2(left, y), Vec2::new(right - left, h));
                    y += h + 2.0;
                    let response = common::click(ui, row, Id::unique(("member-row", server, e.id)))
                        .on_hover_text(format!("Profil von {}", e.name));
                    let under = if response.hovered() {
                        common::fill(ui.painter(), row, theme.radius.md, c.hover);
                        c.hover
                    } else {
                        c.panel
                    };
                    let avatar = Rect::from_min_size(
                        pos2(row.left() + 6.0, row.center().y - 16.0),
                        Vec2::splat(32.0),
                    );
                    common::avatar(
                        ui,
                        app.images,
                        avatar,
                        &e.name,
                        e.avatar.as_deref(),
                        13.0,
                        if offline { 0.55 } else { 1.0 },
                        under,
                    );
                    common::presence_dot(ui.painter(), &theme, avatar, common::presence(*status));
                    let text_left = avatar.right() + 10.0;
                    let text_right = row.right() - 6.0 - 22.0 - 10.0;
                    let mut lines_h = 21.0;
                    if e.status.is_some() {
                        lines_h += 2.0 + 18.0;
                    }
                    if e.live || e.voice.is_some() {
                        lines_h += 3.0 + 15.4;
                    }
                    let mut ty = row.center().y - lines_h / 2.0;
                    let name_color = if offline { c.muted } else { c.text };
                    let own = me == Some(e.id);
                    let small_style = TextStyle::new(11.0, 400.0, 1.5);
                    let du = own.then(|| Block::single(ui, " · Du", small_style, c.muted));
                    let du_w = du.as_ref().map_or(0.0, Block::width);
                    let name = line(
                        ui,
                        &e.name,
                        text::BODY.weight(600.0),
                        name_color,
                        text_right - text_left - du_w,
                    );
                    name.paint(ui.painter(), pos2(text_left, ty));
                    if let Some(du) = du {
                        let top = baseline_top(text::BODY, ty, small_style);
                        du.paint(ui.painter(), pos2(text_left + name.width(), top));
                    }
                    ty += 21.0;
                    if let Some(status) = &e.status {
                        ty += 2.0;
                        let block = line(
                            ui,
                            status,
                            TextStyle::new(12.0, 400.0, 1.5),
                            c.muted,
                            text_right - text_left,
                        );
                        block.paint(ui.painter(), pos2(text_left, ty));
                        ty += 18.0;
                    }
                    let voice_name = e.voice.as_ref().map(|(n, _, _)| n.as_str());
                    let voice_line = if e.live {
                        Some(match voice_name {
                            Some(n) => format!(
                                "Live in {}",
                                if n.is_empty() { "einem Sprachkanal" } else { n }
                            ),
                            None => "Live".to_owned(),
                        })
                    } else {
                        voice_name.map(|n| {
                            if n.is_empty() {
                                "Im Sprachkanal".to_owned()
                            } else {
                                n.to_owned()
                            }
                        })
                    };
                    if let Some(voice_line) = voice_line {
                        ty += 3.0;
                        let block = Block::wrapped(
                            ui,
                            &voice_line,
                            TextStyle::new(11.0, 400.0, 1.4),
                            c.muted,
                            text_right - text_left,
                        );
                        block.paint(ui.painter(), pos2(text_left, ty));
                    }
                    // VoiceStateIcons: a 32px box right-aligned in the 22px column.
                    let column_right = row.right() - 6.0;
                    if let Some((channel, muted, deafened)) = &e.voice {
                        let slot2 = Rect::from_min_size(
                            pos2(column_right - 14.0, row.center().y - 7.0),
                            Vec2::splat(14.0),
                        );
                        let slot1 = slot2.translate(Vec2::new(-16.0, 0.0));
                        let icon = if *muted || *deafened {
                            Icon::MicOff
                        } else {
                            Icon::Speaker
                        };
                        icons::paint(ui.painter(), icon, slot1.min, 14.0, c.muted);
                        if *deafened {
                            icons::paint(ui.painter(), Icon::HeadsetOff, slot2.min, 14.0, c.muted);
                        }
                        let hint = if *deafened {
                            format!("Taub in {channel}")
                        } else if *muted {
                            format!("Stumm in {channel}")
                        } else {
                            format!("In {channel}")
                        };
                        let _ = ui
                            .interact(
                                slot1.union(slot2),
                                Id::unique(("member-voice", e.id)),
                                egui::Sense::hover(),
                            )
                            .on_hover_text(hint);
                    }
                    if e.live {
                        icons::paint(
                            ui.painter(),
                            Icon::Live,
                            pos2(column_right - 11.0 - 10.0, row.center().y + 2.5),
                            20.0,
                            c.muted,
                        );
                    }
                }
            }
        });
}
