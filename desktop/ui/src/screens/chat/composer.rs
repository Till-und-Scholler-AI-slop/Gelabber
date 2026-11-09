//! The composer (components/Composer.tsx, chat.css `.lr-composer`): one
//! field holding attach, the one-line draft, emoji and send; the reply
//! bar, files to attach and the `@…` suggestions above the row.

use std::collections::HashMap;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, TextBuffer, Ui, Vec2, pos2, vec2};
use gelabber_client::OutgoingFile;
use uuid::Uuid;

use super::attachments::format_size;
use super::files;
use super::mentions::{self, Member, Suggestion};
use crate::icons::{self, Icon};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::tooltip::{Side, Tooltip};

/// `CONTENT_MAX` (messages/rules.ts).
const CONTENT_MAX: i64 = 2000;
const DRAFT: TextStyle = TextStyle::new(14.5, 400.0, 22.0 / 14.5);
const ROW: f32 = 5.0 + 34.0 + 5.0;
const SUGGESTION: f32 = 33.0;

#[derive(Default)]
pub struct ComposerState {
    pub draft: String,
    /// Replying to (message, author name).
    pub reply: Option<(Uuid, String)>,
    pub files: Vec<OutgoingFile>,
    /// The files left out last, and why (`fileError`).
    pub file_error: Option<String>,
    pub emoji_open: bool,
    pub gif_open: bool,
    active: usize,
    dismissed: Option<(usize, String)>,
    pub picked: HashMap<String, Uuid>,
    pub focus: bool,
    /// Files chosen in the system's file dialog, once it closes.
    chosen: Option<crate::platform::files::PickedMany>,
    /// Files being read (or a pasted picture encoded) off the UI thread,
    /// in the order they came.
    loading: Vec<files::Loading>,
    /// Rows of attached-file chips the last frame drew.
    chip_rows: usize,
    /// Where the caret goes once the field shows the new draft.
    caret: Option<usize>,
    last_caret: Option<usize>,
}

impl ComposerState {
    /// Inserts `text` at the caret (emoji picker).
    pub fn insert(&mut self, text: &str) {
        let at = self
            .last_caret
            .unwrap_or(self.draft.chars().count())
            .min(self.draft.chars().count());
        let byte = self
            .draft
            .char_indices()
            .nth(at)
            .map_or(self.draft.len(), |(b, _)| b);
        self.draft.insert_str(byte, text);
        self.caret = Some(at + text.chars().count());
        self.focus = true;
    }

    fn mention(&self) -> Option<(usize, String)> {
        let caret = self.last_caret?;
        let at = mentions::mention_at(&self.draft, caret)?;
        (self.dismissed.as_ref() != Some(&at)).then_some(at)
    }
}

pub struct ComposerInput<'a> {
    pub channel: Uuid,
    /// `#allgemein` / `@Jonas` for the placeholder.
    pub placeholder: String,
    pub can_send: bool,
    pub can_send_files: bool,
    pub gifs: bool,
    pub members: &'a [Member],
    pub me: Option<Uuid>,
    pub keywords: bool,
    /// Unsent attempts (failed rows).
    pub unsent: usize,
}

/// What the composer asks for.
pub enum ComposerAction {
    Send {
        text: String,
        reply: Option<Uuid>,
        files: Vec<OutgoingFile>,
    },
    Typing(String),
    StopTyping,
    EditLast,
    ShowUnsent,
}

fn suggestions_for(state: &ComposerState, input: &ComposerInput) -> Vec<Suggestion> {
    match state.mention() {
        Some((_, query)) => mentions::suggestions(&query, input.members, input.me, input.keywords),
        None => Vec::new(),
    }
}

/// Files hover the window while they are dragged over it: the pane says
/// where they go ("Loslassen zum Anhängen").
pub fn dropping(ctx: &egui::Context, input: &ComposerInput) -> bool {
    input.can_send && input.can_send_files && ctx.input(|i| !i.raw.hovered_files.is_empty())
}

const NOTE: TextStyle = TextStyle::new(12.0, 400.0, 1.5);

/// The file note's lines (`.lr-composer-file-error`), at `width`.
fn file_error_height(ctx: &egui::Context, state: &ComposerState, width: f32) -> f32 {
    let Some(error) = &state.file_error else {
        return 0.0;
    };
    let galley = ctx.fonts_mut(|f| {
        f.layout_job(crate::text::job(
            error,
            NOTE,
            Color32::WHITE,
            Some(width - 32.0 - 12.0),
        ))
    });
    4.0 + galley.rows.len() as f32 * NOTE.line_px()
}

/// The composer's height in a pane `width` wide (the list area gives way
/// to it).
pub fn height(
    ctx: &egui::Context,
    state: &ComposerState,
    input: &ComposerInput,
    width: f32,
) -> f32 {
    if !input.can_send {
        return 14.0 + 43.5;
    }
    let mut field = 2.0 + ROW;
    if dropping(ctx, input) {
        field += 10.0 + 19.5;
    }
    if state.reply.is_some() {
        field += 8.0 + 40.0;
    }
    if !state.files.is_empty() {
        field += 10.0 + chips_height(state.chip_rows);
    }
    let n = suggestions_for(state, input).len();
    if n > 0 {
        field += (8.0 + n as f32 * SUGGESTION).min(240.0) + 1.0;
    }
    let mut h = field + 14.0;
    if input.unsent > 0 {
        h += 8.0 + 30.0;
    }
    if remaining(state, input) < 200 {
        h += 4.0 + 18.0;
    }
    h + file_error_height(ctx, state, width)
}

/// `validateContent` of a message's wire text: the field code, or `None`.
pub fn validate_content(wire: &str) -> Option<&'static str> {
    let value = wire.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.is_empty() {
        Some("required")
    } else if value.chars().count() as i64 > CONTENT_MAX {
        Some("too_long")
    } else if value
        .chars()
        .any(|c| matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}'))
    {
        Some("invalid")
    } else {
        None
    }
}

fn remaining(state: &ComposerState, input: &ComposerInput) -> i64 {
    let wire = mentions::to_wire(&state.draft, input.members, &state.picked);
    CONTENT_MAX - wire.replace("\r\n", "\n").chars().count() as i64
}

/// Draws the composer at the bottom of `pane` (its `height()` tall).
pub fn show(
    ui: &mut Ui,
    pane: Rect,
    state: &mut ComposerState,
    input: &ComposerInput,
) -> Vec<ComposerAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut actions = Vec::new();
    let h = height(ui.ctx(), state, input, pane.width());
    let area = Rect::from_min_max(pos2(pane.left(), pane.bottom() - h), pane.max);
    ui.painter().rect_filled(area, 0.0, c.bg);
    if !input.can_send {
        let r = Rect::from_min_max(
            pos2(area.left() + 16.0, area.top()),
            pos2(area.right() - 16.0, area.bottom() - 14.0),
        );
        ui.painter().rect_filled(r, corner(7.0), c.panel);
        crate::toasts::set_anchor(ui.ctx(), r);
        Block::single(
            ui,
            "Du kannst in diesem Kanal nicht schreiben.",
            TextStyle::new(13.0, 400.0, 1.5),
            c.muted,
        )
        .paint(ui.painter(), pos2(r.left() + 14.0, r.top() + 12.0));
        return actions;
    }
    let left = area.left() + 16.0;
    let right = area.right() - 16.0;
    let mut y = area.top();
    if input.unsent > 0 {
        let r = Rect::from_min_max(pos2(left, y), pos2(right, y + 30.0));
        ui.painter()
            .rect_filled(r, corner(5.0), mix(c.panel, c.danger_bg, 0.1));
        ui.painter().rect_filled(
            Rect::from_min_size(r.min, vec2(3.0, r.height())),
            0.0,
            c.danger_bg,
        );
        icons::paint(
            ui.painter(),
            Icon::Warning,
            pos2(r.left() + 10.0, r.center().y - 7.0),
            14.0,
            c.danger,
        );
        let label = if input.unsent == 1 {
            "1 Nachricht nicht gesendet".to_owned()
        } else {
            format!("{} Nachrichten nicht gesendet", input.unsent)
        };
        let style = TextStyle::new(13.0, 500.0, 1.35);
        Block::single(ui, &label, style, c.text)
            .paint(ui.painter(), pos2(r.left() + 32.0, r.top() + 6.0));
        let show = Block::single(ui, "Anzeigen", style.weight(600.0), c.text);
        let sr = Rect::from_min_size(
            pos2(r.right() - 10.0 - show.width(), r.top() + 6.0),
            show.size(),
        );
        show.paint(ui.painter(), sr.min);
        crate::widgets::link::underline(
            ui.painter(),
            style,
            sr.top(),
            sr.left(),
            sr.right(),
            c.text,
        );
        if ui
            .interact(
                sr,
                egui::Id::unique(("gelabber-unsent", input.channel)),
                Sense::click(),
            )
            .clicked()
        {
            actions.push(ComposerAction::ShowUnsent);
        }
        y += 38.0;
    }
    let status_h = if remaining(state, input) < 200 {
        22.0
    } else {
        0.0
    };
    let file_h = file_error_height(ui.ctx(), state, pane.width());
    let field = Rect::from_min_max(
        pos2(left, y),
        pos2(right, area.bottom() - 14.0 - status_h - file_h),
    );
    let drop_target = dropping(ui.ctx(), input);
    let edit_id = egui::Id::unique(("gelabber-composer", input.channel));
    let focused = ui.memory(|m| m.has_focus(edit_id));
    // Esc drops egui's focus before the frame runs; it was ours.
    let had_focus = focused || ui.memory(|m| m.had_focus_last_frame(edit_id));
    let border = if drop_target {
        c.accent
    } else if focused {
        mix(c.border, c.accent, 0.7)
    } else {
        c.border_strong
    };
    let border = crate::widgets::eased(
        ui.ctx(),
        edit_id.with("border"),
        focused || drop_target,
        c.border_strong,
        border,
    );
    ui.painter().rect(
        field,
        corner(11.0),
        c.panel,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    // `anchor-name: --lr-composer` (the whole composer, the pane wide): the
    // toasts sit above it, 16px in from the pane's edge.
    crate::toasts::set_anchor(
        ui.ctx(),
        Rect::from_min_max(pos2(area.left(), field.top()), area.max),
    );
    let mut fy = field.top() + 1.0;
    if drop_target {
        let note = Block::single(
            ui,
            "Loslassen zum Anhängen",
            TextStyle::new(13.0, 600.0, 1.5),
            c.accent_text,
        );
        let at = pos2(field.left() + 1.0 + 14.0, fy + 10.0);
        note.paint(ui.painter(), at);
        // Tests find it; the web hides it from screen readers.
        ui.interact(
            Rect::from_min_size(at, note.size()),
            edit_id.with("drop"),
            Sense::hover(),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Label, true, "Loslassen zum Anhängen")
        });
        fy += 10.0 + 19.5;
    }

    // The reply bar.
    if let Some((_, name)) = state.reply.clone() {
        let r = Rect::from_min_max(
            pos2(field.left() + 11.0, fy + 8.0),
            pos2(field.right() - 11.0, fy + 48.0),
        );
        ui.painter().rect_filled(r, corner(7.0), c.surface);
        icons::paint(
            ui.painter(),
            Icon::Reply,
            pos2(r.left() + 10.0, r.center().y - 7.0),
            14.0,
            c.muted,
        );
        let style = TextStyle::new(13.0, 400.0, 1.5);
        let text = crate::text::Rich::new(style)
            .push("Antwort an ", style, c.muted)
            .push(&name, style.weight(600.0), c.text)
            .push(" · Esc bricht ab", style, c.muted)
            .layout(ui);
        text.paint(
            ui.painter(),
            pos2(r.left() + 32.0, r.center().y - style.line_px() / 2.0),
        );
        let close = Rect::from_min_size(
            pos2(r.right() - 8.0 - 28.0, r.center().y - 14.0),
            Vec2::splat(28.0),
        );
        let resp = ui.interact(close, edit_id.with("reply-close"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(close, corner(5.0), c.hover);
        }
        icons::paint_centered(
            ui.painter(),
            Icon::Close,
            close,
            14.0,
            if resp.hovered() { c.text } else { c.muted },
        );
        Tooltip::new("Antworten abbrechen").on_hover(&resp, Side::Above);
        if resp.clicked() {
            state.reply = None;
            state.focus = true;
        }
        fy += 48.0;
    }
    // Files to attach, in rows that wrap.
    if !state.files.is_empty() {
        let mut x = field.left() + 11.0;
        let mut top = fy + 10.0;
        let mut rows = 1;
        let mut remove = None;
        for (i, file) in state.files.iter().enumerate() {
            let name = Block::single(ui, &file.filename, TextStyle::new(13.0, 600.0, 1.5), c.text);
            let size = Block::single(
                ui,
                &format_size(file.data.len() as i64),
                TextStyle::new(11.0, 400.0, 1.5),
                c.muted,
            );
            let w = (8.0 + name.width().max(size.width()) + 10.0 + 4.0 + 26.0 + 12.0)
                .min(260.0)
                .min(field.width() - 22.0);
            if x > field.left() + 11.0 && x + w > field.right() - 11.0 {
                x = field.left() + 11.0;
                top += 56.0 + 8.0;
                rows += 1;
            }
            let r = Rect::from_min_size(pos2(x, top), vec2(w, 56.0));
            ui.painter().rect_filled(r, corner(7.0), c.surface);
            let clip = ui.painter().with_clip_rect(r);
            name.paint(&clip, pos2(r.left() + 8.0, r.top() + 8.0));
            size.paint(&clip, pos2(r.left() + 8.0, r.top() + 8.0 + 19.5));
            let close = Rect::from_min_size(
                pos2(r.right() - 12.0 - 26.0, r.center().y - 13.0),
                Vec2::splat(26.0),
            );
            let resp = ui.interact(close, edit_id.with(("remove", i)), Sense::click());
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "Anhang entfernen")
            });
            if resp.hovered() {
                ui.painter().circle_filled(close.center(), 13.0, c.hover);
            }
            icons::paint_centered(
                ui.painter(),
                Icon::Close,
                close,
                14.0,
                if resp.hovered() { c.text } else { c.muted },
            );
            Tooltip::new("Anhang entfernen").on_hover(&resp, Side::Above);
            if resp.clicked() {
                remove = Some(i);
            }
            x += w + 8.0;
        }
        if let Some(i) = remove {
            state.files.remove(i);
            state.file_error = None;
        }
        if rows != state.chip_rows {
            // `height()` laid out the last frame's rows.
            state.chip_rows = rows;
            ui.ctx().request_repaint();
        }
        fy += 10.0 + chips_height(rows);
    }
    // Mention suggestions, inside the field above the row.
    let list = suggestions_for(state, input);
    if !list.is_empty() {
        let list_h = (8.0 + list.len() as f32 * SUGGESTION).min(240.0);
        let current = state.active.min(list.len() - 1);
        let mut pick = None;
        for (i, s) in list.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(field.left() + 5.0, fy + 4.0 + i as f32 * SUGGESTION),
                vec2(field.width() - 10.0, SUGGESTION),
            );
            let resp = ui.interact(r, edit_id.with(("mention", i)), Sense::click());
            if resp.hovered() {
                state.active = i;
            }
            if i == current {
                ui.painter().rect_filled(r, corner(5.0), c.hover);
            }
            let style = TextStyle::new(14.0, 400.0, 1.5);
            let label = Block::single(ui, &format!("@{}", s.word()), style, c.text);
            label.paint(ui.painter(), pos2(r.left() + 10.0, r.top() + 6.0));
            let note = match s {
                Suggestion::Everyone => Some("Alle Mitglieder"),
                Suggestion::Here => Some("Alle, die gerade online sind"),
                Suggestion::User(_) => None,
            };
            if let Some(note) = note {
                Block::single(ui, note, TextStyle::new(12.0, 400.0, 1.5), c.muted).paint(
                    ui.painter(),
                    pos2(r.left() + 10.0 + label.width() + 8.0, r.top() + 6.0 + 2.0),
                );
            }
            if resp.clicked() {
                pick = Some(s.clone());
            }
        }
        fy += list_h;
        crate::widgets::rule_h(
            ui.painter(),
            c.border,
            field.left() + 1.0..=field.right() - 1.0,
            fy,
        );
        fy += 1.0;
        // Keys belong to the list while it is open.
        if had_focus {
            let (down, up, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    super::consume_plain(i, egui::Key::Enter)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                )
            });
            if down {
                state.active = (current + 1) % list.len();
            }
            if up {
                state.active = (current + list.len() - 1) % list.len();
            }
            if enter {
                pick = Some(list[current].clone());
            }
            if esc {
                state.dismissed = state.mention();
                state.focus = true;
            }
        }
        if let Some(s) = pick
            && let Some((start, query)) = state.mention()
        {
            let (text, caret) = mentions::insert_mention(&state.draft, start, &query, &s);
            if let Suggestion::User(m) = &s {
                state.picked.insert(m.name.clone(), m.id);
            }
            state.draft = text;
            state.caret = Some(caret);
            state.active = 0;
            state.focus = true;
            actions.push(ComposerAction::Typing(state.draft.clone()));
        }
    }

    // The row: attach, the draft, emoji, GIF, send.
    let row = Rect::from_min_size(
        pos2(field.left() + 1.0 + 5.0, fy + 5.0),
        vec2(field.width() - 12.0, 34.0),
    );
    let mut x = row.left();
    if input.can_send_files {
        let r = Rect::from_min_size(pos2(x, row.top()), Vec2::splat(34.0));
        if composer_icon(
            ui,
            r,
            Icon::Paperclip,
            "Datei anhängen",
            edit_id.with("attach"),
        ) && state.chosen.is_none()
        {
            state.chosen = Some(crate::platform::files::pick_many(
                ui.ctx(),
                "Datei anhängen",
            ));
        }
        x += 36.0;
    }
    let mut rx = row.right();
    let send_rect = Rect::from_min_size(pos2(rx - 34.0, row.top()), Vec2::splat(34.0));
    rx -= 36.0;
    let gif_rect = input.gifs.then(|| {
        let r = Rect::from_min_size(pos2(rx - 34.0, row.top()), Vec2::splat(34.0));
        rx -= 36.0;
        r
    });
    let emoji_rect = Rect::from_min_size(pos2(rx - 34.0, row.top()), Vec2::splat(34.0));
    rx -= 36.0;
    let text_rect = Rect::from_min_max(pos2(x, row.top()), pos2(rx, row.bottom()));

    // Keys before the field sees them: Enter sends, ↑ edits my last one,
    // Esc drops the reply.
    let wire = mentions::to_wire(&state.draft, input.members, &state.picked);
    let over = remaining(state, input) < 0;
    let disabled = over || (state.draft.trim().is_empty() && state.files.is_empty());
    let mut submit = false;
    if focused {
        if ui.input_mut(|i| super::consume_plain(i, egui::Key::Enter)) {
            submit = true;
        }
        if state.draft.is_empty() && ui.input_mut(|i| super::consume_plain(i, egui::Key::ArrowUp)) {
            actions.push(ComposerAction::EditLast);
        }
    }
    if had_focus
        && state.reply.is_some()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        state.reply = None;
        state.focus = true;
    }
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, wrap: f32| {
        let mut job = crate::text::job(buf.as_str(), DRAFT, color, Some(wrap));
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let before = state.draft.clone();
    let output = ui
        .scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(edit_id.with("scroll"))
                .max_height(34.0)
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    egui::TextEdit::multiline(&mut state.draft)
                        .id(edit_id)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                            left: 6,
                            right: 6,
                            top: 6,
                            bottom: 6,
                        }))
                        .desired_width(text_rect.width())
                        .desired_rows(1)
                        .min_size(vec2(text_rect.width(), 34.0))
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        ))
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner
        })
        .inner;
    // The web's textarea is labelled "Nachricht an #channel".
    let _ = output
        .response
        .response
        .clone()
        .accessible_name(format!("Nachricht an {}", input.placeholder));
    if let Some(range) = output.cursor_range {
        let caret = (range.primary.index == range.secondary.index).then_some(range.primary.index.0);
        if caret != state.last_caret {
            state.active = 0;
        }
        state.last_caret = caret;
    } else if !output.response.has_focus() {
        state.last_caret = None;
    }
    if let Some(caret) = state.caret.take() {
        let mut s = output.state.clone();
        let cc = egui::text::CCursor::new(caret);
        s.cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cc)));
        s.store(ui.ctx(), edit_id);
        state.last_caret = Some(caret);
    }
    if std::mem::take(&mut state.focus) {
        ui.memory_mut(|m| m.request_focus(edit_id));
    }
    if state.draft != before {
        actions.push(ComposerAction::Typing(state.draft.clone()));
    }
    if output.response.lost_focus() {
        actions.push(ComposerAction::StopTyping);
    }
    if state.draft.is_empty() {
        let hint = Block::single(
            ui,
            &format!("Nachricht an {}", input.placeholder),
            DRAFT,
            c.subtle,
        );
        let clip = ui.painter().with_clip_rect(text_rect);
        hint.paint(&clip, pos2(text_rect.left() + 6.0, text_rect.top() + 6.0));
    }
    if output.response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }

    if composer_icon(
        ui,
        emoji_rect,
        Icon::Smile,
        "Emoji einfügen",
        edit_id.with("emoji"),
    ) {
        state.emoji_open = true;
    }
    if let Some(r) = gif_rect {
        let resp = ui.interact(r, edit_id.with("gif"), Sense::click());
        let lit = resp.hovered();
        if lit {
            ui.painter().rect_filled(r, corner(7.0), c.hover);
        }
        let label = Block::single(
            ui,
            "GIF",
            TextStyle::new(11.0, 800.0, 1.5).tracking(0.04),
            if lit { c.text } else { c.muted },
        );
        label.paint(ui.painter(), r.center() - label.size() / 2.0);
        Tooltip::new("GIF senden").on_hover(&resp, Side::Above);
        if resp.clicked() {
            state.gif_open = true;
        }
    }
    // Send: accent, or quiet while there is nothing to send.
    let resp = ui.interact(
        send_rect,
        edit_id.with("send"),
        if disabled {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    if disabled {
        icons::paint_centered(
            ui.painter(),
            Icon::Send,
            send_rect,
            17.0,
            c.muted.gamma_multiply(0.55),
        );
    } else {
        let pressed = resp.is_pointer_button_down_on();
        let fill = if resp.hovered() {
            crate::theme::brightness(c.accent, 1.08)
        } else {
            c.accent
        };
        let r = if pressed {
            Rect::from_center_size(send_rect.center(), send_rect.size() * 0.97)
        } else {
            send_rect
        };
        ui.painter().rect_filled(r, corner(7.0), fill);
        icons::paint_centered(ui.painter(), Icon::Send, r, 17.0, c.accent_ink);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    Tooltip::new("Senden").on_hover(&resp, Side::Above);
    if resp.clicked() {
        submit = true;
    }
    if submit && !disabled {
        actions.push(ComposerAction::Send {
            text: wire,
            reply: state.reply.take().map(|(id, _)| id),
            files: std::mem::take(&mut state.files),
        });
        state.draft.clear();
        state.file_error = None;
        state.picked.clear();
        state.focus = true;
        actions.push(ComposerAction::StopTyping);
    }
    // "{n} Zeichen übrig" near the limit.
    if status_h > 0.0 {
        let left_chars = remaining(state, input);
        let (text, color) = if left_chars < 0 {
            (format!("{} Zeichen zu viel", -left_chars), c.danger)
        } else {
            (format!("{left_chars} Zeichen übrig"), c.muted)
        };
        let block = Block::single(ui, &text, TextStyle::new(12.0, 400.0, 1.5), color);
        block.paint(
            ui.painter(),
            pos2(right - 6.0 - block.width(), field.bottom() + 4.0),
        );
    }
    // The files left out, under the status line (`role="alert"`).
    if let Some(error) = &state.file_error {
        let block = Block::wrapped(ui, error, NOTE, c.danger, right - left - 12.0);
        let at = pos2(left + 6.0, field.bottom() + status_h + 4.0);
        block.paint(ui.painter(), at);
        ui.interact(
            Rect::from_min_size(at, block.size()),
            edit_id.with("file-error"),
            Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, error));
    }
    let mut sources = Vec::new();
    // Files picked with the paperclip.
    let picked = state.chosen.as_ref().and_then(|slot| slot.lock().take());
    if let Some(paths) = picked {
        state.chosen = None;
        sources.extend(paths.into_iter().map(files::Source::Path));
        state.focus = true;
    }
    if input.can_send_files {
        // Files dropped on the window attach like the paperclip.
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            sources.push(files::Source::Path(file.path().to_path_buf()));
            state.focus = true;
        }
        // A picture pasted into the draft (`pastedFiles`).
        if had_focus {
            ui.input(|i| {
                for e in &i.events {
                    if let egui::Event::PasteImage(image) = e {
                        sources.push(files::Source::Pasted(image.clone()));
                    }
                }
            });
        }
    }
    // Read (and a picture encoded) on a worker: a large file must not stop
    // the window, and one over the limit is not read at all.
    if !sources.is_empty() {
        state
            .loading
            .push(files::load(ui.ctx(), sources, chrono::Utc::now()));
    }
    while let Some(done) = state.loading.first().and_then(|l| l.lock().take()) {
        state.loading.remove(0);
        let (next, mut notes) = done;
        if let Some(note) = files::add_files(&mut state.files, next) {
            notes.push(note);
        }
        state.file_error = (!notes.is_empty()).then(|| notes.join(" "));
    }
    actions
}

/// The chips' height in `rows` rows, 8 apart.
fn chips_height(rows: usize) -> f32 {
    let rows = rows.max(1) as f32;
    rows * 56.0 + (rows - 1.0) * 8.0
}

fn composer_icon(ui: &mut Ui, rect: Rect, icon: Icon, label: &str, id: egui::Id) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = crate::widgets::eased(
        ui.ctx(),
        id.with("fill"),
        resp.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    ui.painter().rect_filled(rect, corner(7.0), fill);
    icons::paint_centered(
        ui.painter(),
        icon,
        rect,
        18.0,
        if resp.hovered() { c.text } else { c.muted },
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    Tooltip::new(label).on_hover(&resp, Side::Above);
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;

    fn composer<'a>(
        state: &'a std::cell::RefCell<ComposerState>,
        members: &'a [Member],
    ) -> egui_kittest::Harness<'a> {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .build_ui(|ui| {
                crate::fonts::install(ui.ctx());
                crate::theme::set(ui.ctx(), crate::theme::Theme::dark());
                let input = ComposerInput {
                    channel: Uuid::nil(),
                    placeholder: "#allgemein".into(),
                    can_send: true,
                    can_send_files: true,
                    gifs: false,
                    members,
                    me: None,
                    keywords: false,
                    unsent: 0,
                };
                let pane = ui.max_rect();
                let _ = show(ui, pane, &mut state.borrow_mut(), &input);
            })
    }

    /// Steps until the files being read are in.
    fn loaded(harness: &mut egui_kittest::Harness<'_>, state: &std::cell::RefCell<ComposerState>) {
        for _ in 0..300 {
            harness.step();
            if state.borrow().loading.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.run_steps(2);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gelabber-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            panic!("read on the UI thread")
        }
    }

    /// Files dragged over the window: the composer says where they go;
    /// dropped, the ones the API refuses are named and left out.
    #[test]
    fn files_over_the_window_show_where_they_go() {
        let members: Vec<Member> = Vec::new();
        let state = std::cell::RefCell::new(ComposerState::default());
        let mut harness = composer(&state, &members);
        harness.run_steps(2);
        assert!(harness.query_by_label("Loslassen zum Anhängen").is_none());
        harness.input_mut().hovered_files.push(egui::HoveredFile {
            path: Some("liste.txt".into()),
            mime: String::new(),
        });
        harness.run_steps(2);
        harness.get_by_label("Loslassen zum Anhängen");
        harness.input_mut().hovered_files.clear();
        let dir = temp_dir("drop");
        for (name, bytes) in [
            ("liste.txt", b"Milch".to_vec()),
            ("seite.html", b"<p>".to_vec()),
        ] {
            std::fs::write(dir.join(name), bytes).expect("write");
            harness
                .input_mut()
                .dropped_files
                .push(std::sync::Arc::new(Dropped(dir.join(name))));
        }
        harness.step();
        loaded(&mut harness, &state);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(harness.query_by_label("Loslassen zum Anhängen").is_none());
        harness.get_by_label("seite.html: Dieser Dateityp ist nicht erlaubt.");
        let state = state.borrow();
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].content_type, "text/plain");
    }

    /// A file over the limit is named by its size on disk; ten chips wrap
    /// inside the field, each with its remove button.
    #[test]
    fn dropped_files_are_checked_before_reading_and_their_chips_wrap() {
        let members: Vec<Member> = Vec::new();
        let state = std::cell::RefCell::new(ComposerState::default());
        let mut harness = composer(&state, &members);
        harness.run_steps(2);
        let dir = temp_dir("chips");
        // Sparse: 26 MB on paper, nothing on the disk.
        let big = std::fs::File::create(dir.join("film.mp4")).expect("file");
        big.set_len(26 * 1024 * 1024).expect("size");
        let mut names = vec!["film.mp4".to_owned()];
        for i in 0..10 {
            let name = format!("Urlaubsbericht-Teil-{i}.txt");
            std::fs::write(dir.join(&name), b"Sonne").expect("write");
            names.push(name);
        }
        for name in &names {
            harness
                .input_mut()
                .dropped_files
                .push(std::sync::Arc::new(Dropped(dir.join(name))));
        }
        harness.step();
        loaded(&mut harness, &state);
        let _ = std::fs::remove_dir_all(&dir);
        harness.get_by_label("film.mp4: Die Datei ist zu groß (max. 25 MB).");
        assert_eq!(state.borrow().files.len(), 10);
        let buttons: Vec<egui::Rect> = harness
            .query_all_by_label("Anhang entfernen")
            .map(|n| n.rect())
            .collect();
        assert_eq!(buttons.len(), 10);
        assert!(
            buttons.iter().all(|r| r.right() <= 800.0 - 16.0),
            "{buttons:?}"
        );
        assert!(state.borrow().chip_rows > 1);
    }
}
