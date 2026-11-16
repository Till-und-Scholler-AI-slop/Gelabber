//! Blocking people (#565, web/src/blocks/): the composer's place in a DM
//! nobody can write in (`ClosedBar`) and the "Blockiert" settings area
//! (`BlockedArea`).

use chrono::{Datelike, Timelike};
use egui::{Id, Rect, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{DirectMessage, FriendPerson, OpState};
use gelabber_i18n::{blocks as t, intl};

use crate::app::AppCtx;
use crate::screens::settings::controls;
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::{Button, hairline_h, load_error};

/// `ClosedBar`: "Du hast {name} blockiert." with "Blockierung aufheben"
/// for the one who blocks; for the other a neutral line that does not say
/// why. As tall as the request bar.
pub fn closed_bar(ui: &mut Ui, app: &mut AppCtx, rect: Rect, dm: &DirectMessage) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    hairline_h(ui.painter(), &theme, rect.x_range(), rect.top());
    let inner = rect.shrink2(vec2(16.0, 12.0));
    if !dm.blocked {
        let note = Block::wrapped(
            ui,
            t::composer_closed(),
            text::BODY.line_height(1.5),
            c.muted,
            inner.width(),
        );
        let at = pos2(inner.left(), inner.center().y - note.height() / 2.0);
        note.paint(ui.painter(), at);
        return;
    }
    let user = dm.peer.id;
    let busy = app
        .client
        .with_state(|s| matches!(s.op(&format!("unblock:{user}")), Some(OpState::Pending)));
    let unblock = Button::new(t::action_unblock()).ghost().disabled(busy);
    let bw = unblock.natural_width(ui);
    let note = Block::wrapped(
        ui,
        &t::composer_blocked(&dm.peer.name),
        text::BODY.line_height(1.5),
        c.text,
        (inner.width() - bw - 16.0).max(80.0),
    );
    note.paint(
        ui.painter(),
        pos2(inner.left(), inner.center().y - note.height() / 2.0),
    );
    let b = Rect::from_min_size(
        pos2(inner.right() - bw, inner.center().y - 18.0),
        vec2(bw, 36.0),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    if child.put(b, unblock).clicked() {
        app.client.unblock_user(user);
    }
}

/// Einstellungen → Blockiert (`BlockedArea`): the hint, then everyone the
/// user blocks, newest first, each with "Blockierung aufheben".
pub fn area(ui: &mut Ui, app: &mut AppCtx) {
    let c = Theme::of(ui.ctx()).c;
    let width = ui.available_width();
    let (list, failed) = app.client.with_state(|s| {
        (
            s.blocks.clone(),
            matches!(s.op("blocks"), Some(OpState::Failed(_))),
        )
    });
    controls::paragraph(
        ui,
        t::area_hint(),
        text::BODY.line_height(1.6),
        c.muted,
        width,
    );
    ui.add_space(16.0);
    let Some(list) = list else {
        if failed {
            if load_error(ui, t::area_load_failed(), false) {
                app.client.load_blocks();
            }
        } else {
            crate::widgets::skeleton::main(
                ui,
                Rect::from_min_size(ui.cursor().min, vec2(width, 168.0)),
                crate::widgets::skeleton::Kind::List,
                Id::unique("blocks-skeleton"),
            );
        }
        return;
    };
    if list.is_empty() {
        controls::paragraph(
            ui,
            t::area_empty(),
            text::BODY.line_height(1.6),
            c.muted,
            width,
        );
        return;
    }
    for person in &list {
        let user = person.id;
        let local = person.created_at.with_timezone(&chrono::Local);
        let since = t::area_since(&intl::format_date(
            &intl::DateTime {
                year: local.year(),
                month: local.month(),
                day: local.day(),
                weekday: local.weekday().num_days_from_monday(),
                hour: local.hour(),
                minute: local.minute(),
            },
            intl::DateStyle::Long,
        ));
        let busy = app
            .client
            .with_state(|s| matches!(s.op(&format!("unblock:{user}")), Some(OpState::Pending)));
        let row = FriendPerson {
            id: user,
            name: person.name.clone(),
            avatar_url: person.avatar_url.clone(),
        };
        let buttons = vec![
            Button::new(t::action_unblock())
                .small()
                .ghost()
                .disabled(busy),
        ];
        if super::friends::person_row(ui, app, &row, Some(&since), None, buttons, width).is_some() {
            app.client.unblock_user(user);
        }
    }
}
