//! One person or picture in a room (voice/Participant.tsx, VoiceTile.tsx,
//! room.css): the card with the avatar, the free seat, the chip in the
//! row of people without a camera, the person in Fokus's and Kino's row,
//! and a camera or Live on the stage with its name pill, Live badge and
//! hover actions.

use egui::{Color32, Id, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_i18n::voice as texts;
use uuid::Uuid;

use super::look::{self, MARK_ON_PICTURE, VIDEO_BG, pill};
use super::session::{Person, Picture, Room};
use super::stage::{self, Kind};
use super::video::{self, Fit};
use super::{volume, window};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::shell::common::{self, line};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Popover, snap};

/// The participant menu of `user` is open, anchored at this rect.
fn menu_id() -> Id {
    Id::unique("gelabber-voice-participant-menu")
}

pub fn open_menu(ctx: &egui::Context, user: Uuid, anchor: Rect) {
    ctx.data_mut(|d| d.insert_temp(menu_id(), Some((user, anchor))));
}

fn menu_open(ctx: &egui::Context, user: Uuid) -> bool {
    ctx.data(|d| d.get_temp::<Option<(Uuid, Rect)>>(menu_id()))
        .flatten()
        .is_some_and(|(u, _)| u == user)
}

/// The participant menu (`ParticipantMenu`): the name, the volume and
/// "Profil ansehen"; width 288, padding 12, under the "…" at its end.
pub fn participant_menu(ui: &mut Ui, app: &mut AppCtx, room: &Room) {
    let Some((user, anchor)) = ui
        .ctx()
        .data(|d| d.get_temp::<Option<(Uuid, Rect)>>(menu_id()))
        .flatten()
    else {
        return;
    };
    let Some(person) = room.person(user).cloned() else {
        ui.ctx()
            .data_mut(|d| d.insert_temp(menu_id(), None::<(Uuid, Rect)>));
        return;
    };
    let call = app.call.clone();
    let shown = Popover::new(("voice-participant", user), anchor)
        .align_end()
        .min_width(288.0)
        .show(ui.ctx(), |ui| {
            let theme = Theme::of(ui.ctx());
            ui.set_width(288.0 - 18.0);
            ui.add_space(3.0);
            let inner = 288.0 - 2.0 - 24.0;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
                ui.cursor().min + vec2(3.0, 0.0),
                vec2(inner, 400.0),
            )));
            let name = Block::wrapped(
                &child,
                &person.name,
                text::UI.weight(600.0),
                theme.c.text,
                inner,
            );
            let (r, _) = child.allocate_exact_size(vec2(inner, name.height()), Sense::hover());
            name.paint(child.painter(), r.min);
            child.add_space(10.0);
            volume::control(&mut child, &call, user);
            child.add_space(8.0);
            // A row of the menu: out to its edges, the label in line.
            let left = child.cursor().min.x - 12.0;
            let profile = super::chips::ghost_row(
                &mut child,
                texts::participant_view_profile(),
                left,
                inner + 24.0,
            );
            let used = child.min_rect();
            ui.allocate_exact_size(vec2(288.0 - 18.0, used.height() + 3.0), Sense::hover());
            if profile.clicked() {
                open_profile(ui.ctx(), room.server, &person);
            }
            profile.clicked()
        });
    match shown {
        Some(true) | None => {
            ui.ctx()
                .data_mut(|d| d.insert_temp(menu_id(), None::<(Uuid, Rect)>));
        }
        Some(false) => {}
    }
}

/// `MemberProfileDialog` of `p`, when there is a member to show.
fn open_profile(ctx: &egui::Context, server: Uuid, p: &Person) {
    if p.member {
        crate::screens::shell::overlays::open_member(ctx, server, p.id);
    }
}

/// The avatar as the "Profil von …" button (`.voice-room-avatar`).
fn avatar_button(ui: &mut Ui, rect: Rect, id: Id, server: Uuid, p: &Person) {
    let hit = ui.interact(
        rect,
        id,
        if p.member {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if p.member && hit.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    hit.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            p.member,
            texts::participant_profile_label(&p.name),
        )
    });
    if hit.clicked() {
        open_profile(ui.ctx(), server, p);
    }
}

/// The "…" button (`IconButton xs`): 28×28, radius 5.
#[allow(clippy::too_many_arguments)]
fn more_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    name: &str,
    visible: f32,
    on_picture: bool,
) -> Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = texts::participant_options_label(name);
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let response = response.on_hover_text(&label);
    if visible <= 0.0 {
        return response;
    }
    let hovered = response.hovered();
    let (fill, fg) = if on_picture {
        (pill(if hovered { 0.82 } else { 0.62 }), Color32::WHITE)
    } else if hovered {
        (c.hover, c.text)
    } else {
        (Color32::TRANSPARENT, c.muted)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, corner(theme.radius.sm), fill.gamma_multiply(visible));
    icons::paint_centered(painter, Icon::More, rect, 16.0, fg.gamma_multiply(visible));
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// The marks in a card's corner: volume chip, local mute, then the voice
/// state icons, `speaker` with the green speaker (cards only); `mark` is
/// the colour of the closed mic and headset (pink on pictures).
#[allow(clippy::too_many_arguments)]
fn marks(
    ui: &Ui,
    painter: &Painter,
    x: f32,
    center_y: f32,
    p: &Person,
    color: Color32,
    mark: Color32,
    speaker: bool,
) -> f32 {
    let theme = Theme::of(ui.ctx());
    let mut x = x;
    let start = x;
    if !p.me {
        let lv = volume::level(ui.ctx(), p.id);
        if let Some(w) = volume::chip(ui, painter, x, center_y, lv) {
            x += w + 4.0;
        }
        if lv.muted {
            x += volume::muted_mark(painter, x, center_y, color) + 4.0;
        }
    }
    if speaker {
        x = look::state_icons(
            painter,
            &theme,
            pos2(x, center_y - 7.0),
            p.muted,
            p.deafened,
            true,
            mark,
        ) + 4.0;
    } else if p.muted || p.deafened {
        icons::paint(painter, Icon::MicOff, pos2(x, center_y - 7.0), 14.0, mark);
        x += 14.0;
        if p.deafened {
            icons::paint(
                painter,
                Icon::HeadsetOff,
                pos2(x + 2.0, center_y - 7.0),
                14.0,
                mark,
            );
            x += 16.0;
        }
        x += 4.0;
    }
    (x - start - 4.0).max(0.0)
}

/// Whether `p` has marks on a picture or chip (`:has(...)`).
fn has_marks(ctx: &egui::Context, p: &Person) -> bool {
    let lv = volume::level(ctx, p.id);
    p.muted || p.deafened || (!p.me && (lv.muted || (lv.volume - 1.0).abs() > f64::EPSILON))
}

/// A person card (`.voice-room-participant`, not staged): 16:10, the 64px
/// avatar with its connection dot, the name and the status line, the
/// marks top left, "…" top right on hover.
pub fn card(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid, p: &Person) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let shown = snap(ui.ctx(), rect);
    let painter = ui.painter().clone();
    let id = Id::unique(("voice-card", p.id));
    let hover = ui.rect_contains_pointer(shown);
    let body = ui.interact(shown, id, Sense::click());
    // Who speaks is the avatar's ring and the status line; the border stays.
    painter.rect(
        shown,
        corner(theme.radius.lg),
        c.surface,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let inner_w = shown.width() - 2.0 - 24.0;
    let name_text = if p.me {
        texts::people_you(&p.name)
    } else {
        p.name.clone()
    };
    let name = Block::wrapped(ui, &name_text, text::UI.weight(600.0), c.text, inner_w);
    let status_color = if p.speaking { c.online_text } else { c.muted };
    let status = Block::wrapped(ui, &p.status(), text::META, status_color, inner_w);
    let content = 64.0 + 2.0 + 6.0 + name.height() + status.height();
    let inner_top = rect.top() + 1.0 + 14.0;
    let inner_h = rect.height() - 2.0 - 26.0;
    let top = inner_top + (inner_h - content) / 2.0;
    let cx = rect.center().x;
    let avatar = Rect::from_min_size(pos2(cx - 32.0, top), Vec2::splat(64.0));
    look::avatar(
        ui,
        app.images,
        avatar,
        &p.name,
        p.avatar.as_deref(),
        20.0,
        c.surface,
        p.speaking,
    );
    // The connection dot: 15px at 2 % from the corner, 2px surface ring.
    let dot = Rect::from_min_max(
        avatar.max - Vec2::splat(64.0 * 0.02 + 15.0),
        avatar.max - Vec2::splat(64.0 * 0.02),
    );
    painter.circle_filled(dot.center(), 7.5, c.surface);
    painter.circle_filled(dot.center(), 5.5, c.online);
    let ty = avatar.bottom() + 2.0 + 6.0;
    name.paint(&painter, pos2(cx - name.width() / 2.0, ty));
    status.paint(
        &painter,
        pos2(cx - status.width() / 2.0, ty + name.height()),
    );
    marks(
        ui,
        &painter,
        shown.left() + 1.0 + 8.0,
        shown.top() + 1.0 + 8.0 + 7.0,
        p,
        c.muted,
        c.muted,
        true,
    );
    avatar_button(ui, avatar, id.with("avatar"), server, p);
    if p.member && !p.me {
        let open = menu_open(ui.ctx(), p.id);
        let more = Rect::from_min_size(
            pos2(shown.right() - 1.0 - 6.0 - 28.0, shown.top() + 1.0 + 6.0),
            Vec2::splat(28.0),
        );
        let t = ui.ctx().animate_bool_with_time(
            id.with("more"),
            hover || open,
            crate::theme::motion::FAST,
        );
        if more_button(ui, more, id.with("more-button"), &p.name, t, false).clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
        if body.secondary_clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
    }
}

/// A free seat (`.voice-room-seat`): a dashed 64px circle and "Platz frei".
pub fn seat(ui: &Ui, rect: Rect) {
    seat_sized(ui, rect, 64.0, 14.0, 12.0);
}

/// A free seat with a `diameter` circle, centred between the paddings
/// `top` and `bottom` (the overview's compact seat: 56, 12, 12).
pub fn seat_sized(ui: &Ui, rect: Rect, diameter: f32, top: f32, bottom: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = Block::single(ui, texts::room_free_seat(), text::META, c.subtle);
    let content = diameter + 6.0 + 2.0 + label.height();
    let y = rect.top() + top + (rect.height() - top - bottom - content) / 2.0;
    let r = diameter / 2.0;
    let center = pos2(rect.center().x, y + r);
    look::dashed_circle(ui.painter(), center, r, 1.5, c.border_strong);
    label.paint(
        ui.painter(),
        pos2(
            rect.center().x - label.width() / 2.0,
            y + diameter + 6.0 + 2.0,
        ),
    );
}

/// The width of a chip in the row of people without a camera.
pub fn chip_width(ui: &Ui, p: &Person, open: bool) -> f32 {
    let name = if p.me {
        texts::people_you(&p.name)
    } else {
        p.name.clone()
    };
    let block = Block::single(ui, &name, text::UI.weight(600.0), Color32::WHITE);
    let marks_w = marks_width(ui, p);
    let base = 1.0 + 5.0 + 32.0 + 10.0 + block.width() + 10.0 + marks_w + 14.0 + 1.0;
    if p.member && !p.me {
        base + if open { 10.0 + 28.0 - 8.0 } else { -8.0 }
    } else {
        base
    }
}

fn marks_width(ui: &Ui, p: &Person) -> f32 {
    let lv = volume::level(ui.ctx(), p.id);
    let mut parts = Vec::new();
    if !p.me && !lv.muted && (lv.volume - 1.0).abs() > f64::EPSILON {
        let label = Block::single(
            ui,
            &texts::percent(&((lv.volume * 100.0).round() as i64).to_string()),
            text::META.weight(600.0),
            Color32::WHITE,
        );
        parts.push(label.width() + 12.0);
    }
    if !p.me && lv.muted {
        parts.push(16.0);
    }
    if p.muted || p.deafened {
        parts.push(if p.deafened { 30.0 } else { 14.0 });
    }
    if parts.is_empty() {
        0.0
    } else {
        parts.iter().sum::<f32>() + 4.0 * (parts.len() - 1) as f32
    }
}

/// A chip (`.voice-stage-people > .voice-room-participant`): 44 high,
/// round, a 32px avatar, the name, its marks, "…" opening on hover.
pub fn chip(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid, p: &Person) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let shown = snap(ui.ctx(), rect);
    let painter = ui.painter().clone();
    let id = Id::unique(("voice-chip", p.id));
    let body = ui.interact(shown, id, Sense::click());
    painter.rect(
        shown,
        corner(theme.radius.full),
        c.surface,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let cy = shown.center().y;
    let avatar = Rect::from_min_size(pos2(shown.left() + 1.0 + 5.0, cy - 16.0), Vec2::splat(32.0));
    look::avatar(
        ui,
        app.images,
        avatar,
        &p.name,
        p.avatar.as_deref(),
        13.0,
        c.surface,
        p.speaking,
    );
    let name = if p.me {
        texts::people_you(&p.name)
    } else {
        p.name.clone()
    };
    avatar_button(ui, avatar, id.with("avatar"), server, p);
    let block = Block::single(ui, &name, text::UI.weight(600.0), c.text);
    let x = avatar.right() + 10.0;
    common::paint_mid(&painter, &block, x, cy);
    let mx = x + block.width() + 10.0;
    if has_marks(ui.ctx(), p) {
        marks(ui, &painter, mx, cy, p, c.muted, c.muted, false);
    }
    if p.member && !p.me {
        let open = menu_open(ui.ctx(), p.id);
        let hover = ui.rect_contains_pointer(shown) || open;
        let more = Rect::from_min_size(
            pos2(shown.right() - 1.0 - 14.0 + 8.0 - 28.0, cy - 14.0),
            Vec2::splat(28.0),
        );
        let t = if hover { 1.0 } else { 0.0 };
        if more_button(ui, more, id.with("more-button"), &p.name, t, false).clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
        if body.secondary_clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
    }
}

/// A person in Fokus's or Kino's row: a 40px avatar and the name on a
/// surface card (Kino: the panel at 92 % with a shadow).
pub fn strip_person(
    ui: &mut Ui,
    app: &mut AppCtx,
    rect: Rect,
    server: Uuid,
    p: &Person,
    kino: bool,
    opacity: f32,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    let id = Id::unique(("voice-strip-person", p.id));
    let body = ui.interact(rect, id, Sense::click());
    if opacity <= 0.0 {
        return;
    }
    let fill = if kino {
        let shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha((0.45 * 255.0 * opacity) as u8),
        };
        painter.add(shadow.as_shape(rect, corner(theme.radius.md)));
        c.panel.gamma_multiply(0.92)
    } else {
        c.surface
    };
    painter.rect_filled(rect, corner(theme.radius.md), fill.gamma_multiply(opacity));
    let name = if p.me { None } else { Some(p.name.as_str()) };
    let style = text::META.weight(600.0);
    let block = match name {
        Some(n) => line(ui, n, style, c.text, rect.width() - 16.0),
        None => text::Rich::new(style)
            .push(&p.name, style, c.text)
            .push(
                &format!(" {}", texts::participant_self()),
                style.weight(400.0),
                c.text.gamma_multiply(0.75),
            )
            .layout(ui),
    };
    let content = 40.0 + 6.0 + block.height();
    let top = rect.top() + (rect.height() - content) / 2.0;
    let avatar = Rect::from_min_size(pos2(rect.center().x - 20.0, top), Vec2::splat(40.0));
    look::avatar(
        ui,
        app.images,
        avatar,
        &p.name,
        p.avatar.as_deref(),
        15.0,
        fill,
        p.speaking,
    );
    block.paint(
        &painter,
        pos2(rect.center().x - block.width() / 2.0, avatar.bottom() + 6.0),
    );
    avatar_button(ui, avatar, id.with("avatar"), server, p);
    if has_marks(ui.ctx(), p) {
        marks(
            ui,
            &painter,
            rect.left() + 6.0,
            rect.top() + 6.0 + 7.0,
            p,
            c.muted,
            MARK_ON_PICTURE,
            false,
        );
    }
    if p.member && !p.me {
        let more = Rect::from_min_size(
            pos2(rect.right() - 4.0 - 28.0, rect.top() + 4.0),
            Vec2::splat(28.0),
        );
        let open = menu_open(ui.ctx(), p.id);
        let t = if ui.rect_contains_pointer(rect) || open {
            opacity
        } else {
            0.0
        };
        if more_button(ui, more, id.with("more-button"), &p.name, t, false).clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
        if body.secondary_clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
    }
}

/// What the own Live's tile says about its sound (`sourceProps` of the
/// desktop app); nothing while none is asked for.
pub fn live_sound_notice(sound: &gelabber_call::LiveSound) -> Option<String> {
    use gelabber_call::LiveSound;
    match sound {
        LiveSound::Off => None,
        LiveSound::Sharing => Some(texts::room_sound_sharing().into()),
        LiveSound::Silent(app) => Some(texts::room_sound_silent(app)),
        LiveSound::Unavailable(_) => Some(texts::room_sound_not_captured().into()),
        LiveSound::Ended => Some(texts::room_sound_ended().into()),
    }
}

/// Why pictures stay small here (`nativeVideoLimit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoLimit {
    /// The fast frame transport is blocked.
    Transport,
    /// No GPU drawing of video in this renderer.
    Renderer,
}

/// The badge of a limited picture: its text and `LIMIT_REASON` tooltip.
pub fn limit_badge(limit: VideoLimit) -> (&'static str, &'static str) {
    (
        texts::tile_low_quality(),
        match limit {
            VideoLimit::Transport => texts::tile_limit_transport(),
            VideoLimit::Renderer => texts::tile_limit_renderer(),
        },
    )
}

/// `.voice-video-limit` at the picture's top left (under the Live badge).
fn paint_limit(ui: &Ui, painter: &Painter, rect: Rect, live: bool, id: Id, limit: VideoLimit) {
    let theme = Theme::of(ui.ctx());
    let (label, reason) = limit_badge(limit);
    let block = Block::single(ui, label, text::CAPS.tracking(0.0), Color32::WHITE);
    let badge = Rect::from_min_size(
        rect.min + vec2(8.0, if live { 32.0 } else { 8.0 }),
        vec2(block.width() + 12.0, block.height() + 4.0),
    );
    painter.rect_filled(
        badge,
        corner(theme.radius.xs),
        Color32::from_black_alpha(153),
    );
    block.paint(painter, badge.min + vec2(6.0, 2.0));
    let response = ui.interact(badge, id.with("limit"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
    response.on_hover_text(reason);
}

/// A dark note over the picture's lower part (`.voice-fullscreen-error`).
fn tile_note(ui: &Ui, painter: &Painter, rect: Rect, text_s: &str, pad: Vec2, fill: Color32) {
    let width = rect.width() - 24.0;
    let block = Block::wrapped(
        ui,
        text_s,
        text::UI.line_height(1.4),
        Color32::WHITE,
        width - 2.0 * pad.x,
    );
    let h = block.height() + 2.0 * pad.y;
    let note = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.bottom() - 28.0 - h),
        vec2(width, h),
    );
    painter.rect_filled(note, corner(6.0), fill);
    block.paint(painter, note.min + pad);
    ui.interact(
        note,
        Id::unique(("gelabber-tile-note", text_s)),
        Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text_s));
}

/// Where a picture sits on the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Raster.
    Grid,
    /// Fokus's (or Kino's) picture in front.
    Focus,
    /// Fokus's row.
    Strip,
    /// Kino's front picture.
    KinoFocus,
    /// Kino's small pictures.
    KinoStrip,
}

impl Place {
    fn strip(self) -> bool {
        matches!(self, Self::Strip | Self::KinoStrip)
    }
}

/// What a picture's actions asked for.
#[derive(Default)]
pub struct PictureOut {
    pub select: bool,
    pub pin: bool,
    pub fullscreen: bool,
    pub watch: Option<bool>,
}

/// A camera or Live on the stage (`VoiceTile` in `.voice-stage-item`).
#[allow(clippy::too_many_arguments)]
pub fn picture(
    ui: &mut Ui,
    app: &mut AppCtx,
    rect: Rect,
    pic: &Picture,
    person: Option<&Person>,
    place: Place,
    pinned: bool,
    awake: bool,
    opacity: f32,
) -> PictureOut {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut out = PictureOut::default();
    let id = Id::unique(("voice-picture", pic.id.user, pic.id.kind == Kind::Live));
    let radius = match place {
        Place::KinoFocus => 0.0,
        Place::Strip | Place::KinoStrip => theme.radius.md,
        _ => theme.radius.lg,
    };
    let painter = ui.painter().clone();
    let body = ui.interact(rect, id, Sense::click());
    let hover = ui.rect_contains_pointer(rect);
    if opacity <= 0.0 {
        return out;
    }
    if place == Place::KinoStrip {
        let shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha((0.45 * 255.0 * opacity) as u8),
        };
        painter.add(shadow.as_shape(rect, corner(radius)));
    }
    painter.rect_filled(rect, corner(radius), VIDEO_BG);
    let live = pic.id.kind == Kind::Live;
    let fit = if live || matches!(place, Place::Focus | Place::KinoFocus) {
        Fit::Contain
    } else {
        Fit::Cover
    };
    // Someone else's Live is watched by choice; until then, "Zuschauen".
    let needs_watch = live && !pic.own && !pic.watching;
    let in_window = pic.key.is_some_and(|k| window::is_open(ui.ctx(), k));
    if in_window && let Some(key) = pic.key {
        window::retitle(ui.ctx(), key, &pic.label);
    }
    // The large view over the stage has the picture to itself: one ask
    // and one GPU copy, not a second at the tile's size under it.
    let in_large_view = super::page::large_view(ui.ctx()) == Some(pic.id);
    let frame = pic
        .key
        .filter(|_| !in_window && !in_large_view)
        .and_then(|k| app.call.frame(k));
    let wait_id = id.with("waiting");
    if in_window {
        // The picture plays in its own window; the tile says so.
        let color = Color32::from_white_alpha((0.72 * 255.0) as u8);
        let words = Block::single(ui, texts::tile_in_window(), text::META, color);
        let label = texts::tile_close_window();
        let bw = Block::single(ui, label, text::UI.weight(600.0), Color32::WHITE).width() + 26.0;
        let total = words.height() + 8.0 + 32.0;
        let top = rect.center().y - total / 2.0;
        let at = pos2(rect.center().x - words.width() / 2.0, top);
        words.paint(&painter, at);
        ui.interact(
            Rect::from_min_size(at, words.size()),
            id.with("in-window"),
            Sense::hover(),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Label, true, texts::tile_in_window())
        });
        let button = Rect::from_min_size(
            pos2(rect.center().x - bw / 2.0, top + words.height() + 8.0),
            vec2(bw, 32.0),
        );
        if ui
            .put(button, crate::widgets::Button::new(label).small())
            .clicked()
            && let Some(key) = pic.key
        {
            window::close(ui.ctx(), key);
        }
    }
    match (&frame, needs_watch || in_window || in_large_view) {
        (Some(frame), false) => {
            ui.ctx().data_mut(|d| d.remove::<f64>(wait_id));
            if let Some(key) = pic.key {
                let fps = if place.strip() { 15 } else { 30 };
                video::request_fit(ui, app.call, key, rect, Some(frame), fit, fps);
            }
            video::paint(
                ui,
                id.with("video"),
                rect,
                frame.clone(),
                fit,
                pic.mirror,
                radius,
            );
        }
        (_, true) if in_window || in_large_view => {}
        (_, true) => {
            let label = Block::single(
                ui,
                texts::watch_watch(),
                text::UI.weight(600.0),
                c.accent_ink,
            );
            let button = Rect::from_center_size(
                rect.center(),
                vec2(label.width() + 28.0, label.height() + 18.0),
            );
            let response = ui.interact(button, id.with("watch"), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, texts::watch_watch())
            });
            painter.rect_filled(button, corner(theme.radius.sm), c.accent);
            label.paint(&painter, pos2(button.left() + 14.0, button.top() + 9.0));
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                out.watch = Some(true);
            }
        }
        (None, false) => {
            if let Some(key) = pic.key {
                video::request(ui, app.call, key, rect, 30);
            }
            // A Live is watched again; nothing re-asks for a camera, so its
            // tile offers no "Neu laden".
            let reload = !matches!(pic.id.kind, Kind::Camera);
            if placeholder(
                ui,
                &painter,
                rect,
                wait_id,
                &pic.label,
                pic.key.is_some(),
                reload,
            ) {
                ui.ctx().data_mut(|d| d.remove::<f64>(wait_id));
                if let Some(key) = pic.key {
                    video::forget(ui.ctx(), app.call, key);
                }
                if live && !pic.own {
                    app.call.watch(pic.id.user, false);
                    app.call.watch(pic.id.user, true);
                }
            }
        }
    }
    // The speaking ring lies over the picture, under the labels.
    if pic.speaking {
        look::picture_speaking_edge(ui, &painter, rect, radius);
    }
    let kino_focus = place == Place::KinoFocus;
    let strip = place.strip();
    // The Live badge, top left; beside it, on the own Live, what becomes of
    // its sound (`.voice-video-tags`).
    if live && !kino_focus {
        let style = TextStyle::new(11.0, 600.0, 1.3);
        let label = Block::single(ui, texts::live_badge(), style, c.on_live);
        let inset = if strip { 6.0 } else { 8.0 };
        let badge = Rect::from_min_size(
            rect.min + vec2(inset, inset),
            vec2(label.width() + 12.0, label.height() + 4.0),
        );
        painter.rect_filled(badge, corner(theme.radius.xs), c.live);
        label.paint(&painter, badge.min + vec2(6.0, 2.0));
        let sound = (pic.own && !strip).then(|| app.call.with_state(|s| s.live_sound.clone()));
        if let Some(sound) = sound
            && let Some(notice) = live_sound_notice(&sound)
        {
            let on = matches!(sound, gelabber_call::LiveSound::Sharing);
            let word = Block::single(
                ui,
                if on {
                    texts::tile_with_sound()
                } else {
                    texts::tile_without_sound()
                },
                style,
                c.text,
            );
            let chip = Rect::from_min_size(
                pos2(badge.right() + 4.0, badge.top()),
                vec2(6.0 + 12.0 + 4.0 + word.width() + 6.0, badge.height()),
            );
            painter.rect_filled(chip, corner(theme.radius.xs), c.bg.gamma_multiply(0.82));
            icons::paint(
                &painter,
                if on { Icon::Speaker } else { Icon::SpeakerOff },
                pos2(chip.left() + 6.0, chip.center().y - 6.0),
                12.0,
                // Sound that should go out and does not: noticed, no alarm.
                if on { c.text } else { c.warning },
            );
            word.paint(&painter, pos2(chip.left() + 22.0, badge.top() + 2.0));
            // The full sentence for screen readers and on hover.
            ui.interact(chip, id.with("live-sound"), Sense::hover())
                .on_hover_text(&notice)
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &notice));
        }
    }
    // The name pill, bottom left.
    if !kino_focus {
        name_pill(ui, &painter, rect, pic, person, strip, live);
    }
    if frame.is_some()
        && let Some(limit) = video::limit()
    {
        paint_limit(ui, &painter, rect, live && !kino_focus, id, limit);
    }
    if pic.key.is_some_and(|k| window::failed(ui.ctx(), k)) {
        tile_note(
            ui,
            &painter,
            rect,
            window::failed_text(),
            vec2(10.0, 10.0),
            Color32::from_rgba_unmultiplied(22, 25, 27, 242),
        );
    }
    // "…" for someone else's camera.
    if !live
        && !kino_focus
        && let Some(p) = person.filter(|p| p.member && !p.me)
    {
        let inset = if strip { 4.0 } else { 10.0 };
        let more = Rect::from_min_size(
            rect.max - vec2(inset + 28.0, inset + 28.0),
            Vec2::splat(28.0),
        );
        let open = menu_open(ui.ctx(), p.id);
        let t = if hover || open { 1.0 } else { 0.0 };
        if more_button(ui, more, id.with("more"), &p.name, t, true).clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
        if body.secondary_clicked() {
            open_menu(ui.ctx(), p.id, more);
        }
    }
    // Hover actions, top right.
    let show_actions = hover || (kino_focus && awake);
    let actions_t = ui.ctx().animate_bool_with_time(
        id.with("actions"),
        show_actions,
        crate::theme::motion::FAST,
    );
    let (size, gap, top, right) = match place {
        Place::KinoFocus => (34.0, 6.0, 12.0, 16.0),
        Place::Strip | Place::KinoStrip => (32.0, 2.0, 4.0, 4.0),
        _ => (32.0, 6.0, 10.0, 10.0),
    };
    let mut buttons: Vec<(Icon, &str, bool, u8)> = Vec::new();
    if live && pic.watching {
        buttons.push((Icon::EyeOff, texts::tile_stop_watching(), false, 0));
    }
    // `StreamSoundAction`: the global stream sound, on the picture in front.
    let stream_muted = crate::screens::settings::prefs::get(ui.ctx()).source_audio_muted;
    if live && !pic.own && pic.watching && matches!(place, Place::Focus | Place::KinoFocus) {
        buttons.push((
            if stream_muted {
                Icon::SpeakerOff
            } else {
                Icon::Speaker
            },
            texts::tile_mute_live_sound(),
            stream_muted,
            3,
        ));
    }
    if !kino_focus {
        buttons.push((Icon::Pin, texts::tile_pin(), pinned, 1));
    }
    buttons.push((Icon::Expand, texts::tile_fullscreen(), false, 2));
    // "Weitere Aktionen": someone else's video, which can have a window.
    if !pic.own && pic.key.is_some() {
        buttons.push((Icon::More, texts::tile_more_actions(), false, 4));
    }
    let mut x = rect.right() - right;
    for (icon, label, pressed, action) in buttons.iter().rev() {
        let r = Rect::from_min_size(pos2(x - size, rect.top() + top), Vec2::splat(size));
        x -= size + gap;
        let response = ui.interact(r, id.with(("action", *action)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *label));
        let tip = if *action == 2 {
            texts::tile_fullscreen_title()
        } else {
            label
        };
        let response = response.on_hover_text(tip);
        if actions_t > 0.0 {
            let alpha = if kino_focus {
                0.72
            } else if response.hovered() || *pressed {
                0.82
            } else {
                0.62
            };
            let rr = if kino_focus {
                theme.radius.md
            } else {
                theme.radius.sm
            };
            painter.rect_filled(
                r,
                corner(rr),
                pill(alpha).gamma_multiply(actions_t * opacity),
            );
            let fg = if *pressed { c.accent } else { Color32::WHITE };
            icons::paint_centered(
                &painter,
                *icon,
                r,
                16.0,
                fg.gamma_multiply(actions_t * opacity),
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        if response.clicked() {
            match action {
                0 => out.watch = Some(false),
                1 => out.pin = true,
                3 => crate::screens::settings::prefs::update(ui.ctx(), |p| {
                    p.source_audio_muted = !stream_muted;
                }),
                4 => {
                    let open = more_open(ui.ctx()) == Some(pic.id);
                    set_more_open(ui.ctx(), (!open).then_some((pic.id, r)));
                }
                _ => out.fullscreen = true,
            }
        }
        if *action == 4 {
            tile_menu(ui, pic, r);
        }
    }
    if body.double_clicked() {
        out.fullscreen = true;
    } else if body.clicked() {
        out.select = true;
    }
    if place == Place::Strip && hover {
        // `brightness(1.08)` on a selectable picture.
        painter.rect_filled(rect, corner(radius), Color32::from_white_alpha(5));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    out
}

fn more_id() -> Id {
    Id::unique("gelabber-voice-tile-menu")
}

fn more_open(ctx: &egui::Context) -> Option<stage::PicId> {
    ctx.data(|d| d.get_temp::<Option<(stage::PicId, Rect)>>(more_id()))
        .flatten()
        .map(|(id, _)| id)
}

fn set_more_open(ctx: &egui::Context, open: Option<(stage::PicId, Rect)>) {
    ctx.data_mut(|d| d.insert_temp(more_id(), open));
}

/// `TileMenu` with `OwnWindow`: "Eigenes Fenster" or "Fenster schließen",
/// under the "…" at its end.
fn tile_menu(ui: &mut Ui, pic: &Picture, anchor: Rect) {
    let ctx = ui.ctx().clone();
    if more_open(&ctx) != Some(pic.id) {
        return;
    }
    let Some(key) = pic.key else { return };
    let open = window::is_open(&ctx, key);
    let label = if open {
        texts::tile_close_window()
    } else {
        texts::tile_own_window()
    };
    let shown = Popover::new(
        ("voice-tile-menu", pic.id.user, pic.id.kind == Kind::Live),
        anchor,
    )
    .align_end()
    .show(&ctx, |ui| {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let width = 180.0 - 18.0;
        let block = Block::single(ui, label, text::UI, c.text);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::MenuItemCheckBox, true, open, label)
        });
        crate::widgets::popover::focus_first(&response);
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, corner(theme.radius.sm), c.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        common::paint_mid(ui.painter(), &block, rect.left() + 10.0, rect.center().y);
        response.clicked()
    });
    match shown {
        Some(true) => {
            window::toggle(&ctx, key, &pic.label);
            set_more_open(&ctx, None);
        }
        Some(false) => {}
        None => set_more_open(&ctx, None),
    }
}

/// No picture yet: the label, then a spinner and "Video wird geladen …",
/// after 8 s "Kein Bild empfangen".
/// Returns true when "Neu laden" was pressed.
fn placeholder(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    wait_id: Id,
    label: &str,
    expected: bool,
    reload: bool,
) -> bool {
    let now = ui.input(|i| i.time);
    let since: f64 = ui
        .ctx()
        .data_mut(|d| *d.get_temp_mut_or_insert_with(wait_id, || now));
    let waited = now - since;
    let color = Color32::from_white_alpha((0.72 * 255.0) as u8);
    let style = text::UI;
    if !expected || waited < stage::FRAME_LOADING {
        let block = line(ui, label, style, color, rect.width() - 24.0);
        common::paint_mid(
            painter,
            &block,
            rect.center().x - block.width() / 2.0,
            rect.center().y,
        );
        if expected {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        return false;
    }
    let text = if waited < stage::FRAME_STALLED {
        ui.ctx().request_repaint();
        texts::tile_loading()
    } else {
        texts::tile_no_picture()
    };
    let block = Block::single(ui, text, style, color);
    let stalled = waited >= stage::FRAME_STALLED;
    // A spinner above the words while loading, "Neu laden" under them after.
    let total = 16.0 + 8.0 + block.height();
    let top = rect.center().y - total / 2.0;
    if stalled && !reload {
        let at = rect.center() - block.size() / 2.0;
        block.paint(painter, at);
        return false;
    }
    if stalled {
        let words = rect.center().y - (block.height() + 8.0 + 32.0) / 2.0;
        block.paint(painter, pos2(rect.center().x - block.width() / 2.0, words));
        let label = Block::single(
            ui,
            texts::tile_reload(),
            text::UI.weight(600.0),
            Color32::WHITE,
        );
        let w = label.width() + 24.0 + 2.0;
        let button = Rect::from_min_size(
            pos2(rect.center().x - w / 2.0, words + block.height() + 8.0),
            vec2(w, 32.0),
        );
        return ui
            .put(
                button,
                crate::widgets::Button::new(texts::tile_reload()).small(),
            )
            .clicked();
    }
    {
        let center = pos2(rect.center().x, top + 8.0);
        painter.circle_stroke(center, 7.0, Stroke::new(2.0, Color32::from_white_alpha(64)));
        let a = (now as f32 / 0.9).fract() * std::f32::consts::TAU;
        let points: Vec<egui::Pos2> = (0..=8)
            .map(|k| {
                let t = a + k as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
                center + vec2(t.cos(), t.sin()) * 7.0
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, Color32::WHITE)));
    }
    block.paint(
        painter,
        pos2(rect.center().x - block.width() / 2.0, top + 24.0),
    );
    false
}

/// The label on a picture: marks, the name and " (du)" (400, 75 %) on a
/// dark pill; smaller in the rows.
fn name_pill(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    pic: &Picture,
    person: Option<&Person>,
    strip: bool,
    live: bool,
) {
    let theme = Theme::of(ui.ctx());
    let (style, pad_x, pad_y, inset, gap, radius) = if strip {
        (
            TextStyle::new(11.0, 600.0, 1.3),
            6.0,
            1.0,
            6.0,
            4.0,
            theme.radius.xs,
        )
    } else {
        (
            text::UI.weight(600.0),
            10.0,
            0.0,
            10.0,
            6.0,
            theme.radius.sm,
        )
    };
    let max_w = if strip {
        rect.width() - 12.0
    } else if live {
        rect.width() - 20.0
    } else {
        rect.width() - 58.0
    };
    let marks_w = match person {
        Some(p) if !live && has_marks(ui.ctx(), p) => marks_width(ui, p) + gap,
        _ => 0.0,
    };
    let label = if live || !pic.own {
        line(
            ui,
            &pic.label,
            style,
            Color32::WHITE,
            max_w - 2.0 * pad_x - marks_w,
        )
    } else {
        let own = format!(" {}", texts::participant_self());
        let name = pic.label.trim_end_matches(own.as_str());
        text::Rich::new(style)
            .push(name, style, Color32::WHITE)
            .push(&own, style.weight(400.0), Color32::from_white_alpha(191))
            .layout(ui)
    };
    let h = if strip {
        label.height() + 2.0 * pad_y
    } else {
        28.0
    };
    let w = (label.width() + marks_w + 2.0 * pad_x).min(max_w);
    let pill_rect = Rect::from_min_size(
        pos2(rect.left() + inset, rect.bottom() - inset - h),
        vec2(w, h),
    );
    painter.rect_filled(pill_rect, corner(radius), pill(0.62));
    let cy = pill_rect.center().y;
    let mut x = pill_rect.left() + pad_x;
    if marks_w > 0.0
        && let Some(p) = person
    {
        marks(
            ui,
            painter,
            x,
            cy,
            p,
            Color32::WHITE,
            MARK_ON_PICTURE,
            false,
        );
        x += marks_w;
    }
    common::paint_mid(painter, &label, x, cy);
}
