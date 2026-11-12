//! Einstellungen → Feedback (feedback/FeedbackArea.tsx, FeedbackThread.tsx,
//! thread.ts, feedback.css; #542): every entry the person sent, as small
//! threads with the operators' replies, and "Feedback geben" above them.
//! Entries with unread replies are marked read once shown; their "Neu"
//! marks stay until the person leaves the area.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Local, Timelike, Utc};
use egui::{Color32, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{
    ApiError, FEEDBACK_TEXT_MAX, FeedbackAuthor, FeedbackClient, FeedbackEntry,
    FeedbackEntryContext, FeedbackKind, FeedbackMessage, FeedbackPage, FeedbackState,
};
use gelabber_i18n::{Locale, common, feedback as texts, intl, language};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::images::{Fit, ImageState};
use crate::rules::{Failure, FormErrors, field_message};
use crate::screens::feedback::Answer;
use crate::screens::settings::controls::{self, TextareaExtras};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, space};
use crate::widgets::{Badge, BadgeTone, Button, FormError, focus_ring, keyboard_focus};

/// A `<p>` in the page's own type: 16px with the root's 1.5.
const TEXT: TextStyle = TextStyle::new(16.0, 400.0, 1.5);
/// `.feedback-entry__image img`: at most 160px high.
const THUMB_MAX_H: f32 = 160.0;
/// `<Textarea rows={3}>`: three rows of 21px plus the field's padding and
/// border.
const REPLY_HEIGHT: f32 = 3.0 * 21.0 + 18.0;

/// The area's state for one visit (`PageState` starts it afresh).
#[derive(Default)]
pub struct State {
    /// The entries loaded so far, newest activity first.
    entries: Vec<FeedbackEntry>,
    total: u64,
    /// The first page has answered once.
    loaded: bool,
    /// The last read of the list failed.
    failed: bool,
    /// A page on its way, and whether it is the first (a fresh read) or
    /// the next one ("Mehr laden").
    request: Option<(bool, Answer<FeedbackPage>)>,
    /// `feedback::sent` when the list was asked for.
    sent: Option<u64>,
    /// Messages that were new when this visit showed them (`rememberNew`).
    marks: HashSet<Uuid>,
    /// Entries whose read call went out in this visit (`entriesToRead`).
    asked: HashSet<Uuid>,
    reads: Vec<(Uuid, Answer<()>)>,
    /// "Mitgeschickt" opened.
    context_open: HashSet<Uuid>,
    replies: HashMap<Uuid, ReplyForm>,
    /// The image shown large.
    lightbox: Option<String>,
}

/// "Antworten" and the form it opens in place.
#[derive(Default)]
struct ReplyForm {
    open: bool,
    text: String,
    errors: FormErrors,
    sending: Option<Answer<FeedbackMessage>>,
    /// "Antwort gesendet." for screen readers.
    note: Option<&'static str>,
    /// Focus goes back to "Antworten" once the form is gone.
    focus_toggle: bool,
    /// The text field takes the focus when the form opens.
    focus_text: bool,
}

/// `sentAlongLine`: "Web-App · Linux · 0.10.0 · Deutsch · /s/…", leaving out
/// what was not sent along.
pub fn sent_along_line(context: &FeedbackEntryContext) -> String {
    let app = match context.client {
        FeedbackClient::Desktop => texts::context_desktop_app(),
        FeedbackClient::Web | FeedbackClient::Form => texts::context_web_app(),
    };
    let system = context
        .os
        .map(|os| super::security::os_name(os).unwrap_or(texts::context_unknown_system()));
    let locale = Locale::parse(&context.locale).map(language::option);
    [
        Some(app),
        system,
        context.app_version.as_deref(),
        locale,
        context.page.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

fn fields(at: chrono::DateTime<Utc>) -> intl::DateTime {
    let at = at.with_timezone(&Local);
    intl::DateTime {
        year: at.year(),
        month: at.month(),
        day: at.day(),
        weekday: at.weekday().num_days_from_monday(),
        hour: at.hour(),
        minute: at.minute(),
    }
}

/// `dateStyle: "medium", timeStyle: "short"`: "12.11.2026, 19:15" /
/// "12 Nov 2026, 19:15".
pub fn message_time(at: chrono::DateTime<Utc>) -> String {
    let at = fields(at);
    format!(
        "{}, {}",
        intl::format_date(&at, intl::DateStyle::Medium),
        intl::format_time(&at)
    )
}

fn kind_label(kind: FeedbackKind) -> &'static str {
    match kind {
        FeedbackKind::Idea => texts::kind_idea(),
        FeedbackKind::Problem => texts::kind_problem(),
        FeedbackKind::Praise => texts::kind_praise(),
    }
}

/// `sendFieldErrors` for a reply: the text's codes under the field, the
/// rest for the form.
fn reply_errors(errors: &mut FormErrors, error: &ApiError) {
    let failure = Failure::from_api(error);
    if failure.code == "validation_failed"
        && let Some(code) = failure.fields.get("text")
    {
        let message = match code.as_str() {
            "required" => texts::text_required().to_owned(),
            "too_long" => texts::text_too_long().to_owned(),
            code => field_message("text", code),
        };
        errors.set_fields(vec![("text", message)]);
        return;
    }
    errors.from_failure(&failure, &["text"]);
}

/// Asks for the first page again, or the next one.
fn ask(app: &AppCtx, s: &mut State, first: bool) {
    let offset = if first { 0 } else { s.entries.len() as u32 };
    let http = app.client.http().clone();
    s.request = Some((
        first,
        Pending::spawn(app.client, async move { http.feedback_list(offset).await }),
    ));
}

/// Takes what finished: pages, read calls and replies.
fn take(ctx: &egui::Context, app: &AppCtx, s: &mut State) {
    if let Some((first, pending)) = &s.request
        && let Some(result) = pending.take()
    {
        let first = *first;
        s.request = None;
        match result {
            Ok(page) => {
                s.failed = false;
                s.loaded = true;
                s.total = page.total;
                if first {
                    s.entries = page.entries;
                } else {
                    let known: HashSet<Uuid> = s.entries.iter().map(|e| e.id).collect();
                    s.entries
                        .extend(page.entries.into_iter().filter(|e| !known.contains(&e.id)));
                }
                // `rememberNew`: a later read says `new: false`, the marks stay.
                for message in s.entries.iter().flat_map(|e| &e.messages) {
                    if message.new {
                        s.marks.insert(message.id);
                    }
                }
            }
            Err(error) => {
                tracing::warn!("feedback list: {error}");
                s.failed = true;
            }
        }
    }
    // `useReadOnSight`: each entry with unread replies, once a visit.
    for entry in &s.entries {
        if entry.unread && !s.asked.contains(&entry.id) {
            s.asked.insert(entry.id);
            let http = app.client.http().clone();
            let id = entry.id;
            s.reads.push((
                id,
                Pending::spawn(app.client, async move { http.read_feedback(id).await }),
            ));
        }
    }
    let mut read = false;
    s.reads.retain_mut(|(id, pending)| match pending.take() {
        None => true,
        Some(Ok(())) => {
            if let Some(entry) = s.entries.iter_mut().find(|e| e.id == *id) {
                entry.unread = false;
            }
            read = true;
            false
        }
        Some(Err(error)) => {
            // Tried again with the next list it shows up in.
            tracing::warn!("feedback read: {error}");
            s.asked.remove(id);
            false
        }
    });
    let mut replied = false;
    for (id, form) in &mut s.replies {
        if let Some(result) = poll(&mut form.sending) {
            match result {
                Ok(message) => {
                    form.text.clear();
                    form.errors = FormErrors::default();
                    form.open = false;
                    form.note = Some(texts::thread_reply_sent());
                    form.focus_toggle = true;
                    // A reply opens a done entry again.
                    if let Some(entry) = s.entries.iter_mut().find(|e| e.id == *id) {
                        entry.status = FeedbackState::Open;
                        entry.last_activity_at = message.created_at;
                        if !entry.messages.iter().any(|m| m.id == message.id) {
                            entry.messages.push(message);
                        }
                    }
                    replied = true;
                }
                Err(error) => reply_errors(&mut form.errors, &error),
            }
        }
    }
    if read || replied {
        crate::screens::feedback::refresh(ctx);
    }
}

/// Settings area "Feedback".
pub fn area(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let sent = crate::screens::feedback::sent(&ctx);
    // A visit reads the list; an entry sent since reads it again.
    if s.sent != Some(sent) {
        s.sent = Some(sent);
        ask(app, s, true);
    }
    take(&ctx, app, s);

    // `.feedback-area`: a column with 16px gaps.
    let button = Button::new(texts::area_give()).primary();
    let w = button.natural_width(ui);
    let at = Rect::from_min_size(ui.cursor().min, vec2(w, 36.0));
    if ui.put(at, button.width(w)).clicked() {
        crate::screens::feedback::open_dialog(&ctx, app);
    }
    ui.add_space(space::S4);

    let loading = s.request.as_ref().is_some_and(|(first, _)| *first);
    if !s.loaded && !s.failed {
        crate::widgets::skeleton::rows(ui, 3);
    } else if s.failed && s.entries.is_empty() {
        if crate::widgets::load_error(ui, texts::area_load_failed(), loading) && !loading {
            ask(app, s, true);
        }
    } else if s.entries.is_empty() {
        empty(ui);
    } else {
        let width = ui.available_width();
        let ids: Vec<Uuid> = s.entries.iter().map(|e| e.id).collect();
        for (i, id) in ids.into_iter().enumerate() {
            if i > 0 {
                ui.add_space(space::S3);
            }
            entry(ui, app, s, id, width);
        }
        let more = !s.entries.is_empty() && (s.entries.len() as u64) < s.total;
        if more {
            ui.add_space(space::S4);
            let pending = s.request.as_ref().is_some_and(|(first, _)| !*first);
            let button =
                Button::new(texts::area_more()).pending(pending.then_some(texts::area_more()));
            let w = button.natural_width(ui);
            let at = Rect::from_min_size(
                pos2(ui.cursor().min.x + (width - w) / 2.0, ui.cursor().min.y),
                vec2(w, 36.0),
            );
            if ui.put(at, button.width(w)).clicked() && s.request.is_none() {
                ask(app, s, false);
            }
        }
    }

    if let Some(url) = s.lightbox.clone() {
        let (close, download) =
            crate::screens::chat::attachments::lightbox(&ctx, app.images, &url, texts::image_alt());
        if download {
            crate::screens::chat::attachments::download(&ctx, app.client, &url, texts::image_alt());
        }
        if close {
            s.lightbox = None;
        }
    }
}

/// A wrapped paragraph at the cursor, read out as it shows.
fn paragraph(ui: &mut Ui, s: &str, style: TextStyle, color: Color32, role: egui::Role) {
    let width = ui.available_width();
    let block = Block::wrapped(ui, s, style, color, width);
    let (rect, response) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
    block.paint(ui.painter(), rect.min);
    crate::widgets::announce(&response, role, s);
}

/// `.feedback-empty`: the line and its hint, 4px apart.
fn empty(ui: &mut Ui) {
    let c = Theme::of(ui.ctx()).c;
    paragraph(ui, texts::area_empty(), TEXT, c.text, egui::Role::Label);
    ui.add_space(space::S1);
    paragraph(
        ui,
        texts::area_empty_hint(),
        text::META,
        c.muted,
        egui::Role::Label,
    );
}

/// `.feedback-entry`: a bordered card, padding 16, its parts 12px apart.
fn entry(ui: &mut Ui, app: &mut AppCtx, s: &mut State, id: Uuid, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let Some(entry) = s.entries.iter().find(|e| e.id == id).cloned() else {
        return;
    };
    let top = ui.cursor().min;
    let background = ui.painter().add(Shape::Noop);
    let pad = 1.0 + space::S4;
    let inner = Rect::from_min_size(
        pos2(top.x + pad, top.y + pad),
        vec2(width - 2.0 * pad, f32::INFINITY),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing = egui::Vec2::ZERO;
    entry_body(&mut child, app, s, &entry);
    let rect = Rect::from_min_max(top, pos2(top.x + width, child.min_rect().bottom() + pad));
    ui.painter().set(
        background,
        egui::epaint::RectShape::new(
            rect,
            corner(theme.radius.md),
            c.surface,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        ),
    );
    ui.advance_cursor_after_rect(rect);
}

fn entry_body(ui: &mut Ui, app: &mut AppCtx, s: &mut State, entry: &FeedbackEntry) {
    let c = Theme::of(ui.ctx()).c;
    let id = entry.id;
    head(ui, entry);
    ui.add_space(space::S3);
    paragraph(ui, &entry.text, TEXT, c.text, egui::Role::Label);
    if let Some(image) = &entry.image {
        ui.add_space(space::S3);
        if thumbnail(ui, app, &image.url) {
            s.lightbox = Some(image.url.clone());
        }
    }
    ui.add_space(space::S3);
    context(ui, s, entry);
    if !entry.messages.is_empty() {
        ui.add_space(space::S3);
        thread(ui, s, entry);
    }
    if entry.status == FeedbackState::Done {
        ui.add_space(space::S3);
        paragraph(
            ui,
            texts::thread_done_note(),
            text::META,
            c.muted,
            egui::Role::Label,
        );
    }
    ui.add_space(space::S3);
    let form = s.replies.entry(id).or_default();
    reply(ui, app, form, id);
}

/// A run of badges and meta text on one line, centred on `height`, 8px
/// apart (`.feedback-entry__head`, `.feedback-message__meta`).
enum Piece<'a> {
    Badge(&'a str, BadgeTone),
    Text(&'a str, TextStyle, Color32),
}

fn meta_row(ui: &mut Ui, salt: Uuid, pieces: &[Piece<'_>]) {
    let c = Theme::of(ui.ctx()).c;
    let height = if pieces.iter().any(|p| matches!(p, Piece::Badge(..))) {
        20.0
    } else {
        text::META.line_px()
    };
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut x = rect.left();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            x += space::S2;
        }
        match piece {
            Piece::Badge(label, tone) => {
                let mut slot = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(
                    pos2(x, rect.center().y - 10.0),
                    vec2(width, 20.0),
                )));
                let response = slot.add(Badge::new(label).tone(*tone).on(c.surface));
                crate::widgets::announce(&response, egui::Role::Label, label);
                x = response.rect.right();
            }
            Piece::Text(s, style, color) => {
                let block = Block::single(ui, s, *style, *color);
                let at = pos2(x, rect.center().y - block.height() / 2.0);
                block.paint(ui.painter(), at);
                let id = egui::Id::unique(("feedback-meta", salt, i));
                let response =
                    ui.interact(Rect::from_min_size(at, block.size()), id, Sense::hover());
                crate::widgets::announce(&response, egui::Role::Label, s);
                x += block.width();
            }
        }
    }
}

/// The kind, the date and "Offen" / "Erledigt".
fn head(ui: &mut Ui, entry: &FeedbackEntry) {
    let c = Theme::of(ui.ctx()).c;
    let date = crate::rules::format_day(entry.created_at);
    let (status, tone) = match entry.status {
        FeedbackState::Open => (texts::status_open(), BadgeTone::Neutral),
        FeedbackState::Done => (texts::status_done(), BadgeTone::Success),
    };
    meta_row(
        ui,
        entry.id,
        &[
            Piece::Badge(kind_label(entry.kind), BadgeTone::Neutral),
            Piece::Text(&date, text::META, c.muted),
            Piece::Badge(status, tone),
        ],
    );
}

/// `.feedback-entry__image`: the image at most 160px high in a bordered
/// button that enlarges it. True when pressed.
fn thumbnail(ui: &mut Ui, app: &AppCtx, url: &str) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let max_w = ui.available_width() - 2.0;
    let state = app.images.shown(ui.ctx(), url, Fit::Inline);
    let natural = app.images.natural(ui.ctx(), url, Fit::Inline);
    let shown = match (&state, natural) {
        (ImageState::Ready(_), Some(size)) if size.x > 0.0 && size.y > 0.0 => {
            size * (THUMB_MAX_H / size.y).min(max_w / size.x).min(1.0)
        }
        // Not here yet, or not a picture: the box alone.
        _ => vec2(120.0, 80.0),
    };
    let (rect, response) = ui.allocate_exact_size(shown + vec2(2.0, 2.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::ZoomIn);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, texts::image_alt()));
    let painter = ui.painter();
    painter.rect_filled(rect, corner(theme.radius.md), c.bg);
    if let ImageState::Ready(texture) = &state {
        painter.image(
            texture.id(),
            rect.shrink(1.0),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(
        rect,
        corner(theme.radius.md),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(painter, rect, theme.radius.md, &theme);
    }
    response.clicked()
}

/// `.feedback-entry__context`: "Mitgeschickt", closed until opened, and
/// the line of what went along.
fn context(ui: &mut Ui, s: &mut State, entry: &FeedbackEntry) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let open = s.context_open.contains(&entry.id);
    let summary = Block::single(ui, texts::thread_sent_along(), text::META, c.muted);
    // Chromium's marker in front of a 12px summary.
    let marker_w = 13.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), summary.height()), Sense::hover());
    let summary_rect =
        Rect::from_min_size(rect.min, vec2(marker_w + summary.width(), summary.height()));
    let toggle = ui.interact(
        summary_rect,
        egui::Id::unique(("feedback-sent-along", entry.id)),
        Sense::click(),
    );
    toggle.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::DisclosureTriangle,
            true,
            open,
            texts::thread_sent_along(),
        )
    });
    if toggle.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    if toggle.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(painter, summary_rect, 2.0, &theme);
    }
    crate::screens::feedback::dialog::triangle(painter, rect.min, summary.height(), open, c.muted);
    summary.paint(painter, pos2(rect.left() + marker_w, rect.top()));
    if toggle.clicked() {
        if open {
            s.context_open.remove(&entry.id);
        } else {
            s.context_open.insert(entry.id);
        }
    }
    if open {
        ui.add_space(space::S1);
        paragraph(
            ui,
            &sent_along_line(&entry.context),
            text::META,
            c.muted,
            egui::Role::Label,
        );
    }
}

/// `.feedback-thread`: the messages, 8px apart, behind a 2px rule.
fn thread(ui: &mut Ui, s: &State, entry: &FeedbackEntry) {
    let c = Theme::of(ui.ctx()).c;
    let top = ui.cursor().min;
    let width = ui.available_width();
    let inner = Rect::from_min_size(
        pos2(top.x + 2.0 + space::S3, top.y),
        vec2(width - 2.0 - space::S3, f32::INFINITY),
    );
    let mut list = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    list.spacing_mut().item_spacing = egui::Vec2::ZERO;
    for (i, message) in entry.messages.iter().enumerate() {
        if i > 0 {
            list.add_space(space::S2);
        }
        let author = match message.from {
            FeedbackAuthor::You => texts::thread_you(),
            FeedbackAuthor::Operators => texts::thread_operators(),
        };
        let when = message_time(message.created_at);
        let mut pieces = vec![
            Piece::Text(author, text::META.weight(600.0), c.text),
            Piece::Text(&when, text::META, c.muted),
        ];
        if message.new || s.marks.contains(&message.id) {
            pieces.push(Piece::Badge(texts::thread_new(), BadgeTone::Accent));
        }
        meta_row(&mut list, message.id, &pieces);
        paragraph(&mut list, &message.text, TEXT, c.text, egui::Role::Label);
    }
    let bottom = list.min_rect().bottom();
    let rule = Rect::from_min_max(top, pos2(top.x + 2.0, bottom));
    ui.painter().rect_filled(rule, 0.0, c.border);
    ui.advance_cursor_after_rect(Rect::from_min_max(top, pos2(top.x + width, bottom)));
}

/// `ReplyForm`: "Antworten", or the form it opens with "Deine Antwort",
/// "Abbrechen" and "Senden".
fn reply(ui: &mut Ui, app: &AppCtx, form: &mut ReplyForm, id: Uuid) {
    let c = Theme::of(ui.ctx()).c;
    if let Some(note) = form.note {
        // `role="status"`: read out, not shown.
        let response = ui.interact(
            Rect::NOTHING,
            egui::Id::unique(("feedback-reply-note", id)),
            Sense::hover(),
        );
        crate::widgets::announce(&response, egui::Role::Status, note);
    }
    if !form.open {
        let button = Button::new(texts::thread_reply()).small();
        let w = button.natural_width(ui);
        let at = Rect::from_min_size(ui.cursor().min, vec2(w, 32.0));
        let response = ui.put(at, button.width(w));
        if std::mem::take(&mut form.focus_toggle) {
            response.request_focus();
        }
        if response.clicked() {
            form.note = None;
            form.open = true;
            form.focus_text = true;
        }
        return;
    }
    let pending = form.sending.is_some();
    let width = ui.available_width();
    // `.feedback-reply__form`: 12px apart; the label 6px over the field.
    let label = Block::single(ui, texts::thread_reply_label(), text::UI, c.text);
    let (rect, _) = ui.allocate_exact_size(vec2(width, label.height()), Sense::hover());
    label.paint(ui.painter(), rect.min);
    ui.add_space(space::S1_5);
    let error = form.errors.field("text").map(str::to_owned);
    let response = controls::textarea_with(
        ui,
        ("feedback-reply", id),
        texts::thread_reply_label(),
        &mut form.text,
        REPLY_HEIGHT,
        error.is_some(),
        TextareaExtras {
            placeholder: None,
            char_limit: Some(FEEDBACK_TEXT_MAX),
            read_only: pending,
        },
    );
    if std::mem::take(&mut form.focus_text) {
        response.request_focus();
    }
    if response.changed() {
        form.errors.clear_field("text");
    }
    if let Some(error) = &error {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(error.as_str()));
        ui.add_space(space::S1_5);
        paragraph(ui, error, text::META, c.danger, egui::Role::Alert);
    }
    if let Some(message) = &form.errors.form {
        ui.add_space(space::S3);
        ui.add(FormError::new(message).on(c.surface));
    }
    ui.add_space(space::S3);
    let mut send = false;
    let mut cancel = false;
    let cooldown = form.errors.cooldown();
    ui.allocate_ui_with_layout(
        vec2(width, 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::S2;
            if cooldown > 0 {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
                let waiting = gelabber_i18n::app::submit_cooldown(cooldown);
                ui.add(Button::new(&waiting).primary().disabled(true));
            } else if ui
                .add(
                    Button::new(texts::submit())
                        .primary()
                        .pending(pending.then_some(texts::sending())),
                )
                .clicked()
                && !pending
            {
                send = true;
            }
            if ui
                .add(Button::new(common::cancel()).ghost().disabled(pending))
                .clicked()
                && !pending
            {
                cancel = true;
            }
        },
    );
    if cancel {
        form.open = false;
        form.errors = FormErrors::default();
        form.focus_toggle = true;
    } else if send {
        if form.text.trim().is_empty() {
            form.errors
                .set_fields(vec![("text", texts::text_required().to_owned())]);
            return;
        }
        form.errors = FormErrors::default();
        let http = app.client.http().clone();
        let text = form.text.trim().to_owned();
        form.sending = Some(Pending::spawn(app.client, async move {
            http.reply_feedback(id, &text).await
        }));
    }
}

#[cfg(test)]
mod tests {
    //! Einstellungen → Feedback against the stand-in API
    //! (FeedbackArea.test.tsx).

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    use super::*;
    use crate::app::testkit::{FakeApi, Reply, launcher, wait_for, wait_for_label, workspace};
    use crate::platform::paths::testing::TempConfig;

    const OPEN: &str = "00000000-0000-4000-8000-0000000000e1";
    const DONE: &str = "00000000-0000-4000-8000-0000000000e2";
    const REPLY: &str = "00000000-0000-4000-8000-0000000000a1";

    fn entries() -> Value {
        json!({"total": 2, "entries": [
            {
                "id": OPEN, "kind": "problem", "status": "open",
                "text": "Das Vorschaubild bleibt schwarz.",
                "created_at": "2026-11-12T09:00:00Z",
                "last_activity_at": "2026-11-12T10:00:00Z", "image": null,
                "context": {"client": "web", "os": "linux", "app_version": "0.10.0",
                            "locale": "de", "page": "/s/x"},
                "unread": true,
                "messages": [{"id": REPLY, "from": "operators",
                              "text": "Welchen Browser nutzt du?",
                              "created_at": "2026-11-12T10:00:00Z", "new": true}]
            },
            {
                "id": DONE, "kind": "praise", "status": "done", "text": "Schön.",
                "created_at": "2026-11-11T09:00:00Z",
                "last_activity_at": "2026-11-11T09:00:00Z", "image": null,
                "context": {"client": "desktop", "os": null, "app_version": null,
                            "locale": "en", "page": null},
                "unread": false,
                "messages": [{"id": "00000000-0000-4000-8000-0000000000a2", "from": "you",
                              "text": "Danke!", "created_at": "2026-11-11T09:30:00Z",
                              "new": false}]
            }
        ]})
    }

    type Posts = Arc<Mutex<Vec<(String, String)>>>;

    /// Lena on an instance that takes feedback; `list` answers the list.
    fn api(list: impl Fn() -> Reply + Send + Sync + 'static, unread: u32, posts: Posts) -> FakeApi {
        FakeApi::start(true, move |method, path, body| match (method, path) {
            ("GET", "/feedback/status") => {
                Some(Reply::json(json!({"enabled": true, "unread": unread})))
            }
            ("GET", "/feedback") => Some(list()),
            ("POST", p) if p.starts_with("/feedback/") => {
                posts.lock().push((p.to_owned(), body.to_owned()));
                if p.ends_with("/read") {
                    Some(Reply::status(204, Value::Null))
                } else {
                    Some(Reply::status(
                        201,
                        json!({"id": "00000000-0000-4000-8000-0000000000a3", "from": "you",
                               "text": "Firefox.", "created_at": "2026-11-12T11:00:00Z",
                               "new": false}),
                    ))
                }
            }
            _ => workspace(method, path),
        })
    }

    #[test]
    fn what_went_along_leaves_out_what_was_not_sent() {
        let _german = gelabber_i18n::on_this_thread(Locale::De);
        let context = FeedbackEntryContext {
            client: FeedbackClient::Web,
            os: Some(gelabber_client::DeviceOs::Linux),
            app_version: Some("0.10.0".into()),
            locale: "de".into(),
            page: Some("/s/x".into()),
        };
        assert_eq!(
            sent_along_line(&context),
            "Web-App · Linux · 0.10.0 · Deutsch · /s/x"
        );
        let bare = FeedbackEntryContext {
            client: FeedbackClient::Desktop,
            os: Some(gelabber_client::DeviceOs::Other),
            app_version: None,
            locale: "en".into(),
            page: None,
        };
        assert_eq!(sent_along_line(&bare), "Desktop-App · Unbekannt · English");
    }

    /// Each entry as a thread: kind, status, Du and Betreiber, "Neu" on the
    /// reply that was new, the done note; the unread entry is marked read
    /// once, and the nav row says so until the status is read again.
    #[test]
    fn entries_show_as_threads_and_unread_ones_are_marked_read_once() {
        let _config = TempConfig::new("feedback-area");
        let posts: Posts = Arc::default();
        let api = api(|| Reply::json(entries()), 1, posts.clone());
        let mut harness = launcher(&api, "/settings/feedback");
        wait_for_label(&mut harness, "Welchen Browser nutzt du?");
        for shown in [
            "Das Vorschaubild bleibt schwarz.",
            "Problem",
            "Offen",
            "Lob",
            "Erledigt",
            "Betreiber",
            "Du",
            "Neu",
            "Die Betreiber haben das als erledigt markiert. Wenn du antwortest, ist es wieder offen.",
        ] {
            assert!(harness.query_by_label(shown).is_some(), "{shown}");
        }
        assert!(
            harness
                .query_by_role_and_label(Role::Link, "Feedback, neue Antwort")
                .is_some()
        );
        wait_for(&mut harness, "the read call", |_| !posts.lock().is_empty());
        harness.run_steps(10);
        let posts = posts.lock().clone();
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert_eq!(posts[0].0, format!("/feedback/{OPEN}/read"));
        // The mark stays for the visit.
        assert!(harness.query_by_label("Neu").is_some());
        // "Mitgeschickt" opens on the line of what went along.
        harness
            .query_all_by_role_and_label(Role::DisclosureTriangle, "Mitgeschickt")
            .next()
            .expect("Mitgeschickt")
            .click();
        wait_for_label(&mut harness, "Web-App · Linux · 0.10.0 · Deutsch · /s/x");
    }

    #[test]
    fn an_empty_list_is_explained_with_the_button() {
        let _config = TempConfig::new("feedback-area-empty");
        let api = api(
            || Reply::json(json!({"total": 0, "entries": []})),
            0,
            Arc::default(),
        );
        let mut harness = launcher(&api, "/settings/feedback");
        wait_for_label(&mut harness, "Du hast noch kein Feedback geschickt.");
        assert!(
            harness
                .query_by_label(
                    "Ideen, Probleme und Lob gehen direkt an die Betreiber dieser Instanz."
                )
                .is_some()
        );
        assert!(
            harness
                .query_by_role_and_label(Role::Link, "Feedback")
                .is_some()
        );
        harness
            .get_by_role_and_label(Role::Button, "Feedback geben")
            .click();
        wait_for_label(&mut harness, "Wird mitgeschickt");
    }

    #[test]
    fn a_failed_list_offers_a_retry() {
        let _config = TempConfig::new("feedback-area-error");
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let api = api(
            move || {
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    Reply::error(500, "internal")
                } else {
                    Reply::json(json!({"total": 0, "entries": []}))
                }
            },
            0,
            Arc::default(),
        );
        let mut harness = launcher(&api, "/settings/feedback");
        wait_for_label(&mut harness, "Feedback konnte nicht geladen werden.");
        harness
            .get_by_role_and_label(Role::Button, gelabber_i18n::common::try_again())
            .click();
        wait_for_label(&mut harness, "Du hast noch kein Feedback geschickt.");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// "Antworten" opens the form; an empty reply is refused on the spot;
    /// a sent one is appended and the done entry is open again.
    #[test]
    fn a_reply_is_sent_and_appended() {
        let _config = TempConfig::new("feedback-area-reply");
        let posts: Posts = Arc::default();
        let api = api(|| Reply::json(entries()), 0, posts.clone());
        let mut harness = launcher(&api, "/settings/feedback");
        wait_for_label(&mut harness, "Erledigt");
        // The second entry's "Antworten" (the done one).
        harness
            .query_all_by_role_and_label(Role::Button, "Antworten")
            .nth(1)
            .expect("Antworten")
            .click();
        wait_for_label(&mut harness, "Deine Antwort");
        harness
            .get_by_role_and_label(Role::Button, "Senden")
            .click();
        wait_for_label(&mut harness, "Schreib ein paar Worte.");
        let field = || {
            harness
                .query_all_by_label("Deine Antwort")
                .find(|n| n.accesskit_node().role() == Role::MultilineTextInput)
                .expect("the field")
        };
        field().focus();
        harness.step();
        harness
            .query_all_by_label("Deine Antwort")
            .find(|n| n.accesskit_node().role() == Role::MultilineTextInput)
            .expect("the field")
            .type_text("Firefox.");
        harness.run_steps(2);
        harness
            .get_by_role_and_label(Role::Button, "Senden")
            .click();
        wait_for_label(&mut harness, "Firefox.");
        wait_for(&mut harness, "the form closes", |h| {
            h.query_by_label("Deine Antwort").is_none()
        });
        assert!(harness.query_by_label("Erledigt").is_none());
        let posts = posts.lock().clone();
        let sent = posts
            .iter()
            .find(|(p, _)| p == &format!("/feedback/{DONE}/messages"))
            .expect("the reply");
        let body: Value = serde_json::from_str(&sent.1).expect("json");
        assert_eq!(body["text"], "Firefox.");
    }

    /// An instance without feedback: no nav row, and the address falls
    /// back to the default area.
    #[test]
    fn without_feedback_the_area_is_hidden() {
        let _config = TempConfig::new("feedback-area-off");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, "/settings/feedback");
        wait_for(&mut harness, "the status", |_| {
            api.saw("GET /feedback/status")
        });
        let start = std::time::Instant::now();
        loop {
            harness.step();
            let route = harness.state_mut().app().unwrap().nav().route().path();
            if route == "/settings/darstellung" {
                break;
            }
            assert!(start.elapsed().as_secs() < 15, "still at {route}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            harness
                .query_by_role_and_label(Role::Link, "Feedback")
                .is_none()
        );
    }
}
