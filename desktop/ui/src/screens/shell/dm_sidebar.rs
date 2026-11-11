//! The DM sidebar for `/d` routes (components/DmSidebar.tsx,
//! index.css `.dm-sidebar`): "Nachrichten" with "Neue Nachricht", then
//! one 44px row per conversation in a 48px slot.

use egui::{Color32, Id, Rect, Ui, Vec2, pos2};
use gelabber_i18n::{dms as dm_texts, shell};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::row_menu::{self, Owner};
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::text::{self, Block};
use crate::theme::{Theme, mix};
use crate::widgets::{Button, IconButton, count_badge, hairline_h};

const SLOT: f32 = 48.0;

pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, active: Option<Uuid>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    let (loaded, failed) = app.client.with_state(|s| {
        (
            s.dms_loaded,
            matches!(s.op("bootstrap"), Some(gelabber_client::OpState::Failed(_))),
        )
    });
    if !loaded {
        super::channel_sidebar::placeholder(
            ui,
            app,
            rect,
            failed.then_some(shell::dms_load_failed()),
            &mut || app.client.bootstrap(),
        );
        return;
    }

    let title = Block::single(
        ui,
        dm_texts::sidebar_title(),
        text::TITLE_SM.size(15.0),
        c.text,
    );
    paint_mid(&painter, &title, rect.left() + 16.0, rect.top() + 25.5);
    let plus = Rect::from_min_size(
        pos2(rect.right() - 1.0 - 8.0 - 32.0, rect.top() + 25.5 - 16.0),
        Vec2::splat(32.0),
    );
    if ui
        .put(
            plus,
            IconButton::new(Icon::Plus, dm_texts::new_message())
                .size(32.0)
                .icon_size(18.0),
        )
        .clicked()
    {
        super::dialogs::open_new_message(ui.ctx());
    }
    hairline_h(
        &painter,
        &theme,
        rect.left()..=rect.right() - 1.0,
        rect.top() + 51.0,
    );

    let dms = app.client.with_state(|s| s.dms.clone());
    let list = Rect::from_min_max(
        pos2(rect.left(), rect.top() + 52.0),
        pos2(rect.right() - 1.0, rect.bottom()),
    );
    if dms.is_empty() {
        let block = Block::wrapped(
            ui,
            dm_texts::sidebar_empty(),
            text::BODY.line_height(1.6),
            c.muted,
            list.width() - 44.0,
        );
        block.paint(&painter, pos2(list.left() + 22.0, list.top() + 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(
                list.left() + 22.0,
                list.top() + 24.0 + block.height() + 12.0,
            ),
            Vec2::new(list.width() - 44.0, 32.0),
        )));
        if child
            .add(
                Button::new(dm_texts::new_message())
                    .small()
                    .icon(Icon::Plus),
            )
            .clicked()
        {
            super::dialogs::open_new_message(ui.ctx());
        }
        return;
    }
    let now = chrono::Utc::now();
    let content_h = 8.0 + SLOT * dms.len() as f32 + 12.0;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("dm-sidebar-scroll")
        .max_height(list.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let (area, _) =
                ui.allocate_exact_size(Vec2::new(list.width(), content_h), egui::Sense::hover());
            // The list is one Tab stop; ↑ and ↓ move between conversations,
            // → to a row's "…".
            let nav: Vec<common::NavRow> = dms
                .iter()
                .map(|dm| {
                    let item = Id::unique(("dm-row", dm.id));
                    common::NavRow {
                        key: item,
                        items: vec![item, Id::unique(("dm-more", dm.id))],
                    }
                })
                .collect();
            let preferred = dms.iter().position(|dm| active == Some(dm.id));
            let roved = common::rove(ui, Id::unique("dm-list"), &nav, preferred);
            common::list_hint(ui, area, Id::unique("dm-list"), common::list_hint_text());
            let drafts = crate::screens::chat::with_drafts(ui.ctx());
            for (i, dm) in dms.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(area.left() + 8.0, area.top() + 8.0 + SLOT * i as f32),
                    Vec2::new(area.width() - 16.0, 44.0),
                );
                let item = Id::unique(("dm-row", dm.id));
                let more_id = Id::unique(("dm-more", dm.id));
                let within = roved.has_focus_within(i);
                // The "…" hides once focus leaves the row: Tab comes back to
                // the link.
                let stop = roved
                    .stop
                    .is_some_and(|(r, at)| r == i && (at == 0 || !within))
                    || ui.memory(|m| m.has_focus(item));
                if !ui.is_rect_visible(row) && !stop {
                    continue;
                }
                if roved.has_focus_within(i)
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp)
                    })
                {
                    ui.scroll_to_rect(row, None);
                }
                let response = common::click_stop(ui, row, item, stop);
                let owner = || Owner::Dm(dm.id, dm.peer.name.clone());
                if response.secondary_clicked() || row_menu::menu_key(ui, &response) {
                    row_menu::open(ui.ctx(), owner(), row, Some(item));
                }
                let menu_open = row_menu::is_open_for(ui.ctx(), dm.id);
                let is_active = active == Some(dm.id);
                let (badge, muted, presence) = app.client.with_state(|s| {
                    (
                        s.channel_badge(dm.id, now),
                        s.read.get(&dm.id).is_some_and(|r| r.is_muted(now)),
                        s.presence_of(dm.id, dm.peer.id),
                    )
                });
                // Hover wins over active for DM rows (rule order).
                let lit = response.hovered() || ui.rect_contains_pointer(row) || menu_open;
                let fill = if lit {
                    c.hover
                } else if is_active {
                    c.selected
                } else {
                    Color32::TRANSPARENT
                };
                common::fill(ui.painter(), row, theme.radius.md, fill);
                let under = if fill == Color32::TRANSPARENT {
                    c.panel
                } else {
                    fill
                };
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Link,
                        true,
                        common::with_unread(&dm.peer.name, badge),
                    )
                });
                let unread = badge.count > 0 || badge.dot;
                if unread {
                    common::edge_bar(ui.painter(), &theme, row.left() - 8.0, row.center().y);
                }
                let fade = if muted { 0.5 } else { 1.0 };
                let avatar = Rect::from_min_size(
                    pos2(row.left() + 10.0, row.center().y - 16.0),
                    Vec2::splat(32.0),
                );
                common::avatar(
                    ui,
                    app.images,
                    avatar,
                    &dm.peer.name,
                    dm.peer.avatar_url.as_deref(),
                    12.0,
                    fade,
                    under,
                );
                common::presence_dot(ui.painter(), &theme, avatar, common::presence(presence));
                let mut right = row.right() - 10.0;
                if badge.count > 0 {
                    let r = count_badge(
                        ui.painter(),
                        ui,
                        pos2(right, row.center().y + 9.0),
                        badge.count.min(u32::MAX as i64) as u32,
                        None,
                    );
                    right = r.left() - 10.0;
                }
                if drafts.contains(&dm.id) && !is_active {
                    // `.channel-row-draft` as in the channel list.
                    let pencil = Rect::from_min_size(
                        pos2(right - 13.0, row.center().y - 6.5),
                        Vec2::splat(13.0),
                    );
                    crate::icons::paint(
                        ui.painter(),
                        crate::icons::Icon::Pencil,
                        pencil.min,
                        13.0,
                        c.muted,
                    );
                    let mark = ui.interact(pencil, item.with("draft"), egui::Sense::hover());
                    crate::widgets::announce(&mark, egui::Role::Label, shell::channel_draft());
                    mark.on_hover_text(shell::channel_draft());
                    right = pencil.left() - 10.0;
                }
                let color = if lit || is_active || unread {
                    c.text
                } else {
                    c.muted
                };
                let style = if unread {
                    text::BODY.weight(600.0)
                } else {
                    text::BODY
                };
                let left = avatar.right() + 10.0;
                let name = line(
                    ui,
                    &dm.peer.name,
                    style,
                    mix(under, color, fade.into()),
                    right - left,
                );
                paint_mid(ui.painter(), &name, left, row.center().y);
                common::focus_visible(ui, &response, row, theme.radius.md);
                // `.sidebar-row-actions`: "…" over the row's end in its fill,
                // only while in sight (hover, focus in the row, menu open).
                if lit || within {
                    let actions = Rect::from_min_max(
                        pos2(row.right() - 3.0 - 6.0 - 26.0, row.center().y - 13.0),
                        pos2(row.right() - 3.0, row.center().y + 13.0),
                    );
                    ui.painter()
                        .rect_filled(actions, crate::theme::corner(theme.radius.md), fill);
                    let more = Rect::from_min_size(
                        pos2(actions.left() + 6.0, actions.top()),
                        Vec2::splat(26.0),
                    );
                    let more_stop =
                        roved.is_stop(i, 1) && within || ui.memory(|m| m.has_focus(more_id));
                    let label = shell::channel_actions_label(&dm.peer.name);
                    let pressed = super::channel_sidebar::shell_icon_button_stop(
                        ui,
                        more,
                        Icon::More,
                        14.0,
                        &label,
                        more_id,
                        more_stop,
                    );
                    if pressed.clicked() || row_menu::menu_key(ui, &pressed) {
                        row_menu::open(ui.ctx(), owner(), more, Some(more_id));
                        continue;
                    }
                }
                if response.clicked() {
                    app.nav.navigate(&format!("/d/{}", dm.id));
                }
            }
        });
}
