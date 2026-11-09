//! The pieces the settings pages are made of (settings.css, ui.css,
//! voice/quality.css): the page frame, groups with their scope tag, switch
//! rows, chips, radio cards, selects, sliders, text areas, the confirm
//! dialog and the image picker. Server settings use them too.

use std::sync::Arc;
use std::time::Duration;

use egui::text::LayoutJob;
use egui::{
    Color32, Frame, Id, Margin, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextBuffer,
    TextEdit, Ui, UiBuilder, Vec2, pos2, vec2,
};
use parking_lot::Mutex;

use crate::icons::{self, Icon};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{
    Badge, Checkbox, InlineStatus, Popover, StatusTone, Switch, eased, focus_ring, keyboard_focus,
    snap,
};

// ------------------------------------------------------------------ text

/// Paints one line of text with its line box's top-left at `pos`.
pub fn paint(ui: &Ui, pos: Pos2, s: &str, style: TextStyle, color: Color32) -> Block {
    let block = Block::single(ui, s, style, color);
    block.paint(ui.painter(), pos);
    block
}

/// Allocates a wrapped paragraph at the cursor, `width` wide.
pub fn paragraph(ui: &mut Ui, s: &str, style: TextStyle, color: Color32, width: f32) -> Rect {
    let block = Block::wrapped(ui, s, style, color, width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
    block.paint(ui.painter(), rect.min);
    rect
}

/// `.settings-hint`: 12px/1.5 muted.
pub const HINT: TextStyle = text::META.line_height(1.5);

/// The `.lr-channel-header h1`: 650 15px, letter-spacing −0.1px.
pub const HEADER: TextStyle = TextStyle::new(15.0, 650.0, 1.5).tracking(-0.1 / 15.0);

/// `--lr-type-title` with the area title's −0.01em.
const AREA_TITLE: TextStyle = text::TITLE.tracking(-0.01);

// ----------------------------------------------------------------- frame

/// `.lr-channel-header`: 52px incl. its bottom border, h1 at x+20, and an
/// optional muted context after it (" · Wohnzimmer").
pub fn header(ui: &Ui, page: Rect, title: &str, context: Option<&str>) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let painter = ui.painter();
    let bar = Rect::from_min_size(page.min, vec2(page.width(), 52.0));
    painter.rect_filled(bar, 0.0, c.bg);
    crate::widgets::rule_h(painter, c.border, page.x_range(), page.top() + 51.0);
    let top = page.top() + 14.25;
    let block = paint(ui, pos2(page.left() + 20.0, top), title, HEADER, c.text);
    if let Some(context) = context {
        let style = HEADER.weight(500.0).tracking(0.0);
        let x = page.left() + 20.0 + block.width();
        paint(ui, pos2(x, top), &format!(" · {context}"), style, c.muted);
    }
}

/// The scrolling body under the header. The app's scrollbar (style.rs),
/// floating over the content without a gutter of its own.
pub fn scroll<R>(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    offset: Option<f32>,
    content: impl FnOnce(&mut Ui) -> R,
) -> (R, f32) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect);
    child.spacing_mut().scroll.floating_allocated_width = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(Id::unique(("gelabber-settings-scroll", id)))
        .auto_shrink([false, false]);
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    let out = area.show(&mut child, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        content(ui)
    });
    (out.inner, out.state.clamped_offset().y)
}

/// A child Ui at `builder`'s rect that leaves the parent's cursor alone
/// (absolutely placed controls).
pub fn place<R>(ui: &mut Ui, builder: UiBuilder, content: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(builder);
    child.spacing_mut().item_spacing = Vec2::ZERO;
    content(&mut child)
}

/// A child column `width` wide at the cursor; the cursor moves past it.
pub fn column<R>(ui: &mut Ui, width: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, f32::INFINITY));
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing = Vec2::ZERO;
    let out = content(&mut child);
    let used = child.min_rect();
    ui.advance_cursor_after_rect(Rect::from_min_size(
        rect.min,
        vec2(width, used.height().max(0.0)),
    ));
    out
}

/// A row of items `gap` apart, vertically centred, `height` tall.
pub fn row<R>(ui: &mut Ui, gap: f32, height: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width();
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, height));
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing = vec2(gap, 0.0);
    let out = content(&mut child);
    ui.advance_cursor_after_rect(rect);
    out
}

// ---------------------------------------------------------------- scopes

/// Where a group of settings is kept (`settingsScope.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Account,
    Device,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Account => "Für dein Konto",
            Self::Device => "Nur dieses Gerät",
        }
    }
}

/// `ScopeTag`: a quiet neutral badge, the accent is for the page's actions.
pub fn scope_tag(ui: &mut Ui, scope: Scope) -> Response {
    ui.add(Badge::new(scope.label()))
}

/// `.settings-area-heading`: title (+ tag) and description, then 24px.
pub fn area_heading(ui: &mut Ui, title: &str, scope: Option<Scope>, description: &str) {
    let theme = Theme::of(ui.ctx());
    let top = ui.cursor().min;
    let block = paint(ui, top, title, AREA_TITLE, theme.c.text);
    if let Some(scope) = scope {
        let rect = Rect::from_min_size(
            pos2(top.x + block.width() + 12.0, top.y + 2.5),
            vec2(200.0, 20.0),
        );
        place(ui, UiBuilder::new().max_rect(rect), |ui| {
            scope_tag(ui, scope)
        });
    }
    ui.allocate_exact_size(vec2(ui.available_width(), 25.0 + 6.0), Sense::hover());
    // `max-width: 60ch`.
    paragraph(ui, description, text::BODY, theme.c.muted, 529.9);
    ui.add_space(24.0);
}

/// `.settings-group-legend`: 600 16px title + tag, then 8px.
pub fn group_legend(ui: &mut Ui, title: &str, scope: Option<Scope>) {
    let theme = Theme::of(ui.ctx());
    let top = ui.cursor().min;
    let block = paint(ui, top, title, text::TITLE_SM, theme.c.text);
    if let Some(scope) = scope {
        let rect = Rect::from_min_size(
            pos2(top.x + block.width() + 10.0, top.y + 0.4),
            vec2(200.0, 20.0),
        );
        place(ui, UiBuilder::new().max_rect(rect), |ui| {
            scope_tag(ui, scope)
        });
    }
    ui.allocate_exact_size(vec2(ui.available_width(), 20.8 + 8.0), Sense::hover());
}

/// `.settings-hint` at the cursor.
pub fn hint(ui: &mut Ui, s: &str) -> Rect {
    let theme = Theme::of(ui.ctx());
    let w = ui.available_width();
    paragraph(ui, s, HINT, theme.c.muted, w)
}

// ------------------------------------------------------------- switches

/// `Switch` with label and description as a `.settings-row`: 64px min,
/// padding 12, bottom hairline; the whole row toggles it.
pub fn switch_row(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    description: Option<&str>,
    on: &mut bool,
    disabled: bool,
) -> bool {
    switch_row_ruled(ui, id, label, description, on, disabled, None)
}

/// `.settings-row-noted`: a switch row with a note boxed under it (what
/// decides the switch), the hairline below both.
pub fn switch_row_noted(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    on: &mut bool,
    disabled: bool,
    note: &str,
) -> bool {
    // The note describes the switch for screen readers too
    // (`aria-describedby` on the web).
    let changed = switch_row_ruled(ui, id, label, None, on, disabled, Some(note));
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    // `.settings-row-note`: padding 10 12, radius md, surface, meta muted.
    let width = ui.available_width();
    let block = Block::wrapped(ui, note, HINT, c.muted, width - 24.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.height() + 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.surface);
    block.paint(ui.painter(), rect.min + vec2(12.0, 10.0));
    // Padding-bottom 12, then the row's hairline.
    let (gap, _) = ui.allocate_exact_size(vec2(width, 13.0), Sense::hover());
    crate::widgets::rule_h(ui.painter(), c.border, gap.x_range(), gap.bottom() - 1.0);
    changed
}

fn switch_row_ruled(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    description: Option<&str>,
    on: &mut bool,
    disabled: bool,
    // A noted row draws its hairline under the note, not here.
    note: Option<&str>,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let text_w = width - 38.0 - 18.0;
    let label_block = Block::wrapped(ui, label, text::UI, c.text, text_w);
    let desc_block =
        description.map(|d| Block::wrapped(ui, d, text::META.line_height(1.5), c.muted, text_w));
    let content_h = label_block.height() + desc_block.as_ref().map_or(0.0, |d| 4.0 + d.height());
    let height = (content_h + 24.0 + 1.0).max(64.0);
    // The row only forwards clicks; Tab reaches the switch in it.
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, height),
        if disabled {
            Sense::hover()
        } else {
            Sense::CLICK
        },
    );
    let id = Id::unique(("gelabber-switch-row", id));
    // The row is the switch's label, as `<label>` around the web's input.
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, !disabled, *on, label));
    describe(ui, response.id, note);
    let mut changed = false;
    if response.clicked() && !disabled {
        *on = !*on;
        changed = true;
    }
    let painter = ui.painter();
    let inner_top = rect.top() + (height - 1.0 - content_h) / 2.0;
    label_block.paint(painter, pos2(rect.left(), inner_top));
    if let Some(d) = &desc_block {
        d.paint(
            painter,
            pos2(rect.left(), inner_top + label_block.height() + 4.0),
        );
    }
    if note.is_none() {
        crate::widgets::rule_h(painter, c.border, rect.x_range(), rect.bottom() - 1.0);
    }
    let switch_rect = Rect::from_min_size(
        pos2(
            rect.right() - 38.0,
            rect.top() + (height - 1.0 - 22.0) / 2.0,
        ),
        vec2(38.0, 22.0),
    );
    let mut value = *on;
    let sw = place(
        ui,
        UiBuilder::new().max_rect(switch_rect).id_salt(id),
        |ui| ui.add(Switch::new(&mut value).disabled(disabled)),
    );
    // `<label>` of the switch input: the row's text names it.
    sw.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, !disabled, *on, label));
    describe(ui, sw.id, note);
    if sw.changed() && !changed {
        *on = value;
        changed = true;
    }
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    changed
}

/// The accessible description of the widget `id`, if there is one.
fn describe(ui: &Ui, id: Id, note: Option<&str>) {
    if let Some(note) = note {
        ui.ctx()
            .accesskit_node_builder(id, |node| node.set_description(note));
    }
}

// ----------------------------------------------------------------- chips

/// `ChipGroup`: pills `gap 8`; returns the newly chosen index. Arrow keys
/// move the choice while one has focus.
pub fn chips(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    options: &[&str],
    selected: usize,
    height: f32,
    color: Option<Color32>,
) -> Option<usize> {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let id = Id::unique(("gelabber-chips", id));
    let mut chosen = None;
    let start = ui.cursor().min;
    let mut x = start.x;
    let mut focused_any = None;
    for (i, label) in options.iter().enumerate() {
        let on = i == selected;
        let style = if on { text::UI.weight(600.0) } else { text::UI };
        let block = Block::single(ui, label, style, c.text);
        let w = block.width() + 24.0 + 2.0;
        let rect = Rect::from_min_size(pos2(x, start.y), vec2(w, height));
        // One Tab stop for the group, the chosen chip (`role=radiogroup`
        // with a roving tabIndex); the arrows move between them.
        let stop = on || ui.memory(|m| m.has_focus(id.with(i)));
        let sense = if stop { Sense::click() } else { Sense::CLICK };
        let response = ui.interact(rect, id.with(i), sense);
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *label));
        if response.clicked() {
            chosen = Some(i);
        }
        if response.has_focus() {
            focused_any = Some(i);
        }
        let fill = eased(
            ui.ctx(),
            id.with(("hover", i)),
            response.hovered() && !on,
            Color32::TRANSPARENT,
            c.hover,
        );
        let (fill, border) = if on {
            (c.selected, c.accent)
        } else {
            (fill, c.border_strong)
        };
        let painter = ui.painter();
        let shown = snap(ui.ctx(), rect);
        if response.has_focus() && keyboard_focus(ui.ctx()) {
            focus_ring(painter, shown, height / 2.0, &theme);
        }
        painter.rect(
            shown,
            corner(height / 2.0),
            fill,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        block.paint_tinted(
            painter,
            pos2(x + 13.0, rect.center().y - block.height() / 2.0),
            Some(color.unwrap_or(c.text)),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        x += w + 8.0;
    }
    if let Some(i) = focused_any {
        let (left, right) = ui.input(|inp| {
            (
                inp.key_pressed(egui::Key::ArrowLeft) || inp.key_pressed(egui::Key::ArrowUp),
                inp.key_pressed(egui::Key::ArrowRight) || inp.key_pressed(egui::Key::ArrowDown),
            )
        });
        let next = if left {
            Some((i + options.len() - 1) % options.len())
        } else if right {
            Some((i + 1) % options.len())
        } else {
            None
        };
        if let Some(n) = next {
            ui.memory_mut(|m| m.request_focus(id.with(n)));
            chosen = Some(n);
        }
    }
    ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    chosen.filter(|i| *i != selected)
}

// ------------------------------------------------------------ radio card

/// `RadioCard`: border-strong card, radio + label/description; checked
/// border accent on the selected fill.
pub fn radio_card(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    desc: &str,
    checked: bool,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let text_w = width - 2.0 - 24.0 - 20.0 - 12.0;
    let l = Block::wrapped(ui, label, text::UI, c.text, text_w);
    let d = Block::wrapped(ui, desc, text::META, c.muted, text_w);
    let h = 1.0 + 12.0 + l.height() + 2.0 + d.height() + 12.0 + 1.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, h.max(44.0)), Sense::click());
    let id = Id::unique(("gelabber-radio-card", id));
    let hover = eased(ui.ctx(), id, response.hovered(), c.panel, c.hover);
    let (fill, border) = if checked {
        (c.selected, c.accent)
    } else {
        (hover, c.border_strong)
    };
    let painter = ui.painter().clone();
    let shown = snap(ui.ctx(), rect);
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(&painter, shown, theme.radius.md, &theme);
    }
    painter.rect(
        shown,
        corner(theme.radius.md),
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    radio_mark(ui, pos2(rect.left() + 13.0, rect.top() + 14.0), checked);
    l.paint(&painter, pos2(rect.left() + 45.0, rect.top() + 13.0));
    d.paint(
        &painter,
        pos2(rect.left() + 45.0, rect.top() + 13.0 + l.height() + 2.0),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked() && !checked
}

/// A 20px `.ui-check` radio painted at `min` (no interaction of its own).
pub fn radio_mark(ui: &mut Ui, min: Pos2, checked: bool) {
    let rect = Rect::from_min_size(min, Vec2::splat(20.0));
    let mut value = checked;
    place(ui, UiBuilder::new().max_rect(rect), |ui| {
        ui.add(Checkbox::new(&mut value).radio().decorative());
    });
}

// ---------------------------------------------------------------- select

/// `ui/Select.tsx`: a 37px field with the value and a chevron; a click
/// opens the options under it.
/// Whether the [`select`] named `id` has its list open.
pub fn select_open(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::unique(("gelabber-select-open", id))))
        .unwrap_or(false)
}

pub fn select(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Clone,
    options: &[(String, String)],
    value: &mut String,
    fill: Color32,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 37.0), Sense::click());
    crate::widgets::popover::focus_first(&response);
    let open_id = Id::unique(("gelabber-select-open", id.clone()));
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    if response.clicked() {
        open = !open;
    }
    let painter = ui.painter();
    let shown = snap(ui.ctx(), rect);
    let focused = response.has_focus() && keyboard_focus(ui.ctx());
    if focused {
        focus_ring(painter, shown, theme.radius.sm, &theme);
    }
    painter.rect(
        shown,
        corner(theme.radius.sm),
        fill,
        Stroke::new(1.0, if open { c.focus } else { c.border_strong }),
        StrokeKind::Inside,
    );
    let label = options
        .iter()
        .find(|(k, _)| k == value)
        .or(options.first())
        .map(|(_, l)| l.as_str())
        .unwrap_or("");
    let block = Block::single(ui, label, text::BODY, c.text);
    let clip = Rect::from_min_max(rect.min, pos2(rect.right() - 34.0, rect.bottom()));
    block.paint(
        &painter.with_clip_rect(clip),
        pos2(rect.left() + 12.0, rect.top() + 8.0),
    );
    icons::paint(
        painter,
        Icon::Chevron,
        pos2(rect.right() - 11.0 - 14.0, rect.center().y - 7.0),
        14.0,
        c.muted,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut changed = false;
    if open {
        let shown = Popover::new(("select", id), rect)
            .min_width(width)
            .select_list()
            .show(ui.ctx(), |ui| {
                let mut pick = None;
                for (k, l) in options {
                    if option_row(ui, l, k == value).clicked() {
                        pick = Some(k.clone());
                    }
                }
                pick
            });
        match shown {
            Some(Some(k)) => {
                if *value != k {
                    *value = k;
                    changed = true;
                }
                open = false;
            }
            Some(None) => {}
            None => open = false,
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    changed
}

fn option_row(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let fill = if response.hovered() {
        c.hover
    } else if selected {
        c.selected
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, corner(theme.radius.sm), fill);
    let style = if selected {
        text::UI.weight(600.0)
    } else {
        text::UI
    };
    let block = Block::single(ui, label, style, c.text);
    block.paint(
        ui.painter(),
        pos2(rect.left() + 10.0, rect.center().y - block.height() / 2.0),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

// ---------------------------------------------------------------- slider

/// `.lr-range`: a 4px track filled with accent up to the value and a
/// 14px thumb with its halo; 20px tall. Arrow keys step it.
pub fn slider(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    disabled: bool,
) -> bool {
    range(ui, id, None, value, (min, max, step), disabled)
}

/// [`slider`] with an accessible name (`aria-label`).
pub fn slider_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    value: &mut f64,
    (min, max, step): (f64, f64, f64),
    disabled: bool,
) -> bool {
    range(ui, id, Some(label), value, (min, max, step), disabled)
}

fn range(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: Option<&str>,
    value: &mut f64,
    (min, max, step): (f64, f64, f64),
    disabled: bool,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, 20.0));
    let id = Id::unique(("gelabber-slider", id));
    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click_and_drag()
    };
    let response = ui.interact(rect, id, sense);
    crate::widgets::popover::focus_first(&response);
    ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let before = *value;
    // The thumb's centre runs from 7px to width-7px, as in Chromium.
    let span = (width - 14.0).max(1.0);
    if let Some(p) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked() || response.drag_started())
    {
        let t = ((p.x - rect.left() - 7.0) / span).clamp(0.0, 1.0) as f64;
        let raw = min + t * (max - min);
        *value = ((raw - min) / step).round() * step + min;
    }
    if response.has_focus() {
        let (l, r) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp),
            )
        });
        if l {
            *value -= step;
        }
        if r {
            *value += step;
        }
    }
    *value = value.clamp(min, max);
    if let Some(label) = label {
        let shown = *value;
        response.widget_info(|| egui::WidgetInfo::slider(!disabled, shown, label));
    }
    let t = ((*value - min) / (max - min)).clamp(0.0, 1.0) as f32;
    let alpha = if disabled { 0.45 } else { 1.0 };
    let painter = ui.painter();
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 2.0), vec2(width, 4.0));
    let rest = mix(ui_bg(ui), c.text, 0.18);
    painter.rect_filled(track, corner(2.0), rest.gamma_multiply(alpha));
    // The fill follows `--lr-range-value` of the whole input width.
    let fill = Rect::from_min_max(track.min, pos2(track.left() + width * t, track.bottom()));
    painter.rect_filled(fill, corner(2.0), c.accent.gamma_multiply(alpha));
    let cx = rect.left() + 7.0 + span * t;
    let center = pos2(cx, rect.center().y);
    painter.circle_filled(
        center,
        10.0,
        mix(ui_bg(ui), c.accent, 0.22).gamma_multiply(alpha),
    );
    painter.circle_filled(center, 7.0, c.accent.gamma_multiply(alpha));
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(
            painter,
            Rect::from_center_size(center, Vec2::splat(14.0)),
            7.0,
            &theme,
        );
    }
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (*value - before).abs() > f64::EPSILON
}

/// The surface a control sits on, for the translucent mixes.
fn ui_bg(ui: &Ui) -> Color32 {
    let id = Id::unique("gelabber-settings-surface");
    ui.data(|d| d.get_temp::<Color32>(id))
        .unwrap_or_else(|| Theme::of(ui.ctx()).c.bg)
}

// -------------------------------------------------------------- textarea

/// `Textarea` (`.ui-field`, rows=4 → 100px): a multi-line field.
pub fn textarea(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    value: &mut String,
    height: f32,
    invalid: bool,
) -> Response {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, height));
    let field = crate::widgets::field::field_id(id);
    let focused = ui.memory(|m| m.has_focus(field));
    let painter = ui.painter().clone();
    let shown = snap(ui.ctx(), rect);
    if focused {
        focus_ring(&painter, shown, theme.radius.sm, &theme);
    }
    painter.rect(
        shown,
        corner(theme.radius.sm),
        c.bg,
        Stroke::new(
            1.0,
            if invalid {
                c.danger
            } else if focused {
                c.focus
            } else {
                c.border_strong
            },
        ),
        StrokeKind::Inside,
    );
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, wrap: f32| {
        let mut job = LayoutJob::single_section(buf.as_str().to_owned(), text::BODY.format(color));
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let inner = rect.shrink(1.0);
    let edit = TextEdit::multiline(value)
        .id(field)
        .frame(Frame::NONE.inner_margin(Margin {
            left: 11,
            right: 11,
            top: 7,
            bottom: 7,
        }))
        .desired_width(inner.width())
        .min_size(inner.size())
        .layouter(&mut layouter);
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(field.with("scroll"))
                .max_height(inner.height())
                .show(ui, |ui| ui.add(edit))
                .inner
        })
        .inner
        .accessible_name(label);
    ui.advance_cursor_after_rect(rect);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    response
}

// --------------------------------------------------------------- status

/// `useInlineStatus`: success and neutral lines clear after 4 s.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub line: Option<(String, StatusTone, f64)>,
}

impl Status {
    pub fn show(&mut self, ui: &Ui, message: impl Into<String>, tone: StatusTone) {
        let now = ui.input(|i| i.time);
        self.line = Some((message.into(), tone, now));
    }

    pub fn success(&mut self, ui: &Ui, message: impl Into<String>) {
        self.show(ui, message, StatusTone::Success);
    }

    pub fn error(&mut self, ui: &Ui, message: impl Into<String>) {
        self.show(ui, message, StatusTone::Error);
    }

    pub fn clear(&mut self) {
        self.line = None;
    }

    /// The current line, dropping an expired one.
    pub fn current(&mut self, ui: &Ui) -> Option<(String, StatusTone)> {
        let now = ui.input(|i| i.time);
        if let Some((_, tone, at)) = &self.line
            && *tone != StatusTone::Error
        {
            let left = at + 4.0 - now;
            if left <= 0.0 {
                self.line = None;
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(left));
            }
        }
        self.line.as_ref().map(|(m, t, _)| (m.clone(), *t))
    }

    /// Paints the line at the cursor, `width` wide (nothing when empty).
    pub fn ui(&mut self, ui: &mut Ui, width: f32) {
        if let Some((m, t)) = self.current(ui) {
            column(ui, width, |ui| ui.add(InlineStatus::new(Some(&m), t)));
        }
    }
}

// --------------------------------------------------------------- confirm

/// `confirmAction`: one question at a time; the answer is read back by
/// its `key` on a later frame.
#[derive(Clone, Debug)]
pub struct Confirm {
    pub key: String,
    pub title: String,
    pub message: Option<String>,
    pub label: String,
    /// `tone: "neutral"`: a primary confirm button instead of danger.
    pub neutral: bool,
}

impl Confirm {
    pub fn danger(key: &str, title: impl Into<String>, message: Option<&str>, label: &str) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            message: message.map(str::to_owned),
            label: label.into(),
            neutral: false,
        }
    }
}

#[derive(Clone, Default)]
struct ConfirmSlot {
    open: Option<Confirm>,
    answered: Option<(String, bool)>,
}

fn confirm_slot(ctx: &egui::Context) -> Arc<Mutex<ConfirmSlot>> {
    let id = Id::unique("gelabber-settings-confirm");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(ConfirmSlot::default())))
            .clone()
    })
}

/// Asks; a new question replaces an open one (its answer is "no").
pub fn ask(ctx: &egui::Context, confirm: Confirm) {
    let slot = confirm_slot(ctx);
    let mut s = slot.lock();
    s.open = Some(confirm);
    s.answered = None;
}

/// The answer to the question asked with `key`, once, when there is one.
pub fn answer(ctx: &egui::Context, key: &str) -> Option<bool> {
    let slot = confirm_slot(ctx);
    let mut s = slot.lock();
    if s.answered.as_ref().is_some_and(|(k, _)| k == key) {
        return s.answered.take().map(|(_, a)| a);
    }
    None
}

/// Whether a question is open (pages pause their shortcuts meanwhile).
pub fn asking(ctx: &egui::Context) -> bool {
    confirm_slot(ctx).lock().open.is_some()
}

/// Draws the open question (ConfirmHost). Call once per frame.
pub fn show_confirm(ctx: &egui::Context) {
    let slot = confirm_slot(ctx);
    let Some(q) = slot.lock().open.clone() else {
        return;
    };
    // The workspace's ConfirmDialog: starts on "Abbrechen" unless neutral,
    // so Enter answers with the focused button.
    let result = crate::screens::shell::overlays::confirm::dialog(
        ctx,
        &q.title,
        q.message.as_deref(),
        &q.label,
        q.neutral,
        false,
    );
    if let Some(a) = result {
        let mut s = slot.lock();
        s.open = None;
        s.answered = Some((q.key, a));
    }
}

// ----------------------------------------------------------- image pick

/// A file chosen in the system dialog, read off the UI thread.
pub use crate::platform::files::Picked;

/// Opens the desktop's file chooser (platform/files.rs: the portal, zenity
/// or kdialog, else a path field in the window); the slot fills with the
/// bytes and the file name, or `None` when nothing was chosen.
pub fn pick_file(ctx: &egui::Context, title: &str, filter: &[&str]) -> Picked {
    let slot: Picked = Arc::new(Mutex::new(None));
    crate::platform::files::pick(ctx, title, filter, slot.clone());
    slot
}

/// Saves `bytes` where the user chooses (the web's download); without a
/// dialog program the file goes to the Downloads folder.
pub fn save_file(ctx: &egui::Context, title: &str, name: &str, bytes: Vec<u8>) {
    use std::process::Command;
    let (title, name) = (title.to_owned(), name.to_owned());
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let downloads = home.map(|h| h.join("Downloads")).filter(|d| d.is_dir());
        let suggested = downloads
            .clone()
            .map(|d| d.join(&name))
            .unwrap_or_else(|| std::path::PathBuf::from(&name));
        let suggested = suggested.to_string_lossy().into_owned();
        let programs: [(&str, Vec<String>); 2] = [
            (
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--save".into(),
                    "--confirm-overwrite".into(),
                    format!("--title={title}"),
                    format!("--filename={suggested}"),
                ],
            ),
            (
                "kdialog",
                vec![
                    "--title".into(),
                    title.clone(),
                    "--getsavefilename".into(),
                    suggested.clone(),
                ],
            ),
        ];
        let mut target = None;
        let mut ran = false;
        for (program, args) in programs {
            if let Ok(out) = Command::new(program).args(&args).output() {
                ran = true;
                if out.status.success() {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                    target = (!p.is_empty()).then_some(p);
                }
                break;
            }
        }
        if !ran {
            target = Some(suggested);
        }
        if let Some(path) = target {
            let _ = std::fs::write(path, bytes);
        }
        ctx.request_repaint();
    });
}

/// Takes a dropped file from the window, if one was dropped this frame.
pub fn dropped_file(ctx: &egui::Context) -> Option<(Vec<u8>, String)> {
    let file = ctx.input(|i| i.raw.dropped_files.first().cloned())?;
    let path = file.path().to_path_buf();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    file.bytes().ok().map(|b| (b, name))
}

/// `prepareAvatar` / `prepareBanner`: a centred crop to `w`×`h`, encoded
/// as JPEG (opaque) or PNG. Errors are the web's two messages.
pub fn prepare_image(bytes: &[u8], w: u32, h: u32) -> Result<(Vec<u8>, &'static str), String> {
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("Das Bild ist zu groß (höchstens 20 MB).".into());
    }
    let unreadable =
        || "Dieses Bild kann der Browser nicht lesen. PNG, JPEG, WebP oder GIF gehen.".to_owned();
    let img = image::load_from_memory(bytes).map_err(|_| unreadable())?;
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let ratio = f64::from(w) / f64::from(h);
    let cw = iw.min(ih * ratio);
    let ch = cw / ratio;
    let x = ((iw - cw) / 2.0).round() as u32;
    let y = ((ih - ch) / 2.0).round() as u32;
    let cropped = img.crop_imm(x, y, cw.round().max(1.0) as u32, ch.round().max(1.0) as u32);
    let resized = cropped
        .resize_exact(w, h, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let opaque = resized.pixels().all(|p| p[3] == 255);
    let mut out = std::io::Cursor::new(Vec::new());
    if opaque {
        let rgb = image::DynamicImage::ImageRgba8(resized).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .map_err(|_| unreadable())?;
        Ok((out.into_inner(), "image/jpeg"))
    } else {
        image::DynamicImage::ImageRgba8(resized)
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|_| unreadable())?;
        Ok((out.into_inner(), "image/png"))
    }
}

/// The image filters of the pickers (`accept=image/png,…`).
pub const IMAGE_FILTER: [&str; 6] = ["*.png", "*.jpg", "*.jpeg", "*.webp", "*.gif", "*.PNG"];

#[cfg(test)]
mod tests {
    use super::*;

    /// The note under a switch describes it, as `aria-describedby` does on
    /// the web; a plain switch row has no description.
    #[test]
    fn a_noted_switch_is_described_by_its_note() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            crate::fonts::install(ui.ctx());
            let (mut a, mut b) = (false, false);
            switch_row_noted(ui, "noted", "Mit Hinweis", &mut a, false, "Blockiert.");
            switch_row(ui, "plain", "Ohne Hinweis", None, &mut b, false);
        });
        harness.run();
        let switches = |label: &'static str| {
            harness
                .query_all(By::new().role(egui::accesskit::Role::Switch).label(label))
                .map(|n| n.accesskit_node().description())
                .collect::<Vec<_>>()
        };
        let noted = switches("Mit Hinweis");
        assert!(!noted.is_empty());
        assert!(noted.iter().all(|d| d.as_deref() == Some("Blockiert.")));
        let plain = switches("Ohne Hinweis");
        assert!(!plain.is_empty());
        assert!(plain.iter().all(Option::is_none));
    }

    #[test]
    fn crops_to_the_target_ratio() {
        let img = image::RgbaImage::from_pixel(300, 100, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let (out, kind) = prepare_image(&bytes.into_inner(), 256, 256).unwrap();
        assert_eq!(kind, "image/jpeg");
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (256, 256));
        assert!(prepare_image(b"nope", 10, 10).is_err());
    }
}
