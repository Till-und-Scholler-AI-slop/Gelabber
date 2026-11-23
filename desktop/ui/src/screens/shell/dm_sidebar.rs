//! The DM sidebar for `/d` routes (components/DmSidebar.tsx,
//! index.css `.dm-sidebar`): "Nachrichten" with "Neue Nachricht", then
//! one 44px row per conversation in a 48px slot.

use egui::{Color32, Id, Rect, Ui, Vec2, pos2};
use gelabber_i18n::{dms as dm_texts, friends, shell};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::row_menu::{self, Owner};
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::route::Route;
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

    let nav_bottom = friend_nav(ui, app, rect);
    let dms = app.client.with_state(|s| s.dms.clone());
    let list = Rect::from_min_max(
        pos2(rect.left(), nav_bottom),
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

// friends.css `.dm-nav`: `padding: 8px 8px 4px`, `gap: 2px` and a 1px
// border under it; index.css `.dm-row`: 44px high, `padding: 0 10px`.
const NAV_PAD_TOP: f32 = 8.0;
const NAV_PAD_BOTTOM: f32 = 4.0;
const NAV_GAP: f32 = 2.0;
const NAV_ROW: f32 = 44.0;
const NAV_INSET: f32 = 10.0;

/// "Freunde" and "Anfragen" above the conversations (#563), with counts of
/// what waits for an answer. Returns where the list starts.
fn friend_nav(ui: &mut Ui, app: &mut AppCtx, rect: Rect) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let (friend_count, request_count) = app.client.with_state(|s| {
        (
            s.friends.as_ref().map_or(0, |f| f.incoming.len()),
            s.dm_requests.as_ref().map_or(0, Vec::len),
        )
    });
    let route = app.nav.route().clone();
    let rows = [
        (
            friends::nav_friends(),
            "/d/friends",
            friend_count,
            route == Route::Friends,
        ),
        (
            friends::nav_requests(),
            "/d/requests",
            request_count,
            route == Route::DmRequests,
        ),
    ];
    let top = rect.top() + 52.0 + NAV_PAD_TOP;
    for (i, (label, href, count, active)) in rows.into_iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(rect.left() + 8.0, top + (NAV_ROW + NAV_GAP) * i as f32),
            Vec2::new(rect.width() - 1.0 - 16.0, NAV_ROW),
        );
        let id = Id::unique(("dm-nav", href));
        let response = ui.interact(row, id, egui::Sense::click());
        let lit = response.hovered();
        let fill = if lit {
            c.hover
        } else if active {
            c.selected
        } else {
            Color32::TRANSPARENT
        };
        common::fill(ui.painter(), row, theme.radius.md, fill);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, label));
        let mut right = row.right() - NAV_INSET;
        if count > 0 {
            let r = count_badge(
                ui.painter(),
                ui,
                pos2(right, row.center().y + 9.0),
                count.min(u32::MAX as usize) as u32,
                None,
            );
            right = r.left() - 10.0;
        }
        // A count leaves the label muted, as on the web (no `.is-unread`).
        let color = if lit || active { c.text } else { c.muted };
        let left = row.left() + NAV_INSET;
        let name = line(ui, label, text::BODY, color, right - left);
        paint_mid(ui.painter(), &name, left, row.center().y);
        common::focus_visible(ui, &response, row, theme.radius.md);
        if response.clicked() {
            app.nav.navigate(href);
        }
    }
    let bottom = top + NAV_ROW * 2.0 + NAV_GAP + NAV_PAD_BOTTOM;
    hairline_h(
        ui.painter(),
        &theme,
        rect.left()..=rect.right() - 1.0,
        bottom,
    );
    bottom + 1.0
}

#[cfg(test)]
mod tests {
    use crate::app::testkit::{FakeApi, Reply, launcher, wait_for, wait_for_label, workspace};
    use crate::platform::paths::testing::TempConfig;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    /// "Freunde" and "Anfragen" as the web draws them (friends.css
    /// `.dm-nav`, index.css `.dm-row`): 44px rows 2px apart under 8px, 4px
    /// and a border below, the conversations 8px under that border, the
    /// label 10px in.
    #[test]
    fn friend_rows_sit_like_the_web() {
        let _config = TempConfig::new("dm-nav-rows");
        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("GET", "/dms") => Some(Reply::json(json!([{
                "id": "00000000-0000-4000-8000-0000000000d1", "kind": "dm",
                "created_at": "2026-01-01T00:00:00Z",
                "peer": {"id": "00000000-0000-4000-8000-00000000007c", "name": "Jonas",
                         "avatar_url": null}
            }]))),
            _ => workspace(method, path),
        });
        let mut h = launcher(&api, "/d");
        wait_for_label(&mut h, "Jonas");
        h.run_steps(4);
        let friends = h.get_by_label("Freunde").rect();
        let requests = h.get_by_label("Anfragen").rect();
        let jonas = h.get_by_label("Jonas").rect();
        // The sidebar starts at the window's top: its 52px header, then 8px.
        assert_eq!(friends.top(), 52.0 + 8.0, "under the header");
        assert_eq!((friends.height(), requests.height()), (44.0, 44.0));
        assert_eq!(requests.top() - friends.bottom(), 2.0, "gap");
        // 4px padding, the 1px border, then the list's 8px.
        assert_eq!(jonas.top() - requests.bottom(), 4.0 + 1.0 + 8.0);

        let image = h.render().expect("render");
        let theme = crate::theme::Theme::dark();
        let is = |c: egui::Color32, x: f32, y: f32| {
            image.get_pixel(x as u32, y as u32).0[..3] == c.to_array()[..3]
        };
        // The divider fills the pixel row 4px under "Anfragen", only that one.
        let line = requests.bottom() + 4.0;
        for x in [friends.left() + 20.0, friends.right() - 20.0] {
            assert!(is(theme.c.border, x, line), "divider at {x},{line}");
            assert!(!is(theme.c.border, x, line - 1.0) && !is(theme.c.border, x, line + 1.0));
        }
        // The label's first ink is 10px in (plus the glyph's own bearing).
        let ink = (0..40)
            .map(|dx| friends.left() + dx as f32)
            .find(|&x| {
                (friends.top() as u32..friends.bottom() as u32)
                    .any(|y| !is(theme.c.panel, x, y as f32))
            })
            .expect("the label");
        let inset = ink - friends.left();
        assert!((10.0..12.0).contains(&inset), "label inset {inset}");
    }

    /// A waiting request shows its count, but "Anfragen" stays muted: the
    /// web's `.dm-row` is `--lr-muted` and the nav rows never get
    /// `.is-unread`.
    #[test]
    fn a_count_leaves_the_label_muted() {
        let _config = TempConfig::new("dm-nav-count");
        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("GET", "/dms/requests") => Some(Reply::json(json!([{
                "id": "00000000-0000-4000-8000-0000000000d2", "kind": "dm",
                "created_at": "2026-01-01T00:00:00Z",
                "peer": {"id": "00000000-0000-4000-8000-00000000007d", "name": "Mara",
                         "avatar_url": null},
                "request": {"incoming": true}
            }]))),
            _ => workspace(method, path),
        });
        let client = gelabber_client::Client::new(&api.origin).expect("client");
        let state = client.clone();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| {
                crate::Launcher::with_app(cc, client.clone(), Some("/d".into()))
            });
        wait_for(&mut h, "the request", |_| {
            state.with_state(|s| s.dm_requests.as_ref().is_some_and(|r| r.len() == 1))
        });
        wait_for_label(&mut h, "Anfragen");
        h.run_steps(4);
        let row = h.get_by_label("Anfragen").rect();

        let image = h.render().expect("render");
        let theme = crate::theme::Theme::dark();
        let px = |x: f32, y: f32| image.get_pixel(x as u32, y as u32).0;
        let ys = row.top() as u32..row.bottom() as u32;
        // The badge is drawn at the row's right end.
        let accent = theme.c.accent.to_array();
        assert!(
            (row.right() as u32 - 40..row.right() as u32).any(|x| ys
                .clone()
                .any(|y| px(x as f32, y as f32)[..3] == accent[..3])),
            "the count badge"
        );
        // The label (well left of the badge) has ink, none of it brighter
        // than the muted colour; the dark theme's full text colour would be.
        let peak = |p: &[u8]| p[..3].iter().copied().max().unwrap_or(0);
        let brightest = (row.left() as u32 + 8..row.left() as u32 + 80)
            .flat_map(|x| ys.clone().map(move |y| peak(&px(x as f32, y as f32))))
            .max()
            .unwrap_or(0);
        let muted = peak(&theme.c.muted.to_array());
        let panel = peak(&theme.c.panel.to_array());
        assert!(brightest > panel + 60, "the label has ink ({brightest})");
        assert!(
            brightest <= muted + 4,
            "label brighter than muted ({brightest} > {muted})"
        );
    }
}
