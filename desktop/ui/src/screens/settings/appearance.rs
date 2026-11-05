//! "Darstellung" (pages/AppearanceSettings.tsx): light, dark or system,
//! text size, compact voice rooms and reduced motion.

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::controls::{self, Scope, Status};
use super::prefs::{self, UiScale};
use super::themes::{self, preset};
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{BaseColors, Theme, corner};
use crate::widgets::{eased, focus_ring, keyboard_focus};

const MODES: [(&str, &str, &str); 3] = [
    ("light", "Hell", "Gelabber Hell"),
    ("dark", "Dunkel", "Gelabber Dunkel"),
    ("system", "System", "Folgt dem Gerät"),
];

#[derive(Default)]
pub struct State {
    status: Status,
    waiting: bool,
}

/// `ModeSwatch`: page, panel strip, a text bar and an accent bar.
fn swatch(ui: &Ui, rect: Rect, colors: &BaseColors, radius: egui::CornerRadius, edges: [bool; 4]) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, colors.background);
    let inner = Rect::from_min_max(
        pos2(
            rect.left() + if edges[3] { 1.0 } else { 0.0 },
            rect.top() + 1.0,
        ),
        pos2(
            rect.right() - if edges[1] { 1.0 } else { 0.0 },
            rect.bottom() - 1.0,
        ),
    );
    let strip = Rect::from_min_size(inner.min, vec2(inner.width() * 0.3, inner.height()));
    painter
        .with_clip_rect(rect)
        .rect_filled(strip, egui::CornerRadius::ZERO, colors.panel);
    let bar = Rect::from_min_size(
        pos2(inner.left() + inner.width() * 0.4, inner.top() + 14.0),
        vec2(inner.width() * 0.4, 6.0),
    );
    painter.rect_filled(bar, corner(3.0), colors.text.gamma_multiply(0.8));
    let accent = Rect::from_min_size(
        pos2(
            inner.left() + inner.width() * 0.4,
            inner.bottom() - 12.0 - 10.0,
        ),
        vec2(inner.width() * 0.24, 10.0),
    );
    painter.rect_filled(accent, corner(3.0), colors.accent);
    // The border, only on the sides that keep it.
    let stroke = Stroke::new(1.0, colors.border);
    let r = rect.shrink(0.5);
    if edges[0] {
        painter.hline(r.left()..=r.right(), r.top(), stroke);
    }
    if edges[2] {
        painter.hline(r.left()..=r.right(), r.bottom(), stroke);
    }
    if edges[1] {
        painter.vline(r.right(), r.top()..=r.bottom(), stroke);
    }
    if edges[3] {
        painter.vline(r.left(), r.top()..=r.bottom(), stroke);
    }
}

pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let sync = themes::sync_state(ui.ctx(), app);
    if s.waiting && !sync.saving {
        s.waiting = false;
        match &sync.error {
            None => s.status.success(ui, "Gespeichert."),
            Some(_) => s.status.error(ui, "Nicht gespeichert – erneut versuchen"),
        }
    }
    let active = sync.doc.active.clone();
    controls::group_legend(ui, "Farbschema", Some(Scope::Account));
    ui.add_space(4.0);
    let top = ui.cursor().min;
    let gap = 12.0;
    let width = ui.available_width().min(640.0);
    let card_w = (width - 2.0 * gap) / 3.0;
    let card_h = 126.3;
    for (i, (id, label, hint)) in MODES.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(top.x + i as f32 * (card_w + gap), top.y),
            vec2(card_w, card_h),
        );
        let wid = Id::unique(("gelabber-mode", *id));
        let response = ui.interact(rect, wid, Sense::click());
        let checked = active == *id;
        let fill = eased(ui.ctx(), wid, response.hovered(), c.panel, c.hover);
        let painter = ui.painter();
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, rect, theme.radius.md, &theme);
        }
        painter.rect(
            rect,
            corner(theme.radius.md),
            fill,
            Stroke::new(1.0, if checked { c.accent } else { c.border_strong }),
            StrokeKind::Inside,
        );
        if checked {
            painter.rect_stroke(
                rect.shrink(1.0),
                corner(theme.radius.md - 1.0),
                Stroke::new(1.0, c.accent),
                StrokeKind::Inside,
            );
        }
        let sw = Rect::from_min_size(
            pos2(rect.left() + 13.0, rect.top() + 11.0),
            vec2(card_w - 26.0, 56.0),
        );
        let r = theme.radius.sm.round() as u8;
        if *id == "system" {
            let (l, d) = (preset("light").colors, preset("dark").colors);
            let half = sw.width() / 2.0;
            let left = Rect::from_min_size(sw.min, vec2(half, sw.height()));
            let right =
                Rect::from_min_size(pos2(sw.left() + half, sw.top()), vec2(half, sw.height()));
            swatch(
                ui,
                left,
                &l,
                egui::CornerRadius {
                    nw: r,
                    sw: r,
                    ne: 0,
                    se: 0,
                },
                [true, false, true, true],
            );
            swatch(
                ui,
                right,
                &d,
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: r,
                    se: r,
                },
                [true, true, true, false],
            );
        } else {
            swatch(
                ui,
                sw,
                &preset(id).colors,
                corner(theme.radius.sm),
                [true; 4],
            );
        }
        controls::radio_mark(ui, pos2(rect.left() + 13.0, rect.top() + 78.0), checked);
        let x = rect.left() + 43.0;
        Block::single(ui, label, text::UI, c.text).paint(ui.painter(), pos2(x, rect.top() + 77.0));
        Block::single(ui, hint, text::META, c.muted).paint(
            ui.painter(),
            pos2(x, rect.top() + 77.0 + text::UI.line_px() + 2.0),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && !checked && !sync.saving && sync.ready {
            themes::change(ui.ctx(), app, themes::Op::Apply(id.to_string()));
            s.waiting = true;
        }
    }
    ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
    if let Some(error) = sync.error.as_deref().filter(|_| !sync.ready) {
        ui.add_space(6.0);
        let w = ui.available_width();
        controls::paragraph(ui, error, text::META, c.danger, w);
    }
    ui.add_space(10.0);
    // "Gerade aktiv: …" + the link to Themes.
    let other =
        (!MODES.iter().any(|m| m.0 == active)).then(|| themes::theme_name(&sync.doc, &active));
    let lead = other
        .map(|n| format!("Gerade aktiv: {n}. "))
        .unwrap_or_default();
    let rich = text::Rich::new(controls::HINT)
        .push(&lead, controls::HINT, c.muted)
        .push_underlined("Weitere Themes", controls::HINT, c.accent_text)
        .layout(ui);
    let link_top = ui.cursor().min;
    text::show(ui, &rich);
    if let Some(r) = rich.section_rect(link_top, 1) {
        let resp = ui.interact(r, Id::unique("gelabber-more-themes"), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            super::go(ui.ctx(), app, "/settings/themes");
        }
    }
    let w = ui.available_width();
    s.status.ui(ui, w);
    ui.add_space(24.0 + 32.0);

    let mut p = prefs::get(ui.ctx());
    let before = p.clone();
    controls::group_legend(ui, "Ansicht", Some(Scope::Device));
    // Stacked row: label, hint, chips.
    let row_top = ui.cursor().min;
    ui.add_space(12.0);
    controls::paint(ui, ui.cursor().min, "Textgröße", text::UI, c.text);
    ui.add_space(text::UI.line_px() + 4.0);
    controls::paint(
        ui,
        ui.cursor().min,
        "Gilt für die ganze Oberfläche.",
        controls::HINT,
        c.muted,
    );
    ui.add_space(18.0 + 10.0);
    let labels: Vec<&str> = UiScale::ALL.iter().map(|s| s.label()).collect();
    let current = UiScale::ALL
        .iter()
        .position(|s| *s == p.ui_scale)
        .unwrap_or(0);
    if let Some(i) = controls::chips(ui, "ui-scale", &labels, current, 32.0, None) {
        p.ui_scale = UiScale::ALL[i];
    }
    ui.add_space(12.0);
    let bottom = ui.cursor().min.y;
    ui.painter().hline(
        row_top.x..=row_top.x + ui.available_width(),
        bottom + 0.5,
        Stroke::new(1.0, c.border),
    );
    ui.add_space(1.0);
    controls::switch_row(
        ui,
        "compact-rooms",
        "Kompakte Sprachkanäle",
        Some("Kleinere Avatare und weniger Abstand."),
        &mut p.compact_rooms,
        false,
    );
    controls::switch_row(
        ui,
        "reduced-motion",
        "Bewegung reduzieren",
        Some("Berücksichtigt auch die Einstellung deines Geräts."),
        &mut p.reduced_motion,
        false,
    );
    if p != before {
        prefs::update(ui.ctx(), |x| *x = p);
    }
}
