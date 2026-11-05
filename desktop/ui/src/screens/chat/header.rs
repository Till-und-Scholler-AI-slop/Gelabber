//! The conversation header (chat.css `.lr-channel-header`,
//! MessagePane.tsx, ChannelPage.tsx, DmChannelPage.tsx): the title, the
//! search field with its Strg F hint, the bell and the member toggle.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, TextBuffer, Ui, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::images::Images;
use crate::screens::layout;
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::tooltip::{Side, Tooltip};
use crate::widgets::{Avatar, Presence, eased};

pub const HEIGHT: f32 = 52.0;
const TITLE: TextStyle = TextStyle::new(15.0, 650.0, 1.5).tracking(-0.1 / 15.0);
const SEARCH: TextStyle = TextStyle::new(13.0, 400.0, 1.5);

pub enum Title<'a> {
    Channel {
        name: &'a str,
        voice: bool,
    },
    Dm {
        name: &'a str,
        avatar: Option<&'a str>,
        presence: Presence,
    },
}

pub struct HeaderOut {
    pub search_changed: bool,
    pub search_focused: bool,
    pub peer_clicked: bool,
    pub bell_clicked: bool,
}

pub fn search_id(channel: uuid::Uuid) -> egui::Id {
    egui::Id::unique(("gelabber-header-search", channel))
}

/// Draws the header at the top of `page`.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    page: Rect,
    channel: uuid::Uuid,
    title: Title,
    members_toggle: bool,
    muted: bool,
    search: &mut String,
    images: &Images,
) -> HeaderOut {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let rect = Rect::from_min_size(page.min, vec2(page.width(), HEIGHT));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, c.bg);
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.left(), rect.bottom() - 1.0), rect.max),
        0.0,
        c.border,
    );
    let mid = rect.top() + (HEIGHT - 1.0) / 2.0;
    let mut out = HeaderOut {
        search_changed: false,
        search_focused: false,
        peer_clicked: false,
        bell_clicked: false,
    };
    // Right side, from the end: toggle, bell, search.
    let mut right = rect.right() - 12.0;
    let toggle_rect = members_toggle.then(|| {
        let r = Rect::from_min_size(pos2(right - 32.0, mid - 16.0), Vec2::splat(32.0));
        right -= 32.0 + 10.0;
        r
    });
    let bell_rect = Rect::from_min_size(pos2(right - 32.0, mid - 16.0), Vec2::splat(32.0));
    right -= 32.0 + 10.0;

    let max_title = right - 260.0 - 10.0 - rect.left() - 20.0;
    // NotificationBell's aria-label: "#allgemein" or the person.
    let bell_name = match &title {
        Title::Channel { name, .. } => format!("Benachrichtigungen für #{name}"),
        Title::Dm { name, .. } => format!("Benachrichtigungen für {name}"),
    } + if muted { " (stumm)" } else { "" };
    match title {
        Title::Channel { name, voice } => {
            let x = rect.left() + 20.0;
            icons::paint(
                ui.painter(),
                if voice { Icon::Speaker } else { Icon::Hash },
                pos2(x, mid - 9.0),
                18.0,
                c.muted,
            );
            let mut job = crate::text::job(name, TITLE, c.text, Some(max_title.max(40.0)));
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let block = Block::new(ui, job, TITLE);
            block.paint(ui.painter(), pos2(x + 28.0, mid - TITLE.line_px() / 2.0));
        }
        Title::Dm {
            name,
            avatar,
            presence,
        } => {
            let mut job = crate::text::job(name, TITLE, c.text, Some(max_title.max(40.0)));
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let block = Block::new(ui, job, TITLE);
            let button = Rect::from_min_size(
                pos2(rect.left() + 14.0, mid - 20.0),
                vec2(6.0 + 32.0 + 10.0 + block.width() + 6.0, 40.0),
            );
            let response = ui.interact(
                button,
                egui::Id::unique(("gelabber-dm-peer", channel)),
                Sense::click(),
            );
            // The button reads its contents: the presence, then the name.
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("{} {name}", presence.label()),
                )
            });
            let fill = eased(
                ui.ctx(),
                response.id.with("hover"),
                response.hovered(),
                Color32::TRANSPARENT,
                c.hover,
            );
            ui.painter().rect_filled(button, corner(7.0), fill);
            let avatar_rect =
                Rect::from_min_size(pos2(button.left() + 6.0, mid - 16.0), Vec2::splat(32.0));
            Avatar::new(name)
                .image(avatar, images)
                .presence(Some(presence))
                .paint(ui, avatar_rect);
            block.paint(
                ui.painter(),
                pos2(avatar_rect.right() + 10.0, mid - TITLE.line_px() / 2.0),
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            Tooltip::new(&format!("Profil von {name}")).on_hover(&response, Side::Below);
            out.peer_clicked = response.clicked();
        }
    }

    // The search field: 200 wide, 260 while focused or filled.
    let id = search_id(channel);
    let focused = ui.memory(|m| m.has_focus(id));
    let wide = focused || !search.is_empty();
    let width =
        ui.ctx()
            .animate_value_with_time(id.with("width"), if wide { 260.0 } else { 200.0 }, 0.1);
    let field = Rect::from_min_size(pos2(right - width, mid - 15.0), vec2(width, 30.0));
    // Clicking the frame focuses the input; Tab reaches only the input.
    let field_resp = ui.interact(field, id.with("box"), Sense::CLICK);
    let hot = field_resp.hovered() || focused || !search.is_empty();
    let border = if focused {
        mix(c.border, c.accent, 0.7)
    } else if hot {
        mix(c.border, c.text, 0.18)
    } else {
        c.border
    };
    ui.painter().rect(
        field,
        corner(7.0),
        c.panel,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Icon::Search,
        pos2(field.left() + 11.0, mid - 8.0),
        16.0,
        if hot { c.text } else { c.muted },
    );
    let kbd = Block::single(ui, "Strg F", TextStyle::new(11.0, 400.0, 1.5), c.muted);
    let kbd_rect = Rect::from_min_size(
        pos2(field.right() - 11.0 - kbd.width() - 12.0, mid - 9.25),
        vec2(kbd.width() + 12.0, 18.5),
    );
    let input_left = field.left() + 11.0 + 16.0 + 8.0;
    let input_right = if search.is_empty() {
        kbd_rect.left()
    } else {
        field.right() - 11.0
    };
    let input = Rect::from_min_max(
        pos2(input_left, field.top() + 1.0),
        pos2(input_right, field.bottom() - 1.0),
    );
    // Strg+F focuses the field; Esc clears it, then leaves it.
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let had_focus = focused || ui.memory(|m| m.had_focus_last_frame(id));
    if had_focus && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        // Not also "mark as read"; egui already let go of the field.
        if !search.is_empty() {
            search.clear();
            out.search_changed = true;
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    if field_resp.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
        let mut job = crate::text::job(buf.as_str(), SEARCH, color, None);
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let edit = ui
        .scope_builder(egui::UiBuilder::new().max_rect(input), |ui| {
            egui::TextEdit::singleline(search)
                .id(id)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    left: 0,
                    right: 0,
                    top: 4,
                    bottom: 4,
                }))
                .desired_width(input.width())
                .min_size(input.size())
                .char_limit(200)
                .layouter(&mut layouter)
                .show(ui)
        })
        .inner;
    // The web names the field by its aria-label "Suchen".
    let edit_response = edit.response.response.clone().accessible_name("Suchen");
    out.search_changed |= edit_response.changed();
    out.search_focused = edit_response.has_focus();
    if search.is_empty() {
        let hint = Block::single(ui, "Suchen", SEARCH, mix(c.panel, c.subtle, 0.8));
        hint.paint(ui.painter(), pos2(input_left, mid - SEARCH.line_px() / 2.0));
        ui.painter().rect(
            kbd_rect,
            corner(5.0),
            Color32::TRANSPARENT,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        kbd.paint(
            ui.painter(),
            pos2(kbd_rect.left() + 6.0, kbd_rect.top() + 1.0),
        );
    }
    if field_resp.hovered() || edit.response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    Tooltip::new("Wörter oder eine Phrase in Anführungszeichen").on_hover(&field_resp, Side::Below);

    // The bell and the member toggle (`.lr-header-icon`).
    let bell = header_icon(
        ui,
        bell_rect,
        if muted { Icon::BellOff } else { Icon::Bell },
        16.0,
        false,
        if muted {
            "Stummgeschaltet"
        } else {
            "Benachrichtigungen"
        },
        id.with("bell"),
    );
    bell.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &bell_name));
    out.bell_clicked = bell.clicked();
    if let Some(r) = toggle_rect {
        let on = layout::members_visible(ui.ctx());
        let label = if on {
            "Mitgliederliste ausblenden"
        } else {
            "Mitgliederliste einblenden"
        };
        let toggle = header_icon(ui, r, Icon::Users, 18.0, on, label, id.with("members"));
        toggle.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if toggle.clicked() {
            layout::set_members_visible(ui.ctx(), !on);
        }
    }
    out
}

fn header_icon(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    size: f32,
    on: bool,
    label: &str,
    id: egui::Id,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let response = ui.interact(rect, id, Sense::click());
    let lit = on || response.hovered();
    let fill = eased(
        ui.ctx(),
        id.with("fill"),
        lit,
        Color32::TRANSPARENT,
        c.hover,
    );
    ui.painter().rect_filled(rect, corner(7.0), fill);
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(ui.painter(), rect, 7.0, &theme);
    }
    icons::paint_centered(
        ui.painter(),
        icon,
        rect,
        size,
        if lit { c.text } else { c.muted },
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    Tooltip::new(label).on_hover(&response, Side::Below);
    response
}
