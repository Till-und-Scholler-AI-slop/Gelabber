//! The room's buttons and chips (room.css, ui.css): "Beitreten" and its
//! secondary buttons, the stage bar's view switch and Kino button, and
//! Kino's exit and label chips.

use egui::{Color32, Id, Rect, Response, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::look::pill;
use super::stage::View;
use crate::icons::{self, Icon};
use crate::screens::shell::common::{self, paint_mid};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{eased, focus_ring, keyboard_focus, snap};

/// The primary lg join button (`.voice-room-join`): min-width 180,
/// `SpeakerIcon 20` + label, at the left of `row`.
pub fn join_button(ui: &mut Ui, theme: &Theme, row: Rect, label: &str, id: Id) -> Response {
    let c = theme.c;
    let style = text::UI.size(14.0).weight(600.0);
    let block = Block::single(ui, label, style, c.accent_ink);
    let content = 20.0 + 8.0 + block.width();
    let width = (content + 40.0 + 2.0).max(180.0);
    let rect = Rect::from_min_size(row.min, Vec2::new(width, 44.0));
    let response = common::click(ui, rect, id);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        c.accent,
        theme.primary_hover,
    );
    let pressed = response.is_pointer_button_down_on();
    let scale = ui.ctx().animate_value_with_time(
        response.id.with("press"),
        if pressed {
            crate::theme::motion::PRESS_SCALE
        } else {
            1.0
        },
        crate::theme::motion::MICRO,
    );
    let shown = snap(
        ui.ctx(),
        Rect::from_center_size(rect.center(), rect.size() * scale),
    );
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(ui.painter(), shown, theme.radius.sm, theme);
    }
    ui.painter()
        .rect_filled(shown, corner(theme.radius.sm), fill);
    let x = rect.center().x - content / 2.0;
    icons::paint(
        ui.painter(),
        Icon::Speaker,
        pos2(x, rect.center().y - 10.0),
        20.0,
        c.accent_ink,
    );
    paint_mid(ui.painter(), &block, x + 28.0, rect.center().y);
    response
}

/// A ghost sm button with its label at the start (`justify-content:
/// flex-start`): 32 high, padding 0 12, `left` its left edge.
pub fn ghost_row(ui: &mut Ui, label: &str, left: f32, width: f32) -> Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let top = ui.cursor().min.y;
    let rect = Rect::from_min_size(pos2(left, top), vec2(width, 32.0));
    ui.allocate_exact_size(
        vec2(ui.available_width().max(1.0), 32.0),
        egui::Sense::hover(),
    );
    let response = common::click(ui, rect, Id::unique(("gelabber-ghost-row", label)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    ui.painter()
        .rect_filled(rect, corner(theme.radius.sm), fill);
    let block = Block::single(ui, label, text::UI, c.text);
    paint_mid(ui.painter(), &block, rect.left() + 12.0, rect.center().y);
    response
}

/// The view switch (`ChipGroup "Ansicht"`): Raster and Fokus in one 36px
/// frame; returns the choice when it changed. `right` is its right edge.
pub fn view_switch(ui: &mut Ui, right: f32, center_y: f32, view: View) -> (Rect, Option<View>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let options = [
        (View::Raster, Icon::Grid, "Raster"),
        (View::Fokus, Icon::Focus, "Fokus"),
    ];
    let widths: Vec<(f32, Block, Block)> = options
        .iter()
        .map(|(v, _, label)| {
            let on = *v == view;
            let a = Block::single(ui, label, text::UI, if on { c.text } else { c.muted });
            let b = Block::single(ui, label, text::UI, c.text);
            (10.0 + 15.0 + 6.0 + a.width() + 10.0, a, b)
        })
        .collect();
    let inner: f32 = widths.iter().map(|w| w.0).sum::<f32>() + 2.0;
    let frame = Rect::from_min_size(
        pos2(right - inner - 8.0, center_y - 18.0),
        vec2(inner + 8.0, 36.0),
    );
    ui.painter().rect(
        frame,
        corner(theme.radius.md),
        c.panel,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let mut x = frame.left() + 4.0;
    let mut chosen = None;
    let id = Id::unique("gelabber-stage-views");
    for (i, ((v, icon, label), (w, block, hover_block))) in options.iter().zip(widths).enumerate() {
        let rect = Rect::from_min_size(pos2(x, center_y - 14.0), vec2(w, 28.0));
        x += w + 2.0;
        let on = *v == view;
        let response = common::click(ui, rect, id.with(i));
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *label));
        if on {
            ui.painter().rect(
                rect,
                corner(theme.radius.sm),
                c.surface,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, corner(theme.radius.sm), c.hover);
        }
        let lit = on || response.hovered();
        let color = if lit { c.text } else { c.muted };
        icons::paint(
            ui.painter(),
            *icon,
            pos2(rect.left() + 10.0, center_y - 7.5),
            15.0,
            color,
        );
        let shown = if lit { &hover_block } else { &block };
        paint_mid(
            ui.painter(),
            shown,
            rect.left() + 10.0 + 15.0 + 6.0,
            center_y,
        );
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(ui.painter(), rect, theme.radius.sm, &theme);
        }
        if response.clicked() && !on {
            chosen = Some(*v);
        }
        if response.has_focus() {
            let step = ui.input(|i| {
                i.key_pressed(egui::Key::ArrowLeft)
                    || i.key_pressed(egui::Key::ArrowRight)
                    || i.key_pressed(egui::Key::ArrowUp)
                    || i.key_pressed(egui::Key::ArrowDown)
            });
            if step {
                let other = 1 - i;
                ui.memory_mut(|m| m.request_focus(id.with(other)));
                chosen = Some(options[other].0);
            }
        }
    }
    (frame, chosen)
}

/// The Kino button (`.voice-stage-kino`): 34 high, `TheaterIcon 15` +
/// "Kino", tooltip "Kino (K)". `right` is its right edge.
pub fn kino_button(ui: &mut Ui, right: f32, center_y: f32) -> (Rect, bool) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = Block::single(ui, "Kino", text::UI, c.text);
    let w = 12.0 + 15.0 + 6.0 + label.width() + 12.0 + 2.0;
    let rect = Rect::from_min_size(pos2(right - w, center_y - 17.0), vec2(w, 34.0));
    let response = common::click(ui, rect, Id::unique("gelabber-stage-kino"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Kino"));
    let response = response.on_hover_text("Kino (K)");
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    ui.painter().rect(
        rect,
        corner(theme.radius.md),
        fill,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Icon::Theater,
        pos2(rect.left() + 13.0, center_y - 7.5),
        15.0,
        c.text,
    );
    paint_mid(
        ui.painter(),
        &label,
        rect.left() + 13.0 + 15.0 + 6.0,
        center_y,
    );
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(ui.painter(), rect, theme.radius.md, &theme);
    }
    (rect, response.clicked())
}

/// Kino's bar, top 24 left 16: "Kino verlassen Esc" and the label of the
/// picture in front. Returns whether the exit was pressed.
pub fn kino_bar(ui: &mut Ui, room: Rect, label: Option<(bool, String, String)>) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let white = Color32::WHITE;
    let grey = Color32::from_rgb(0xc9, 0xcd, 0xd0);
    let top = room.top() + 24.0;
    let cy = top + 17.0;
    let text_s = Block::single(ui, "Kino verlassen", text::UI, white);
    let kbd = Block::single(ui, "Esc", TextStyle::new(11.0, 400.0, 1.4), grey);
    let kbd_w = kbd.width() + 10.0 + 2.0;
    let w = 12.0 + 15.0 + 8.0 + text_s.width() + 8.0 + kbd_w + 12.0;
    let exit = Rect::from_min_size(pos2(room.left() + 16.0, top), vec2(w, 34.0));
    let response = common::click(ui, exit, Id::unique("gelabber-kino-exit"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Kino verlassen"));
    let alpha = if response.hovered() { 0.88 } else { 0.72 };
    ui.painter()
        .rect_filled(exit, corner(theme.radius.md), pill(alpha));
    icons::paint(
        ui.painter(),
        Icon::TheaterExit,
        pos2(exit.left() + 12.0, cy - 7.5),
        15.0,
        white,
    );
    let mut x = exit.left() + 12.0 + 15.0 + 8.0;
    paint_mid(ui.painter(), &text_s, x, cy);
    x += text_s.width() + 8.0;
    let kbd_rect = Rect::from_min_size(
        pos2(x, cy - (kbd.height() + 4.0) / 2.0),
        vec2(kbd_w, kbd.height() + 4.0),
    );
    ui.painter().rect(
        kbd_rect,
        corner(theme.radius.xs),
        Color32::TRANSPARENT,
        Stroke::new(1.0, Color32::from_white_alpha(64)),
        StrokeKind::Inside,
    );
    kbd.paint(ui.painter(), kbd_rect.min + vec2(6.0, 2.0));
    if let Some((live, name, place)) = label {
        let max = room.width() - 160.0 - w - 10.0;
        let mut x = exit.right() + 10.0;
        let chip_left = x;
        let badge =
            live.then(|| Block::single(ui, "Live", TextStyle::new(10.0, 600.0, 1.3), white));
        let strong = Block::single(ui, &name, text::UI.weight(900.0), white);
        let rest = Block::single(ui, &format!("· {place}"), text::UI.weight(400.0), grey);
        let mut content = strong.width() + 8.0 + rest.width();
        if let Some(b) = &badge {
            content += b.width() + 10.0 + 8.0;
        }
        let chip = Rect::from_min_size(
            pos2(chip_left, top),
            vec2((content + 24.0).min(max.max(60.0)), 34.0),
        );
        ui.painter()
            .rect_filled(chip, corner(theme.radius.md), pill(0.72));
        let painter = ui.painter().with_clip_rect(chip.shrink2(vec2(12.0, 0.0)));
        x += 12.0;
        if let Some(b) = badge {
            let r = Rect::from_min_size(
                pos2(x, cy - (b.height() + 2.0) / 2.0),
                vec2(b.width() + 10.0, b.height() + 2.0),
            );
            painter.rect_filled(r, corner(theme.radius.xs), c.danger_bg);
            b.paint(&painter, r.min + vec2(5.0, 1.0));
            x = r.right() + 8.0;
        }
        paint_mid(&painter, &strong, x, cy);
        x += strong.width() + 8.0;
        paint_mid(&painter, &rest, x, cy);
    }
    response.clicked()
}
