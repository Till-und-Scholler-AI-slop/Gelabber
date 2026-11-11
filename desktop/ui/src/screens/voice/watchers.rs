//! Who watches a Live (voice/LiveWatchers.tsx, room.css `.voice-watchers`):
//! the list that the count on a Live's caption and the own Live's call bar
//! button open. Width 272: a title and where the Live runs, who watches,
//! then apart the others in the channel; the own Live's list ends with
//! "Live beenden" and what the caller adds under it.

use egui::{Id, Rect, Sense, Ui, Vec2, pos2, vec2};
use gelabber_i18n::voice as texts;
use uuid::Uuid;

use super::session::Room;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::shell::common::{self, line};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::{Popover, hairline_h};

/// `.ui-popover.voice-watchers`.
const WIDTH: f32 = 272.0;

/// "3 schauen zu", "1 schaut zu", "Niemand schaut zu".
pub fn title(count: usize) -> String {
    if count == 0 {
        texts::watchers_title_none().to_owned()
    } else {
        texts::watchers_title(count as u64)
    }
}

/// The count's name: "{n} schauen zu – Liste anzeigen".
pub fn count_label(count: usize) -> String {
    if count == 0 {
        texts::watchers_count_label_none().to_owned()
    } else {
        texts::watchers_count_label(count as u64)
    }
}

/// How many watch `owner`'s Live in `room`.
pub fn count(room: &Room, owner: Uuid) -> usize {
    room.watchers.get(&owner).map_or(0, |(w, _)| w.len())
}

/// The list a caption's count opened: whose Live, under which rect.
fn open_id() -> Id {
    Id::unique("gelabber-voice-watchers-open")
}

/// A press on the count of `owner`'s Live at `anchor`: opens its list, or
/// closes it when it is open.
pub fn toggle(ctx: &egui::Context, owner: Uuid, anchor: Rect) {
    let next = (!is_open(ctx, owner)).then_some((owner, anchor));
    ctx.data_mut(|d| d.insert_temp(open_id(), next));
}

/// Whether the list of `owner`'s Live is open from its caption.
pub fn is_open(ctx: &egui::Context, owner: Uuid) -> bool {
    ctx.data(|d| d.get_temp::<Option<(Uuid, Rect)>>(open_id()))
        .flatten()
        .is_some_and(|(o, _)| o == owner)
}

/// The list a caption's count opened (`top-start`), while its Live runs.
pub fn show(ui: &mut Ui, app: &mut AppCtx, room: &Room) {
    let ctx = ui.ctx().clone();
    let Some((owner, anchor)) = ctx
        .data(|d| d.get_temp::<Option<(Uuid, Rect)>>(open_id()))
        .flatten()
    else {
        return;
    };
    let keep = room.watchers.contains_key(&owner)
        && matches!(list(ui, app, room, owner, anchor, false, None), Some(false));
    if !keep {
        ctx.data_mut(|d| d.insert_temp(open_id(), None::<(Uuid, Rect)>));
    }
}

/// The list of `owner`'s Live over `anchor`, from its start or (`end`) its
/// end. `None` once it should close (Esc, a press outside), `Some(true)`
/// once "Live beenden" ended the own Live. `extra` draws under that.
pub fn list(
    ui: &mut Ui,
    app: &mut AppCtx,
    room: &Room,
    owner: Uuid,
    anchor: Rect,
    end: bool,
    extra: Option<&mut dyn FnMut(&mut Ui)>,
) -> Option<bool> {
    let ctx = ui.ctx().clone();
    let (watching, others) = room.watchers.get(&owner).cloned().unwrap_or_default();
    let own = Some(owner) == room.me;
    let popover = Popover::new(("voice-watchers", owner), anchor)
        .min_width(WIDTH)
        .above();
    let popover = if end { popover.align_end() } else { popover };
    popover.show(&ctx, |ui| {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let width = WIDTH - 18.0;
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        // The head: padding 6 8 8, the title and where it runs.
        let heading = title(watching.len());
        let place = if own {
            texts::watchers_own_subtitle(&room.channel_name)
        } else {
            let name = room
                .person(owner)
                .map_or(texts::people_member().to_owned(), |p| p.name.clone());
            texts::watchers_subtitle(&name, &room.channel_name)
        };
        let strong = line(ui, &heading, text::UI.weight(600.0), c.text, width - 16.0);
        let small = line(ui, &place, text::META, c.muted, width - 16.0);
        let (head, _) = ui.allocate_exact_size(
            vec2(width, 6.0 + strong.height() + 2.0 + small.height() + 8.0),
            Sense::hover(),
        );
        let at = head.min + vec2(8.0, 6.0);
        strong.paint(ui.painter(), at);
        let below = at + vec2(0.0, strong.height() + 2.0);
        small.paint(ui.painter(), below);
        for (i, (words, pos, size)) in
            [(&heading, at, strong.size()), (&place, below, small.size())]
                .into_iter()
                .enumerate()
        {
            ui.interact(
                Rect::from_min_size(pos, size),
                Id::unique(("voice-watchers-head", owner, i)),
                Sense::hover(),
            )
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, words));
        }
        for id in &watching {
            person_row(ui, app, room, *id, false, width);
        }
        if !others.is_empty() {
            let label = texts::watchers_not_watching().to_uppercase();
            let caps = line(ui, &label, text::CAPS, c.muted, width - 16.0);
            let (r, response) =
                ui.allocate_exact_size(vec2(width, 10.0 + caps.height() + 2.0), Sense::hover());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Label, true, texts::watchers_not_watching())
            });
            caps.paint(ui.painter(), r.min + vec2(8.0, 10.0));
            for id in &others {
                person_row(ui, app, room, *id, true, width);
            }
        }
        let mut ended = false;
        if own {
            ui.add_space(4.0);
            let top = ui.cursor().min;
            hairline_h(ui.painter(), &theme, top.x..=top.x + width, top.y);
            ui.add_space(1.0 + 4.0);
            ended = end_live(ui, width);
            if ended {
                app.call.stop_live();
            }
            if let Some(extra) = extra {
                extra(ui);
            }
        }
        ended
    })
}

/// A watcher or someone else in the channel: the 24px avatar and the name
/// ("(du)" on the own), padding 8, gap 10; the others muted.
fn person_row(ui: &mut Ui, app: &AppCtx, room: &Room, id: Uuid, muted: bool, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let Some(p) = room.person(id) else {
        return;
    };
    let label = room.name_of(id);
    let (row, response) = ui.allocate_exact_size(vec2(width, 8.0 + 24.0 + 8.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, true, &label));
    let avatar = Rect::from_min_size(pos2(row.left() + 8.0, row.top() + 8.0), Vec2::splat(24.0));
    common::avatar(
        ui,
        app.images,
        avatar,
        &p.name,
        p.avatar.as_deref(),
        10.0,
        1.0,
        c.elevated,
    );
    let name = line(
        ui,
        &label,
        text::UI,
        if muted { c.muted } else { c.text },
        row.right() - 8.0 - (avatar.right() + 10.0),
    );
    common::paint_mid(ui.painter(), &name, avatar.right() + 10.0, row.center().y);
}

/// "Live beenden" (`.voice-menu-item.is-danger`): the square stop and the
/// words in the danger colour, 36 high; true when pressed.
fn end_live(ui: &mut Ui, width: f32) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = texts::live_end();
    let words = Block::single(ui, label, text::UI, c.danger);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, (words.height() + 16.0).max(36.0)),
        Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    crate::widgets::popover::focus_first(&response);
    let keyed = response.has_focus() && crate::widgets::keyboard_focus(ui.ctx());
    if response.hovered() || keyed {
        ui.painter()
            .rect_filled(rect, corner(theme.radius.sm), c.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        Icon::Stop,
        pos2(rect.left() + 10.0, rect.center().y - 8.0),
        16.0,
        c.danger,
    );
    common::paint_mid(
        ui.painter(),
        &words,
        rect.left() + 10.0 + 16.0 + 10.0,
        rect.center().y,
    );
    response.clicked()
}
