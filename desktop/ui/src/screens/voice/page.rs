//! A voice channel's page (pages/ChannelPage.tsx voice branch,
//! voice/VoiceRoom.tsx, room.css): the header with the members toggle,
//! then the room. Before joining: who is there, their cards, free seats,
//! "Beitreten" with the microphone test and the quick settings. Inside: the
//! cards, or the stage while pictures run (Raster, Fokus, Kino with K and
//! Esc), the people without a camera under it, and the fullscreen view. In
//! a call of two or more without pictures the people fill the stage. In the
//! call the header carries the room's facts, the view switch and Kino.

use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use gelabber_i18n::voice as texts;
use uuid::Uuid;

use super::look::VIDEO_BG;
use super::session::{self, Person, Picture, Room};
use super::stage::{self, Kind, KinoKey, PicId, StageRect, Thumb, View};
use super::tile::{self, Place};
use super::video::{self, Fit};
use super::{chips, hint, quick, summary, watchers};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::layout;
use crate::screens::shell::common;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::tooltip::{Side, Tooltip};
use crate::widgets::{Button, IconButton, eased};

const HEADER: f32 = 52.0;
const TITLE: TextStyle = TextStyle::new(15.0, 650.0, 1.5).tracking(-0.1 / 15.0);
/// `FREE_SEATS_EMPTY`.
const FREE_SEATS_EMPTY: usize = 3;

/// Whether the room shows pictures, for the shell's member panel default.
pub fn staged(app: &AppCtx, server: Uuid, channel: Uuid) -> bool {
    session::room(app, server, channel).is_some_and(|r| r.staged())
}

/// Whether the page takes the window for Kino (the shell hides its parts).
pub fn kino(ctx: &egui::Context, app: &AppCtx, server: Uuid, channel: Uuid) -> bool {
    stage::kino(ctx)
        && ctx.content_rect().width() > 800.0
        && session::room(app, server, channel).is_some_and(|r| r.staged() && r.here)
}

/// `narrow` (640px): a phone's column of pictures, always Raster.
fn narrow(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() <= 640.0
}

/// The stage's view: Raster when `narrow`, else `stageViewFor`.
fn view_of(ctx: &egui::Context, room: &Room, picked: Option<View>) -> View {
    if narrow(ctx) {
        return View::Raster;
    }
    stage::view_for(
        picked,
        room.shares.iter().any(|t| Some(t.id.user) != room.me),
    )
}

/// Draws the page into `ui.max_rect()`.
pub fn show(ui: &mut Ui, app: &mut AppCtx, server: Uuid, channel: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let page = ui.max_rect();
    ui.painter().rect_filled(page, 0.0, c.bg);
    let Some(room) = session::room(app, server, channel) else {
        return;
    };
    draw(ui, app, &room, page);
}

/// The page of `room` in `page` (tests build the room by hand).
pub(super) fn draw(ui: &mut Ui, app: &mut AppCtx, room: &Room, page: Rect) {
    let (server, channel) = (room.server, room.channel);
    let joined = app.call.with_state(session::joined);
    let call = (server, channel);
    let choices = stage::choices(ui.ctx(), call, joined);
    let staged = room.staged();
    let roomy = ui.ctx().content_rect().width() > 800.0;
    let can_kino = staged && roomy;
    if choices.kino && !can_kino {
        stage::set_kino(ui.ctx(), false);
    }
    let kino = choices.kino && can_kino;
    // The large view has the keys: Esc leaves it, not Kino under it.
    if can_kino && !fullscreen_open(ui.ctx()) {
        kino_keys(ui, kino);
    }
    if kino {
        let body = page;
        stage_room(ui, app, room, body, choices, true);
    } else {
        header(ui, app, page, room, choices, can_kino);
        let body = Rect::from_min_max(pos2(page.left(), page.top() + HEADER), page.max);
        if staged {
            stage_room(ui, app, room, body, choices, false);
        } else if people_stage(room) {
            people_room(ui, app, room, body);
        } else {
            plain_room(ui, app, room, body);
        }
    }
    tile::participant_menu(ui, app, room);
    watchers::show(ui, app, room);
    fullscreen(ui, app, room);
}

/// No pictures, and company: the people fill the stage, in the grid the
/// pictures use. Alone, the card and the invite stay.
pub(super) fn people_stage(room: &Room) -> bool {
    room.here && !room.staged() && room.occupants.len() >= 2
}

/// The header's facts in the call: with pictures how many are there and
/// what runs, else who is there; nothing outside the call.
pub(super) fn facts(room: &Room) -> Option<String> {
    if !room.here {
        return None;
    }
    Some(if room.staged() {
        summary::stage_facts(
            room.occupants.len(),
            room.cameras.len(),
            room.occupants.iter().filter(|p| p.live).count(),
        )
    } else {
        summary::room_summary(&room.others, true, &[])
    })
}

/// The header (`.lr-channel-header`): the speaker and the name, in the
/// call the room's facts after it (else the Live pill), the view switch
/// while pictures run and Kino, the members toggle at the end; gap 10.
fn header(
    ui: &mut Ui,
    app: &mut AppCtx,
    page: Rect,
    room: &Room,
    choices: stage::Choices,
    can_kino: bool,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let (name, server, channel) = (&room.channel_name, room.server, room.channel);
    let rect = Rect::from_min_size(page.min, vec2(page.width(), HEADER));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, c.bg);
    crate::widgets::rule_h(&painter, c.border, rect.x_range(), rect.bottom() - 1.0);
    let mid = rect.top() + (HEADER - 1.0) / 2.0;
    let x = rect.left() + 20.0;
    icons::paint(
        ui.painter(),
        Icon::Speaker,
        pos2(x, mid - 9.0),
        18.0,
        c.muted,
    );
    let toggle = Rect::from_min_size(
        pos2(rect.right() - 12.0 - 32.0, mid - 16.0),
        Vec2::splat(32.0),
    );
    // Kino, then the view switch, right to left before the members.
    let ctx = ui.ctx().clone();
    let mut end = toggle.left();
    if can_kino {
        let (r, pressed) = chips::kino_button(ui, end - 10.0, mid);
        if pressed {
            stage::set_kino(&ctx, true);
        }
        end = r.left();
    }
    // A phone keeps its column of pictures (`narrow`, 640px).
    if room.staged() && !narrow(&ctx) {
        let view = view_of(&ctx, room, choices.view);
        let (frame, chosen) = chips::view_switch(ui, end - 10.0, mid, view);
        if let Some(v) = chosen {
            stage::choose_view(&ctx, (server, channel), v);
        }
        end = frame.left();
    }
    // A room limited to roles: the lock after its name (14, gap 10).
    let limited = crate::screens::shell::access::is_limited(app, server, channel);
    let lock_room = if limited { 14.0 + 10.0 } else { 0.0 };
    let max = end - 10.0 - x - 28.0 - lock_room;
    let mut job = text::job(name, TITLE, c.text, Some(max.max(40.0)));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let title = Block::new(ui, job, TITLE);
    title.paint(ui.painter(), pos2(x + 28.0, mid - TITLE.line_px() / 2.0));
    let mut after = x + 28.0 + title.width() + 10.0;
    if limited {
        let id = Id::unique(("gelabber-voice-title", channel));
        let lock = crate::screens::shell::access::lock(ui, ui.painter(), id, after, mid);
        after = lock.right() + 10.0;
    }
    if let Some(facts) = facts(room) {
        // `.voice-header-facts`: 13/500 muted, one line with an ellipsis.
        let room_for = end - 10.0 - after;
        if room_for > 0.0 {
            let block = common::line(ui, &facts, text::UI, c.muted, room_for);
            common::paint_mid(ui.painter(), &block, after, mid);
        }
    } else {
        // Who is live on the server, right after the name (`LiveHint pill`).
        hint::pill(ui, app, server, after, mid, end - 20.0);
    }

    let on = layout::members_visible(ui.ctx());
    let label = if on {
        texts::members_hide()
    } else {
        texts::members_show()
    };
    let id = Id::unique(("gelabber-voice-members", channel));
    let response = ui.interact(toggle, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let lit = on || response.hovered();
    let fill = eased(
        ui.ctx(),
        id.with("fill"),
        lit,
        Color32::TRANSPARENT,
        c.hover,
    );
    ui.painter().rect_filled(toggle, corner(7.0), fill);
    icons::paint_centered(
        ui.painter(),
        Icon::Users,
        toggle,
        18.0,
        if lit { c.text } else { c.muted },
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    Tooltip::new(label).on_hover(&response, Side::Below);
    if response.clicked() {
        layout::set_members_visible(ui.ctx(), !on);
    }
}

/// The room without pictures: the summary, the cards and free seats, the
/// way in.
fn plain_room(ui: &mut Ui, app: &mut AppCtx, room: &Room, body: Rect) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt(("voice-room-scroll", room.channel))
        .max_height(body.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let top = ui.cursor().min.y;
            let bottom = room_content(ui, app, room, body.left(), top, body.width());
            ui.allocate_exact_size(vec2(body.width(), bottom - top), Sense::hover());
        });
}

/// The people stage (`.voice-room[data-stage="people"]`): inset 20 24 24,
/// at least 360 high, every occupant in a tile of the pictures' grid.
fn people_room(ui: &mut Ui, app: &mut AppCtx, room: &Room, body: Rect) {
    let inner = Rect::from_min_max(
        pos2(body.left() + 24.0, body.top() + 20.0),
        pos2(
            body.right() - 24.0,
            (body.bottom() - 24.0).max(body.top() + 360.0 - 24.0),
        ),
    );
    let rects = stage::raster_rects(
        room.occupants.len(),
        inner.width().floor() as i32,
        inner.height().floor() as i32,
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(body));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    for (p, rect) in room.occupants.iter().zip(rects) {
        tile::person_tile(&mut child, app, rect.on(inner.min), room.server, p);
    }
}

/// The not-staged room from `top`; returns its bottom (padding included).
pub(super) fn room_content(
    ui: &mut Ui,
    app: &mut AppCtx,
    room: &Room,
    left: f32,
    top: f32,
    width: f32,
) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    // `html[data-compact-rooms]`: 16px above and below, cards as high as
    // their content.
    let compact = crate::screens::settings::prefs::get(ui.ctx()).compact_rooms;
    let left = left + 28.0;
    let w = (width - 56.0).max(0.0);
    let mut y = top + if compact { 16.0 } else { 20.0 };
    // In the call the header carries the facts, and a card alone starts
    // where the people stage does.
    if !room.here {
        let line = summary::room_summary(&room.others, room.here, &room.others_live);
        let block = Block::wrapped(ui, &line, text::BODY, c.muted, w);
        block.paint(ui.painter(), pos2(left, y + 2.0));
        y += 2.0 + block.height();
    }

    let free = if room.here || !room.allowed {
        0
    } else if room.occupants.is_empty() {
        FREE_SEATS_EMPTY
    } else {
        1
    };
    let count = room.occupants.len() + free;
    if count > 0 {
        if !room.here {
            y += 16.0;
        }
        let grid_w = w.min(1100.0);
        let cols = (((grid_w + 12.0) / 212.0).floor() as usize).max(1);
        let tile_w = (grid_w - 12.0 * (cols as f32 - 1.0)) / cols as f32;
        let tile_h = if compact {
            compact_card_height()
        } else {
            tile_w * 10.0 / 16.0
        };
        for i in 0..count {
            let (row, col) = (i / cols, i % cols);
            let rect = Rect::from_min_size(
                pos2(
                    left + col as f32 * (tile_w + 12.0),
                    y + row as f32 * (tile_h + 12.0),
                ),
                vec2(tile_w, tile_h),
            );
            match room.occupants.get(i) {
                Some(p) => tile::card(ui, app, rect, room.server, p),
                None => tile::seat(ui, rect),
            }
        }
        let rows = count.div_ceil(cols);
        y += rows as f32 * tile_h + (rows as f32 - 1.0) * 12.0;
    }
    if !room.here && room.allowed && room.elsewhere {
        y += 16.0;
        let note = Block::wrapped(
            ui,
            &match room.elsewhere_name.as_deref() {
                Some(name) => texts::room_switch_note(name),
                None => texts::room_switch_note_unnamed().to_owned(),
            },
            text::META,
            c.muted,
            w,
        );
        note.paint(ui.painter(), pos2(left, y));
        y += note.height();
    }
    if !room.here {
        y += 20.0;
        y = actions(ui, app, room, left, y);
    }
    if room.others.is_empty() {
        y += 24.0;
        y = invite_prompt(ui, app, room, left, y, w);
    }
    y + if compact { 16.0 } else { 28.0 }
}

/// `.voice-room-invite`: nobody else in the room, so the space under it
/// asks for company and opens the invite dialog. The button wraps under
/// the question when the row is too narrow (flex-wrap, row gap 8).
/// Returns the row's bottom.
fn invite_prompt(ui: &mut Ui, app: &AppCtx, room: &Room, left: f32, top: f32, width: f32) -> f32 {
    let theme = Theme::of(ui.ctx());
    let row_h = 32.0;
    let ask = Block::single(ui, texts::room_ask_company(), text::BODY, theme.c.muted);
    // The small button: its label, the 16px icon and gap 8, padding 12.
    let label = Block::single(ui, texts::room_invite(), text::UI, theme.c.text);
    let button_w = label.width() + 24.0 + 24.0;
    let wraps = ask.width() + 16.0 + button_w > width;
    common::paint_mid(ui.painter(), &ask, left, top + row_h / 2.0);
    let (bx, by) = if wraps {
        (left, top + row_h + 8.0)
    } else {
        (left + ask.width() + 16.0, top)
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(
        pos2(bx, by),
        vec2(button_w.max(200.0), row_h),
    )));
    let button = Button::new(texts::room_invite()).small().icon(Icon::Link);
    if child.add(button).clicked() {
        let name = app.client.with_state(|s| {
            s.server(room.server)
                .map(|v| v.name.clone())
                .unwrap_or_default()
        });
        crate::screens::shell::dialogs::open_invite(ui.ctx(), room.server, &name);
    }
    by + row_h
}

/// A card's height without its 16:10 (`aspect-ratio: auto`): the 64px
/// avatar, the name and the status line inside the card's padding.
fn compact_card_height() -> f32 {
    1.0 + 14.0 + 64.0 + 2.0 + 6.0 + text::UI.line_px() + text::META.line_px() + 12.0 + 1.0
}

/// "Beitreten" (or the missing permission), the microphone test and the
/// quick settings; returns the row's bottom.
pub fn actions(ui: &mut Ui, app: &mut AppCtx, room: &Room, left: f32, top: f32) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let row = Rect::from_min_size(pos2(left, top), vec2(400.0, 44.0));
    let mut x = left;
    if room.allowed {
        let label = if room.elsewhere {
            texts::room_switch_here()
        } else {
            texts::room_join()
        };
        let join = chips::join_button(
            ui,
            &theme,
            row,
            label,
            Id::unique(("voice-join", room.channel)),
        );
        if join.clicked() {
            app.call.join(room.server, room.channel);
        }
        x = join.rect.right() + 8.0;
    } else {
        let p = Block::single(
            ui,
            texts::room_no_permission(),
            text::UI.weight(400.0),
            c.muted,
        );
        common::paint_mid(ui.painter(), &p, x, row.center().y);
        x += p.width() + 8.0;
    }
    let mic = Rect::from_min_size(pos2(x, row.center().y - 18.0), Vec2::splat(36.0));
    if ui
        .put(
            mic,
            IconButton::new(Icon::Mic, texts::room_test_microphone())
                .secondary()
                .icon_size(18.0),
        )
        .clicked()
    {
        super::dialogs::open_mic_test(ui.ctx());
    }
    let gear = mic.translate(Vec2::new(36.0 + 8.0, 0.0));
    let response = ui.put(
        gear,
        IconButton::new(
            Icon::Gear,
            gelabber_i18n::voice_settings::quick_call_settings(),
        )
        .secondary()
        .icon_size(17.0),
    );
    quick::button(ui, app, response, gear);
    top + 44.0
}

/// The room with pictures: the stage, the people without a camera under
/// it; in Kino the stage alone on the video background.
fn stage_room(
    ui: &mut Ui,
    app: &mut AppCtx,
    room: &Room,
    body: Rect,
    choices: stage::Choices,
    kino: bool,
) {
    let ctx = ui.ctx().clone();
    let call = (room.server, room.channel);
    let now = ui.input(|i| i.time);
    let pinned = choices.pinned;
    let id_of = |p: &Picture| p.id;
    let view = view_of(&ctx, room, choices.view);
    let focus = (view == View::Fokus || kino)
        .then(|| stage::focus_of(&room.shares, &room.cameras, pinned, room.me, id_of))
        .flatten();
    let ordered = stage::raster_order(&room.shares, &room.cameras, pinned, id_of);
    let strip: Vec<Picture> = ordered
        .iter()
        .filter(|t| Some(t.id) != focus.as_ref().map(|f| f.id))
        .cloned()
        .collect();
    let without_camera: Vec<Person> = room
        .occupants
        .iter()
        .filter(|p| !p.camera)
        .cloned()
        .collect();
    let strip_people: Vec<Person> = if focus.is_some() {
        without_camera.clone()
    } else {
        Vec::new()
    };
    // The row says who watches the Live in front; a camera in front, nobody.
    let watching_focus: Vec<Uuid> = focus
        .as_ref()
        .filter(|f| f.id.kind == Kind::Live)
        .and_then(|f| room.watchers.get(&f.id.user))
        .map(|(w, _)| w.clone())
        .unwrap_or_default();

    // The room box.
    let (stage_rect, people_top) = if kino {
        ui.painter().rect_filled(body, 0.0, VIDEO_BG);
        (body, None)
    } else {
        let inner = Rect::from_min_max(
            pos2(body.left() + 24.0, body.top() + 16.0),
            pos2(
                body.right() - 24.0,
                (body.bottom() - 16.0).max(body.top() + 360.0 - 16.0),
            ),
        );
        let stage_top = inner.top();
        let mut stage_bottom = inner.bottom();
        let mut people_top = None;
        if focus.is_none() && !without_camera.is_empty() {
            let rows = people_rows(ui, &without_camera, inner.width());
            let h = rows.len() as f32 * 44.0 + (rows.len() as f32 - 1.0) * 8.0;
            stage_bottom -= h + 12.0;
            people_top = Some((inner.bottom() - h, rows));
        }
        let rect = Rect::from_min_max(
            pos2(inner.left(), stage_top),
            pos2(inner.right(), stage_bottom.max(stage_top)),
        );
        (rect, people_top)
    };
    let (w, h) = (
        stage_rect.width().floor() as i32,
        stage_rect.height().floor() as i32,
    );
    let origin = stage_rect.min;
    let layout: Option<(Option<StageRect>, Vec<StageRect>)> = if kino {
        let thumbs: Vec<Thumb> = strip
            .iter()
            .map(|_| Thumb::Picture)
            .chain(strip_people.iter().map(|_| Thumb::Person))
            .collect();
        stage::kino_rects(&thumbs, w, h).map(|(f, s)| (Some(f), s))
    } else if focus.is_some() {
        stage::focus_rects(strip.len() + strip_people.len(), w, h).map(|(f, s)| (Some(f), s))
    } else {
        Some((None, stage::raster_rects(ordered.len(), w, h)))
    };

    // Kino: awake for a while after the pointer moves or a key.
    let awake = if kino {
        let moved = ui.input(|i| {
            i.pointer.delta() != Vec2::ZERO
                || i.pointer.any_pressed()
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { .. }))
        });
        if moved && ui.rect_contains_pointer(stage_rect) {
            stage::wake(&ctx);
        }
        let until = stage::choices(&ctx, call, Some(call))
            .awake_until
            .max(choices.awake_until);
        let awake = now < until;
        if awake {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - now).max(0.05)));
        } else if ui.rect_contains_pointer(stage_rect) {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        awake
    } else {
        false
    };

    if let Some((focus_rect, strip_rects)) = layout {
        // One tile per picture in a fixed order; a place is only a rect.
        for pic in room.shares.iter().chain(room.cameras.iter()) {
            let is_focus = focus.as_ref().is_some_and(|f| f.id == pic.id);
            let rect = if is_focus {
                focus_rect
            } else {
                let list = if focus.is_some() { &strip } else { &ordered };
                list.iter()
                    .position(|t| t.id == pic.id)
                    .and_then(|i| strip_rects.get(i).copied())
            };
            let Some(rect) = rect else { continue };
            let rect = rect.on(origin);
            let place = match (kino, is_focus, focus.is_some()) {
                (true, true, _) => Place::KinoFocus,
                (true, false, _) => Place::KinoStrip,
                (false, true, _) => Place::Focus,
                (false, false, true) => Place::Strip,
                (false, false, false) => Place::Grid,
            };
            let opacity = if place == Place::KinoStrip && !awake && !ui.rect_contains_pointer(rect)
            {
                0.0
            } else {
                1.0
            };
            let person = room.person(pic.id.user);
            let out = tile::picture(
                ui,
                app,
                rect,
                pic,
                person,
                place,
                pinned == Some(pic.id),
                awake,
                opacity,
                (pic.id.kind == Kind::Live).then(|| watchers::count(room, pic.id.user)),
            );
            if out.pin {
                stage::pin(
                    &ctx,
                    call,
                    if pinned == Some(pic.id) {
                        None
                    } else {
                        Some(pic.id)
                    },
                );
            } else if out.select && focus.is_some() && !is_focus {
                stage::pin(&ctx, call, Some(pic.id));
            }
            if out.fullscreen {
                open_fullscreen(&ctx, pic.id);
            }
            if let Some(on) = out.watch {
                app.call.watch(pic.id.user, on);
            }
        }
        for (i, p) in strip_people.iter().enumerate() {
            let Some(rect) = strip_rects.get(strip.len() + i) else {
                continue;
            };
            let rect = rect.on(origin);
            let opacity = if kino && !awake && !ui.rect_contains_pointer(rect) {
                0.0
            } else {
                1.0
            };
            tile::strip_person(
                ui,
                app,
                rect,
                room.server,
                p,
                kino,
                opacity,
                watching_focus.contains(&p.id),
            );
        }
    }
    if let Some((top, rows)) = people_top {
        let left = stage_rect.left();
        for (r, row) in rows.iter().enumerate() {
            let mut x = left;
            for (p, width) in row {
                let rect = Rect::from_min_size(pos2(x, top + r as f32 * 52.0), vec2(*width, 44.0));
                tile::chip(ui, app, rect, room.server, p);
                x += width + 8.0;
            }
        }
    }
    if kino {
        let label = focus.as_ref().map(|f| {
            (
                f.id.kind == Kind::Live,
                room.name_of(f.id.user),
                room.channel_name.clone(),
            )
        });
        if chips::kino_bar(ui, body, label) {
            stage::set_kino(&ctx, false);
        }
    }
}

/// The chips of the people without a camera, wrapped into rows.
fn people_rows(ui: &Ui, people: &[Person], width: f32) -> Vec<Vec<(Person, f32)>> {
    let mut rows: Vec<Vec<(Person, f32)>> = vec![Vec::new()];
    let mut x = 0.0;
    for p in people {
        let hover = false;
        let w = tile::chip_width(ui, p, hover);
        let row = rows.last_mut().expect("a row");
        if !row.is_empty() && x + w > width {
            rows.push(Vec::new());
            x = 0.0;
        }
        rows.last_mut().expect("a row").push((p.clone(), w));
        x += w + 8.0;
    }
    rows
}

/// K and Esc for Kino, unless a field has the keys or a dialog is open.
pub(super) fn kino_keys(ui: &Ui, kino: bool) {
    let ctx = ui.ctx();
    // A dialog over the page owns the keys: also the quick switcher, the
    // shortcut sheet and the profile (`modalOpen` on the web).
    if ctx.egui_wants_keyboard_input()
        || crate::screens::shell::dialog_open(ctx)
        || super::dialogs::open(ctx)
        || ctx.memory(|m| m.top_modal_layer().is_some())
    {
        return;
    }
    let ptt = {
        let p = crate::screens::settings::prefs::get(ctx);
        (p.voice_input == "ptt")
            .then(|| crate::screens::settings::media::window_key(&p.ptt_key, &p.ptt_key_label))
            .flatten()
    };
    // An open menu or popover (the call bar's too) closes on Esc first.
    let popover = crate::widgets::popover::any_open(ctx);
    let mut action = None;
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers,
            } = e
            else {
                return true;
            };
            let ptt = ptt_of(ptt, *key, *physical_key);
            match stage::kino_key(*key, *modifiers, *repeat, kino, ptt) {
                Some(KinoKey::Leave) if popover => true,
                Some(a) => {
                    action = Some(a);
                    false
                }
                None => true,
            }
        });
    });
    match action {
        Some(KinoKey::Leave) => stage::set_kino(ctx, false),
        Some(KinoKey::Toggle) => stage::set_kino(ctx, !kino),
        None => {}
    }
}

/// The push-to-talk key as Kino compares it with this press's `key`: by
/// what typed its label, or this press when it is at its place.
fn ptt_of(
    ptt: Option<crate::screens::settings::media::WindowKey>,
    key: egui::Key,
    physical: Option<egui::Key>,
) -> Option<egui::Key> {
    use crate::screens::settings::media::WindowKey;
    match ptt? {
        WindowKey::Logical(k) => Some(k),
        WindowKey::Place(k) => (physical == Some(k)).then_some(key),
    }
}

fn fullscreen_id() -> Id {
    Id::unique("gelabber-voice-fullscreen")
}

fn fullscreen_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<PicId>>(fullscreen_id()))
        .flatten()
        .is_some()
}

/// The picture the large view shows, while it is open.
pub(super) fn large_view(ctx: &egui::Context) -> Option<PicId> {
    ctx.data(|d| d.get_temp::<Option<PicId>>(fullscreen_id()))
        .flatten()
}

pub(super) fn open_fullscreen(ctx: &egui::Context, id: PicId) {
    ctx.data_mut(|d| d.insert_temp(fullscreen_id(), Some(id)));
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
}

fn close_fullscreen(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(fullscreen_id(), None::<PicId>));
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
}

/// The large view: the picture contained on black over the whole window,
/// the WatchStrip at the bottom (mute, deafen, "Nicht mehr zuschauen",
/// "Vollbild beenden"); Esc, M and Leertaste as in the web.
fn fullscreen(ui: &mut Ui, app: &mut AppCtx, room: &Room) {
    let ctx = ui.ctx().clone();
    let Some(id) = ctx
        .data(|d| d.get_temp::<Option<PicId>>(fullscreen_id()))
        .flatten()
    else {
        return;
    };
    let Some(pic) = room
        .shares
        .iter()
        .chain(room.cameras.iter())
        .find(|p| p.id == id)
        .cloned()
    else {
        close_fullscreen(&ctx);
        return;
    };
    let screen = ctx.content_rect();
    let state = app.call.state();
    let mut leave = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    // As `VoiceTile` onKeyDown: not with Strg/Alt/⌘ (Strg+Umschalt+M is the
    // call bar's), and Space stays a focused button's.
    let space_free = ctx.memory(|m| m.focused().is_none());
    let toggle = ctx.input(|i| {
        let m = i.modifiers;
        !(m.ctrl || m.command || m.mac_cmd || m.alt)
            && (i.key_pressed(egui::Key::M) || (space_free && i.key_pressed(egui::Key::Space)))
    });
    if room.here && toggle {
        app.call.set_muted(!(state.muted || state.deafened));
    }
    egui::Area::new(Id::unique("gelabber-voice-fullscreen-area"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(&ctx, |ui| {
            let painter = ui.painter().clone();
            painter.rect_filled(screen, 0.0, Color32::BLACK);
            ui.allocate_rect(screen, Sense::click());
            if let Some(frame) = pic.key.and_then(|k| app.call.frame(k)) {
                if let Some(key) = pic.key {
                    video::request_fit(ui, app.call, key, screen, Some(&frame), Fit::Contain, 60);
                }
                video::paint(
                    ui,
                    Id::unique(("fullscreen", pic.id.user, pic.id.kind == Kind::Live)),
                    screen,
                    Arc::clone(&frame),
                    Fit::Contain,
                    pic.mirror,
                    0.0,
                );
            }
            // No Live tag: on the stage a Live's caption carries the badge,
            // and the large view hides the caption (`immersive`).
            leave |= watch_strip(ui, app, room, &pic, screen, &state);
        });
    if leave {
        close_fullscreen(&ctx);
    }
}

/// `WatchStrip`; returns true for "Vollbild beenden".
fn watch_strip(
    ui: &mut Ui,
    app: &mut AppCtx,
    room: &Room,
    pic: &Picture,
    screen: Rect,
    state: &gelabber_call::CallState,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let white = Color32::WHITE;
    let label = Block::single(ui, &pic.label, text::UI.weight(600.0), white);
    let button_label = |ui: &Ui, s: &str| Block::single(ui, s, text::UI, white);
    let end = button_label(ui, texts::tile_exit_fullscreen());
    let stop = (pic.id.kind == Kind::Live && pic.watching)
        .then(|| button_label(ui, texts::tile_stop_watching()));
    let toggles = room.here;
    // `StreamVolume`: someone else's Live has the stream sound beside it.
    let stream_sound = pic.id.kind == Kind::Live && !pic.own;
    // `.voice-control-compact`: 9px padding and a 1px border around the
    // 17px icon make 37, past its 36px minimum.
    const COMPACT: f32 = 1.0 + 9.0 + 17.0 + 9.0 + 1.0;
    // The strip's 1px border on both sides, then its padding and children.
    let mut w = 2.0 + 14.0 + label.width() + 4.0 + 8.0;
    if toggles {
        w += 1.0 + 8.0 + COMPACT + 4.0 + COMPACT + 8.0;
    }
    if stream_sound {
        w += 1.0 + 8.0 + COMPACT + 4.0 + 112.0 + 8.0;
    }
    if let Some(s) = &stop {
        w += 12.0 + 16.0 + 6.0 + s.width() + 12.0 + 8.0;
    }
    w += 12.0 + 16.0 + 6.0 + end.width() + 12.0 + 8.0;
    let h = 36.0 + 12.0 + 2.0;
    let strip = Rect::from_min_size(
        pos2(screen.center().x - w / 2.0, screen.bottom() - 16.0 - h),
        vec2(w, h),
    );
    let painter = ui.painter().clone();
    painter.add(
        egui::epaint::Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(128),
        }
        .as_shape(strip, corner(10.0)),
    );
    painter.rect(
        strip,
        corner(10.0),
        Color32::from_rgba_unmultiplied(18, 20, 21, 235),
        Stroke::new(1.0, Color32::from_white_alpha(36)),
        egui::StrokeKind::Inside,
    );
    let cy = strip.center().y;
    let mut x = strip.left() + 1.0 + 14.0;
    common::paint_mid(&painter, &label, x, cy);
    x += label.width() + 4.0 + 8.0;
    if toggles {
        crate::widgets::rule_v(
            &painter,
            Color32::from_white_alpha(36),
            x,
            cy - 18.0..=cy + 18.0,
        );
        x += 9.0;
        let off_mic = state.muted || state.deafened;
        for (icon, label, off, which) in [
            (
                if off_mic { Icon::MicOff } else { Icon::Mic },
                texts::controls_mute(),
                off_mic,
                0,
            ),
            (
                if state.deafened {
                    Icon::HeadsetOff
                } else {
                    Icon::Headset
                },
                texts::controls_deafen(),
                state.deafened,
                1,
            ),
        ] {
            let r = Rect::from_min_size(pos2(x, cy - 18.0), vec2(COMPACT, 36.0));
            x += COMPACT + 4.0;
            let resp = common::click(ui, r, Id::unique(("watch-strip", which)));
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            // The call bar's tooltip, icons only (`VoiceAudioToggles`).
            let (title, detail) = super::call_bar::call_tip(
                ui.ctx(),
                if which == 0 {
                    super::call_bar::CallKey::Mute
                } else {
                    super::call_bar::CallKey::Deafen
                },
                off,
            );
            Tooltip::new(title)
                .detail(Some(&detail))
                .on_hover_or_focus(&resp, Side::Above);
            let fill = if off {
                c.danger_bg.gamma_multiply(0.45)
            } else if resp.hovered() {
                Color32::from_white_alpha(36)
            } else {
                Color32::TRANSPARENT
            };
            painter.rect_filled(r, corner(theme.radius.sm), fill);
            icons::paint_centered(&painter, icon, r, 17.0, white);
            if resp.clicked() {
                if which == 0 {
                    app.call.set_muted(!off_mic);
                } else {
                    app.call.set_deafened(!state.deafened);
                }
            }
        }
        x += 4.0;
    }
    if stream_sound {
        crate::widgets::rule_v(
            &painter,
            Color32::from_white_alpha(36),
            x,
            cy - 18.0..=cy + 18.0,
        );
        x += 9.0;
        let mut p = crate::screens::settings::prefs::get(ui.ctx());
        let muted = p.source_audio_muted;
        let r = Rect::from_min_size(pos2(x, cy - 18.0), vec2(COMPACT, 36.0));
        let resp = common::click(ui, r, Id::unique("watch-strip-stream-mute"));
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::Button,
                true,
                muted,
                texts::tile_mute_live_sound(),
            )
        });
        let resp = resp.on_hover_text(if muted {
            texts::tile_unmute_live_sound()
        } else {
            texts::tile_mute_live_sound()
        });
        let fill = if muted {
            c.danger_bg.gamma_multiply(0.45)
        } else if resp.hovered() {
            Color32::from_white_alpha(36)
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(r, corner(theme.radius.md), fill);
        icons::paint_centered(
            &painter,
            if muted {
                Icon::SpeakerOff
            } else {
                Icon::Speaker
            },
            r,
            17.0,
            white,
        );
        if resp.clicked() {
            p.source_audio_muted = !muted;
        }
        x += COMPACT + 4.0;
        let slider = Rect::from_min_size(pos2(x, cy - 10.0), vec2(112.0, 20.0));
        let mut pos = super::volume::position(p.source_audio_volume);
        let changed = crate::screens::settings::controls::place(
            ui,
            UiBuilder::new().max_rect(slider),
            |ui| {
                crate::screens::settings::controls::slider_named(
                    ui,
                    "watch-strip-stream-volume",
                    texts::tile_live_sound_volume(),
                    &mut pos,
                    (0.0, 200.0, 1.0),
                    muted || state.deafened,
                )
            },
        );
        if changed {
            p.source_audio_volume = super::volume::volume(pos);
        }
        if p != crate::screens::settings::prefs::get(ui.ctx()) {
            crate::screens::settings::prefs::update(ui.ctx(), |q| *q = p);
        }
        x += 112.0 + 8.0;
    }
    let pill_button = |ui: &mut Ui, x: &mut f32, icon: Icon, block: &Block, id: &str| -> bool {
        let bw = 12.0 + 16.0 + 6.0 + block.width() + 12.0;
        let r = Rect::from_min_size(pos2(*x, cy - 18.0), vec2(bw, 36.0));
        *x += bw + 8.0;
        let resp = common::click(ui, r, Id::unique(("watch-strip-button", id)));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, id));
        let a = if resp.hovered() { 0.2 } else { 0.12 };
        painter.rect_filled(
            r,
            corner(theme.radius.sm),
            Color32::from_white_alpha((a * 255.0) as u8),
        );
        icons::paint(&painter, icon, pos2(r.left() + 12.0, cy - 8.0), 16.0, white);
        common::paint_mid(&painter, block, r.left() + 12.0 + 16.0 + 6.0, cy);
        resp.clicked()
    };
    if let Some(s) = &stop
        && pill_button(ui, &mut x, Icon::EyeOff, s, texts::tile_stop_watching())
    {
        app.call.watch(pic.id.user, false);
    }
    pill_button(
        ui,
        &mut x,
        Icon::Collapse,
        &end,
        texts::tile_exit_fullscreen(),
    )
}

#[cfg(test)]
mod tests {
    use super::ptt_of;
    use crate::screens::settings::media::WindowKey;
    use egui::Key;

    /// Kino leaves the push-to-talk key alone: QWERTZ's Z (stored at KeyY)
    /// by what it types, a key egui names no character for by its place.
    #[test]
    fn kino_knows_the_push_to_talk_key_as_it_was_pressed() {
        let qwertz = Some(WindowKey::Logical(Key::Z));
        assert_eq!(ptt_of(qwertz, Key::Z, Some(Key::Y)), Some(Key::Z));
        let place = Some(WindowKey::Place(Key::K));
        assert_eq!(ptt_of(place, Key::K, Some(Key::K)), Some(Key::K));
        assert_eq!(ptt_of(place, Key::K, Some(Key::L)), None);
        assert_eq!(ptt_of(None, Key::K, Some(Key::K)), None);
    }
}
