//! The member panel beside channels and the overview
//! (components/MemberPanel.tsx, index.css `.member-panel`): Online,
//! Abwesend and Offline groups with counts, one row per member with
//! presence, status line, voice line and voice state icons.

use egui::{Id, Rect, Ui, Vec2, pos2};
use gelabber_client::PresenceStatus;
use gelabber_i18n::members;
use uuid::Uuid;

use super::common::{self, baseline_top, line, paint_mid};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block, TextStyle};
use crate::theme::Theme;
use crate::widgets::{hairline_h, hairline_v};

struct Entry {
    id: Uuid,
    name: String,
    avatar: Option<String>,
    status: Option<String>,
    voice: Option<(String, bool, bool)>,
    live: bool,
}

impl Entry {
    /// The line under the name and status: "Live in …" or the voice
    /// channel's name (MemberPanel's `<small>`).
    fn voice_line(&self) -> Option<String> {
        let channel = self.voice.as_ref().map(|(n, _, _)| n.as_str());
        if self.live {
            return Some(match channel {
                Some(n) => members::row_live_in(if n.is_empty() {
                    members::row_a_voice_channel()
                } else {
                    n
                }),
                None => members::row_live().to_owned(),
            });
        }
        channel.map(|n| {
            if n.is_empty() {
                members::row_in_voice().to_owned()
            } else {
                n.to_owned()
            }
        })
    }
}

/// The row's text column at its width: the voice line wraps there
/// (`.member-description > small`, `overflow-wrap: anywhere`) and the row
/// grows with it, as the web list measures its rows (GEL-63).
struct RowText {
    voice: Option<Block>,
    /// Name, status line and voice line together.
    lines_h: f32,
}

impl RowText {
    fn new(ui: &Ui, e: &Entry, width: f32, color: egui::Color32) -> Self {
        let voice = e
            .voice_line()
            .map(|line| Block::wrapped(ui, &line, TextStyle::new(11.0, 400.0, 1.4), color, width));
        let mut lines_h = 21.0;
        if e.status.is_some() {
            lines_h += 2.0 + 18.0;
        }
        if let Some(voice) = &voice {
            lines_h += 3.0 + voice.height();
        }
        Self { voice, lines_h }
    }

    /// The row: 4px above and below the lines, at least the 42px button.
    fn row_h(&self) -> f32 {
        f32::max(4.0 + self.lines_h + 4.0, 42.0)
    }
}

/// The text column's width in a list `list_w` wide: the list's 10px
/// padding, then avatar (6 + 32 + 10) on the left and the 22px voice
/// state column (10 + 22 + 6) on the right.
fn text_width(list_w: f32) -> f32 {
    list_w - 2.0 * 10.0 - (6.0 + 32.0 + 10.0) - (10.0 + 22.0 + 6.0)
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.panel);
    hairline_v(&painter, &theme, rect.left(), rect.top()..=rect.bottom());
    let own_seat = crate::screens::voice::session::own_seat(app, server);
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
                            own_seat
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
        members::panel_title(),
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
    let texts: Vec<Vec<RowText>> = groups
        .iter()
        .map(|(_, entries)| {
            entries
                .iter()
                .map(|e| RowText::new(&child, e, text_width(list.width()), c.muted))
                .collect()
        })
        .collect();
    let content_h: f32 = 14.0
        + texts
            .iter()
            .map(|t| 8.0 + 16.5 + 6.0 + t.iter().map(|x| x.row_h() + 2.0).sum::<f32>())
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
                    PresenceStatus::Online => gelabber_i18n::common::online(),
                    PresenceStatus::Idle => members::card_away(),
                    // Grouped with Online (`presenceGroup`); never a group.
                    PresenceStatus::Dnd => gelabber_i18n::common::online(),
                    PresenceStatus::Offline => gelabber_i18n::common::offline(),
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
                for (e, text) in entries.iter().zip(&texts[gi]) {
                    let h = text.row_h();
                    let row = Rect::from_min_size(pos2(left, y), Vec2::new(right - left, h));
                    y += h + 2.0;
                    let response = common::click(ui, row, Id::unique(("member-row", server, e.id)));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            members::row_profile_label(&e.name),
                        )
                    });
                    if response.clicked() {
                        super::overlays::open_member(ui.ctx(), server, e.id);
                    }
                    // A row out of view keeps its place in the Tab order and
                    // its focus, but is neither laid out nor painted.
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    let response = response.on_hover_text(members::row_profile_label(&e.name));
                    let under = if response.hovered() {
                        common::fill(ui.painter(), row, theme.radius.md, c.hover);
                        c.hover
                    } else {
                        c.panel
                    };
                    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
                        crate::widgets::focus_ring(ui.painter(), row, theme.radius.md, &theme);
                    }
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
                    let mut ty = row.center().y - text.lines_h / 2.0;
                    let name_color = if offline { c.muted } else { c.text };
                    let own = me == Some(e.id);
                    let small_style = TextStyle::new(11.0, 400.0, 1.5);
                    let du = own.then(|| {
                        Block::single(
                            ui,
                            &format!(" · {}", members::row_you()),
                            small_style,
                            c.muted,
                        )
                    });
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
                    if let Some(voice) = &text.voice {
                        ty += 3.0;
                        voice.paint(ui.painter(), pos2(text_left, ty));
                    }
                    // The 22px column centres its parts, gap 5: VoiceStateIcons
                    // (a 32px box, its two 14px slots at the end) above the Live icon.
                    let column_x = row.right() - 6.0 - 11.0;
                    let parts_h = match (e.voice.is_some(), e.live) {
                        (true, true) => 14.0 + 5.0 + 20.0,
                        (true, false) => 14.0,
                        (false, true) => 20.0,
                        (false, false) => 0.0,
                    };
                    let mut part_top = row.center().y - parts_h / 2.0;
                    if let Some((channel, muted, deafened)) = &e.voice {
                        let slot1 =
                            Rect::from_min_size(pos2(column_x - 14.0, part_top), Vec2::splat(14.0));
                        let slot2 = slot1.translate(Vec2::new(16.0, 0.0));
                        part_top += 14.0 + 5.0;
                        let (icon, tone) = if *muted || *deafened {
                            (Icon::MicOff, c.muted)
                        } else {
                            // `.voice-state-icons-speaker` (room.css).
                            (Icon::Speaker, c.online)
                        };
                        icons::paint(ui.painter(), icon, slot1.min, 14.0, tone);
                        if *deafened {
                            icons::paint(ui.painter(), Icon::HeadsetOff, slot2.min, 14.0, c.muted);
                        }
                        let hint = if *deafened {
                            members::row_deafened_in(channel)
                        } else if *muted {
                            members::row_muted_in(channel)
                        } else {
                            members::row_in_channel(channel)
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
                            pos2(column_x - 10.0, part_top),
                            20.0,
                            c.muted,
                        );
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{Entry, RowText, text_width};

    fn in_voice(live: bool) -> Entry {
        Entry {
            id: uuid::Uuid::nil(),
            name: "Rafi".into(),
            avatar: None,
            status: Some("Viben".into()),
            voice: Some(("Lernen 1 a bubensahne".into(), false, false)),
            live,
        }
    }

    /// GEL-63: in a narrow member list a long voice line wraps, and its
    /// row grows with it instead of running into the next one.
    #[test]
    fn a_wrapped_voice_line_makes_its_row_taller() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = Rc::clone(&seen);
        let mut harness = egui_kittest::Harness::new_ui(move |ui| {
            crate::fonts::install(ui.ctx());
            let mut out = out.borrow_mut();
            out.clear();
            for live in [false, true] {
                for list_w in [200.0, 360.0] {
                    let text = RowText::new(
                        ui,
                        &in_voice(live),
                        text_width(list_w),
                        ui.visuals().text_color(),
                    );
                    let rows = text.voice.as_ref().map_or(0, |v| v.rows());
                    let voice_h = text.voice.as_ref().map_or(0.0, |v| v.height());
                    out.push((live, list_w, rows, voice_h, text.row_h()));
                }
            }
        });
        harness.run_steps(2);
        let seen = seen.borrow();
        let narrow_live = seen.iter().find(|s| s.0 && s.1 == 200.0).expect("narrow");
        let wide_live = seen.iter().find(|s| s.0 && s.1 == 360.0).expect("wide");
        assert!(
            narrow_live.2 > 1,
            "the voice line wraps when narrow: {seen:?}"
        );
        assert!(
            narrow_live.4 > wide_live.4,
            "the narrow row grows with its wrapped line: {seen:?}"
        );
        for &(_, _, _, voice_h, row_h) in seen.iter() {
            // Name, status and voice line fit between the row's 4px edges.
            assert!(
                4.0 + 21.0 + 2.0 + 18.0 + 3.0 + voice_h + 4.0 <= row_h + 0.01,
                "a row holds its lines: {seen:?}"
            );
        }
    }
}
