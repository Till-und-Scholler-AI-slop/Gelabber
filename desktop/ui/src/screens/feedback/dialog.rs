//! "Feedback geben" (feedback/FeedbackDialog.tsx, feedback.css): a kind, a
//! text, one optional image and the list of what goes along. One dialog for
//! the whole app; its draft stays when it closes, until it is sent or the
//! app quits. The image comes from "Bild anhängen" or a file dropped on the
//! window (egui delivers no pasted image) and goes up as it is, right
//! before the entry is created.

use std::path::PathBuf;

use egui::{Color32, ColorImage, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gelabber_client::{
    ApiError, FEEDBACK_IMAGE_MAX_BYTES, FEEDBACK_TEXT_MAX, FeedbackContext, FeedbackEntry,
    FeedbackKind, NewFeedback,
};
use gelabber_i18n::{common, feedback as texts, language};

use super::{Answer, State};
use crate::app::AppCtx;
use crate::icons::Icon;
use crate::rules::{Failure, FormErrors, field_message};
use crate::screens::settings::controls::{self, Picked, TextareaExtras};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, space};
use crate::widgets::{
    Button, FormError, IconButton, Link, Modal, focus_ring, keyboard_focus, modal::ModalWidth,
};

/// From this many characters the dialog counts down (`COUNTER_FROM`).
const COUNTER_FROM: usize = 3600;
/// The thumbnail's height at most (`.feedback-form__thumb img`).
const THUMB_MAX_H: f32 = 160.0;
/// Six rows of 21px plus the field's padding and border
/// (`min-block-size: calc(6lh + 18px)`).
const TEXT_HEIGHT: f32 = 6.0 * 21.0 + 18.0;
/// The chooser's filter (`accept="image/png,image/jpeg,image/webp"`).
const FILTER: [&str; 5] = ["*.png", "*.jpg", "*.jpeg", "*.webp", "*.PNG"];

/// The image that goes along, as it was chosen.
pub(super) struct Attached {
    bytes: Vec<u8>,
    content_type: &'static str,
    /// Decoded for the thumbnail only; `None` if it does not decode here.
    preview: Option<ColorImage>,
    /// The image's own size in pixels, which the thumbnail never exceeds.
    natural: egui::Vec2,
    texture: Option<egui::TextureHandle>,
}

/// The draft: kept while the dialog is closed (`dialogStore.ts`).
#[derive(Default)]
struct Draft {
    kind: Option<FeedbackKind>,
    text: String,
    image: Option<Attached>,
}

impl Draft {
    fn kind(&self) -> FeedbackKind {
        self.kind.unwrap_or(FeedbackKind::Idea)
    }
}

/// Why a file cannot go along (`imageProblem`), or that it did not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refused {
    WrongType,
    TooLarge,
    Unreadable,
}

impl Refused {
    fn message(self) -> &'static str {
        match self {
            Self::WrongType => texts::image_wrong_type(),
            Self::TooLarge => texts::image_too_large(),
            Self::Unreadable => texts::image_invalid(),
        }
    }
}

enum Source {
    Bytes(Vec<u8>),
    Path(PathBuf),
}

pub(super) struct Dialog {
    open: bool,
    /// The page the dialog was opened on, fixed while it is open.
    path: String,
    draft: Draft,
    errors: FormErrors,
    /// "Bild angehängt." / "Bild ersetzt." for screen readers.
    note: Option<&'static str>,
    sending: Option<Answer<FeedbackEntry>>,
    picking: Option<Picked>,
    reading: Option<Pending<Result<Attached, Refused>>>,
    /// "Wird mitgeschickt" is open until the person closes it.
    details_closed: bool,
}

impl Default for Dialog {
    fn default() -> Self {
        Self {
            open: false,
            path: "/".into(),
            draft: Draft::default(),
            errors: FormErrors::default(),
            note: None,
            sending: None,
            picking: None,
            reading: None,
            details_closed: false,
        }
    }
}

impl Dialog {
    pub(super) fn open(&mut self, path: &str) {
        if !self.open {
            self.path = path.to_owned();
            self.errors = FormErrors::default();
            self.note = None;
        }
        self.open = true;
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }
}

/// `sniff` of the API: PNG, JPEG or WebP by their first bytes.
pub(super) fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The type first, then the size, as the web's `imageProblem` checks.
fn check(bytes: &[u8], size: u64) -> Result<&'static str, Refused> {
    let content_type = sniff(bytes).ok_or(Refused::WrongType)?;
    if size > FEEDBACK_IMAGE_MAX_BYTES as u64 {
        return Err(Refused::TooLarge);
    }
    Ok(content_type)
}

/// Reads and checks a chosen or dropped file off the UI thread. A dropped
/// file over the limit is not read beyond its first bytes. The bytes stay
/// as they are; only the thumbnail is scaled.
fn read(source: Source) -> Result<Attached, Refused> {
    let bytes = match source {
        Source::Bytes(bytes) => bytes,
        Source::Path(path) => {
            use std::io::Read;
            let mut file = std::fs::File::open(&path).map_err(|_| Refused::Unreadable)?;
            let size = file.metadata().map_err(|_| Refused::Unreadable)?.len();
            if size > FEEDBACK_IMAGE_MAX_BYTES as u64 {
                let mut head = [0u8; 12];
                let n = file.read(&mut head).map_err(|_| Refused::Unreadable)?;
                check(&head[..n], size)?;
                return Err(Refused::TooLarge);
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|_| Refused::Unreadable)?;
            bytes
        }
    };
    let content_type = check(&bytes, bytes.len() as u64)?;
    let decoded = image::load_from_memory(&bytes).ok();
    let natural = decoded.as_ref().map_or(egui::Vec2::ZERO, |img| {
        vec2(img.width() as f32, img.height() as f32)
    });
    let preview = decoded.map(|img| {
        // At most twice the largest size it shows at, for sharp edges.
        let (w, h) = (1024, (THUMB_MAX_H * 2.0) as u32);
        let small = if img.width() > w || img.height() > h {
            img.thumbnail(w, h)
        } else {
            img
        }
        .to_rgba8();
        let size = [small.width() as usize, small.height() as usize];
        ColorImage::from_rgba_unmultiplied(size, small.as_raw())
    });
    Ok(Attached {
        bytes,
        content_type,
        preview,
        natural,
        texture: None,
    })
}

/// `sendFieldErrors`: a failed send under its fields, else for the form.
fn send_errors(errors: &mut FormErrors, error: &ApiError) {
    if error.status() == Some(413) {
        errors.set_fields(vec![("image", texts::image_too_large().to_owned())]);
        return;
    }
    let failure = Failure::from_api(error);
    if failure.code == "validation_failed" {
        let mut fields = Vec::new();
        match failure.fields.get("text").map(String::as_str) {
            Some("required") => fields.push(("text", texts::text_required().to_owned())),
            Some("too_long") => fields.push(("text", texts::text_too_long().to_owned())),
            Some(code) => fields.push(("text", field_message("text", code))),
            None => {}
        }
        if failure.fields.contains_key("image") {
            fields.push(("image", texts::image_invalid().to_owned()));
        }
        if !fields.is_empty() {
            errors.set_fields(fields);
            return;
        }
    }
    errors.from_failure(&failure, &["text", "image"]);
}

/// Takes what finished: a chosen file, a read image, a send. Runs every
/// frame, so a send outlives a closed dialog as the web's mutation does.
fn take(ctx: &egui::Context, app: &mut AppCtx, s: &mut State) {
    let d = &mut s.dialog;
    if let Some(slot) = &d.picking {
        let done = slot.lock().take();
        if let Some(file) = done {
            d.picking = None;
            if let Some((bytes, _)) = file {
                d.reading = Some(Pending::thread(
                    ctx,
                    move || read(Source::Bytes(bytes)),
                    || Err(Refused::Unreadable),
                ));
            }
        }
    }
    if let Some(result) = poll(&mut d.reading) {
        match result {
            Ok(image) => {
                d.errors.clear_field("image");
                d.note = Some(if d.draft.image.is_some() {
                    texts::image_replaced()
                } else {
                    texts::image_attached()
                });
                d.draft.image = Some(image);
            }
            Err(refused) => {
                d.note = None;
                d.errors
                    .set_fields(vec![("image", refused.message().to_owned())]);
            }
        }
    }
    if let Some(result) = poll(&mut d.sending) {
        match result {
            Ok(_) => {
                d.open = false;
                d.draft = Draft::default();
                d.note = None;
                d.errors = FormErrors::default();
                app.toasts.notify_action(
                    texts::toast_sent(),
                    texts::toast_view(),
                    "/settings/feedback",
                );
                super::stale(&mut s.status);
                s.sent += 1;
                ctx.request_repaint();
            }
            Err(error) => send_errors(&mut d.errors, &error),
        }
    }
}

/// The dialog, while open; call once per frame from the shell.
pub fn show(ctx: &egui::Context, app: &mut AppCtx) {
    let unread = super::unread(ctx, app);
    let state = super::shared(ctx);
    let mut s = state.lock();
    take(ctx, app, &mut s);
    if !s.dialog.open {
        return;
    }
    let State { dialog: d, .. } = &mut *s;
    if d.sending.is_none()
        && d.reading.is_none()
        && let Some(controls::ImageFile::Path(path)) = controls::dropped_image(ctx)
    {
        d.reading = Some(Pending::thread(
            ctx,
            move || read(Source::Path(path)),
            || Err(Refused::Unreadable),
        ));
    }
    let mut action = Action::None;
    let output = Modal::new("feedback-dialog", texts::dialog_title())
        .width(ModalWidth::Wide)
        .show(ctx, |ui| action = body(ui, d, unread), None);
    let close = output.close_requested || matches!(action, Action::Cancel | Action::View);
    match action {
        Action::Send if d.sending.is_none() => send(app, d),
        Action::Pick if d.picking.is_none() => {
            d.picking = Some(controls::pick_file(ctx, texts::image_attach(), &FILTER));
        }
        _ => {}
    }
    if close {
        d.open = false;
    }
    drop(s);
    if matches!(action, Action::View) {
        app.nav.navigate("/settings/feedback");
    }
}

/// "Senden": a text is required; the image goes up first, then the entry
/// is created.
fn send(app: &AppCtx, d: &mut Dialog) {
    if d.draft.text.trim().is_empty() {
        d.errors
            .set_fields(vec![("text", texts::text_required().to_owned())]);
        return;
    }
    d.errors = FormErrors::default();
    let http = app.client.http().clone();
    let mut body = NewFeedback {
        kind: d.draft.kind(),
        text: d.draft.text.trim().to_owned(),
        image: None,
        context: FeedbackContext::desktop(&d.path),
    };
    let image = d
        .draft
        .image
        .as_ref()
        .map(|a| (a.content_type, a.bytes.clone()));
    d.sending = Some(Pending::spawn(app.client, async move {
        if let Some((content_type, bytes)) = image {
            body.image = Some(http.upload_feedback_image(content_type, bytes).await?.image);
        }
        http.create_feedback(&body).await
    }));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    Cancel,
    Send,
    Pick,
    /// "Ansehen" in the unread line: Einstellungen → Feedback.
    View,
}

fn label(ui: &mut Ui, s: &str) {
    let theme = Theme::of(ui.ctx());
    let width = ui.available_width();
    let block = Block::single(ui, s, text::UI, theme.c.text);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
    block.paint(ui.painter(), rect.min);
}

/// A line of meta text (`.ui-hint`, `.ui-field-error`).
fn meta(ui: &mut Ui, s: &str, color: Color32, role: egui::Role) {
    let width = ui.available_width();
    let block = Block::wrapped(ui, s, text::META, color, width);
    let (rect, response) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
    block.paint(ui.painter(), rect.min);
    crate::widgets::announce(&response, role, s);
}

/// The 8px accent dot (`.unread-dot`) centred on `center`.
pub fn dot(painter: &egui::Painter, center: Pos2, color: Color32) {
    painter.circle_filled(center, 4.0, color);
}

fn body(ui: &mut Ui, d: &mut Dialog, unread: u32) -> Action {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let pending = d.sending.is_some();
    let mut action = Action::None;
    // `.feedback-form`: a column with 16px gaps.
    ui.spacing_mut().item_spacing.y = space::S4;
    let width = ui.available_width();

    // The lead.
    let lead = Block::wrapped(ui, texts::dialog_description(), text::BODY, c.muted, width);
    let (rect, response) = ui.allocate_exact_size(vec2(width, lead.height()), Sense::hover());
    lead.paint(ui.painter(), rect.min);
    crate::widgets::announce(&response, egui::Role::Label, texts::dialog_description());

    // "Es gibt eine neue Antwort auf dein Feedback. Ansehen"
    if unread > 0 {
        let mut rich = text::Rich::new(text::BODY);
        let mut link_at = None;
        let mut link_label = "";
        for (i, piece) in gelabber_i18n::pieces(texts::dialog_unread())
            .into_iter()
            .enumerate()
        {
            if piece.tag == Some("link") {
                link_at = Some(i);
                link_label = piece.text;
                rich = rich.push(piece.text, text::BODY, c.accent_text);
            } else {
                rich = rich.push(piece.text, text::BODY, c.text);
            }
        }
        let block = rich.wrap(width - 16.0).layout(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(width, block.height()), Sense::hover());
        dot(
            ui.painter(),
            pos2(rect.left() + 4.0, rect.top() + block.height() / 2.0),
            c.accent,
        );
        let at = pos2(rect.left() + 16.0, rect.top());
        block.paint(ui.painter(), at);
        if let Some(i) = link_at
            && let Some(link) = block.section_rect(at, i)
        {
            // The browser's underline, just under the baseline.
            crate::widgets::link::underline(
                ui.painter(),
                text::BODY,
                link.top() - 1.0,
                link.left(),
                link.right(),
                c.accent_text,
            );
            let id = egui::Id::unique("feedback-unread-link");
            if Link::interact(ui, link, id, link_label).clicked() {
                action = Action::View;
            }
        }
    }

    // "Worum geht es?" and the chips, 8px apart.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = space::S2;
        label(ui, texts::kind_label());
        let labels = FeedbackKind::ALL.map(kind_label);
        let selected = FeedbackKind::ALL
            .iter()
            .position(|k| *k == d.draft.kind())
            .unwrap_or(0);
        if let Some(i) = controls::chips(ui, "feedback-kind", &labels, selected, 32.0, None)
            && !pending
        {
            d.draft.kind = Some(FeedbackKind::ALL[i]);
        }
    });

    // "Deine Nachricht": the label, 6px, the textarea; the error under it.
    let text_error = d.errors.field("text").map(str::to_owned);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = space::S1_5;
        label(ui, texts::text_label());
        let placeholder = match d.draft.kind() {
            FeedbackKind::Idea => texts::text_placeholder_idea(),
            FeedbackKind::Problem => texts::text_placeholder_problem(),
            FeedbackKind::Praise => texts::text_placeholder_praise(),
        };
        let response = controls::textarea_with(
            ui,
            "feedback-text",
            texts::text_label(),
            &mut d.draft.text,
            TEXT_HEIGHT,
            text_error.is_some(),
            TextareaExtras {
                placeholder: Some(placeholder),
                char_limit: Some(FEEDBACK_TEXT_MAX),
                read_only: pending,
            },
        );
        if response.changed() {
            d.errors.clear_field("text");
        }
        if let Some(error) = &text_error {
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(error.as_str()));
            meta(ui, error, c.danger, egui::Role::Alert);
        }
    });
    // "Noch {count} Zeichen", 4px under the field.
    let typed = d.draft.text.chars().count();
    if typed >= COUNTER_FROM {
        ui.add_space(-space::S3);
        let left = FEEDBACK_TEXT_MAX.saturating_sub(typed) as u64;
        meta(ui, &texts::text_left(left), c.muted, egui::Role::Label);
    }

    // The image: the thumbnail, "Bild anhängen", the hint or the error.
    let image_error = d.errors.field("image").map(str::to_owned);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = space::S2;
        if let Some(image) = d.draft.image.as_mut()
            && thumbnail(ui, image, pending)
        {
            d.draft.image = None;
            d.note = None;
            d.errors.clear_field("image");
        }
        let button = Button::new(texts::image_attach())
            .small()
            .icon(Icon::Paperclip)
            .disabled(pending);
        let w = button.natural_width(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
        let mut slot = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let pressed = slot.add(button.width(w));
        let note = image_error.as_deref().unwrap_or(texts::image_hint());
        ui.ctx()
            .accesskit_node_builder(pressed.id, |node| node.set_description(note));
        if pressed.clicked() && !pending {
            action = Action::Pick;
        }
        match &image_error {
            Some(error) => meta(ui, error, c.danger, egui::Role::Alert),
            None => meta(ui, texts::image_hint(), c.muted, egui::Role::Label),
        }
        if let Some(note) = d.note {
            // `role="status"`: read out, not shown.
            let id = egui::Id::unique("feedback-image-note");
            let response = ui.interact(Rect::NOTHING, id, Sense::hover());
            crate::widgets::announce(&response, egui::Role::Status, note);
        }
    });

    sent_along(ui, d);

    if let Some(message) = &d.errors.form {
        ui.add(FormError::new(message).on(c.elevated));
    }

    // "Abbrechen" and "Senden", right-aligned.
    let cooldown = d.errors.cooldown();
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
                action = Action::Send;
            }
            if ui.add(Button::new(common::cancel()).ghost()).clicked() {
                action = Action::Cancel;
            }
        },
    );
    action
}

fn kind_label(kind: FeedbackKind) -> &'static str {
    match kind {
        FeedbackKind::Idea => texts::kind_idea(),
        FeedbackKind::Problem => texts::kind_problem(),
        FeedbackKind::Praise => texts::kind_praise(),
    }
}

/// `.feedback-form__thumb`: the image at most 160px high in a bordered box,
/// "Bild entfernen" in its top right corner. True when removed.
fn thumbnail(ui: &mut Ui, image: &mut Attached, pending: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let max_w = ui.available_width() - 2.0;
    if image.texture.is_none()
        && let Some(preview) = image.preview.take()
    {
        image.texture = Some(ui.ctx().load_texture(
            "feedback-thumb",
            preview,
            egui::TextureOptions::LINEAR,
        ));
    }
    let shown = match &image.texture {
        Some(_) if image.natural.x > 0.0 && image.natural.y > 0.0 => {
            let size = image.natural;
            let scale = (THUMB_MAX_H / size.y).min(max_w / size.x).min(1.0);
            size * scale
        }
        // It does not decode here: the box keeps the remove button.
        _ => vec2(120.0, 80.0),
    };
    let (rect, response) = ui.allocate_exact_size(shown + vec2(2.0, 2.0), Sense::hover());
    crate::widgets::announce(&response, egui::Role::Image, texts::image_alt());
    let painter = ui.painter();
    painter.rect_filled(rect, corner(theme.radius.md), c.surface);
    if let Some(texture) = &image.texture {
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
    let at = Rect::from_min_size(
        pos2(rect.right() - 4.0 - 32.0, rect.top() + 4.0),
        vec2(32.0, 32.0),
    );
    let remove = ui.add(
        IconButton::new(Icon::Close, texts::image_remove())
            .size(32.0)
            .secondary()
            .disabled(pending)
            .at(at),
    );
    remove.clicked() && !pending
}

/// `.feedback-context`: "Wird mitgeschickt" with exactly the values that go
/// along, open until closed.
fn sent_along(ui: &mut Ui, d: &mut Dialog) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let context = FeedbackContext::desktop(&d.path);
    let system = crate::screens::settings::security::os_name(context.os)
        .unwrap_or(texts::context_unknown_system());
    let rows: [(&str, &str); 5] = [
        (texts::context_app(), texts::context_desktop_app()),
        (texts::context_system(), system),
        (texts::context_version(), &context.app_version),
        (
            texts::context_language(),
            language::option(gelabber_i18n::get()),
        ),
        (texts::context_page(), &context.page),
    ];
    let width = ui.available_width();
    let inner_w = width - 2.0 - 24.0;
    let summary = Block::single(ui, texts::context_title(), text::UI, c.text);
    let open = !d.details_closed;
    // The rows: `dt` as wide as the widest, 12px, `dd`; 4px between rows.
    let dts: Vec<Block> = rows
        .iter()
        .map(|(dt, _)| Block::single(ui, dt, text::META, c.muted))
        .collect();
    let dt_w = dts.iter().map(Block::width).fold(0.0, f32::max);
    let dd_w = inner_w - dt_w - space::S3;
    let dds: Vec<Block> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, dd))| {
            if i == 4 {
                code(ui, dd, c.text, dd_w)
            } else {
                Block::wrapped(ui, dd, text::META, c.text, dd_w)
            }
        })
        .collect();
    let nothing = Block::wrapped(
        ui,
        texts::context_nothing_else(),
        text::META,
        c.muted,
        inner_w,
    );
    let rows_h: f32 = dts
        .iter()
        .zip(&dds)
        .map(|(dt, dd)| dt.height().max(dd.height()))
        .sum::<f32>()
        + space::S1 * (rows.len() - 1) as f32;
    let body_h = if open {
        space::S2 + rows_h + space::S2 + nothing.height()
    } else {
        0.0
    };
    let height = 1.0 + space::S2 + summary.height() + body_h + space::S2 + 1.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_stroke(
        rect,
        corner(theme.radius.md),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let left = rect.left() + 1.0 + space::S3;
    let mut y = rect.top() + 1.0 + space::S2;
    // The summary: the disclosure triangle, then the title.
    let marker_w = 14.0;
    let summary_rect = Rect::from_min_size(
        pos2(left, y),
        vec2(marker_w + summary.width(), summary.height()),
    );
    let toggle = ui.interact(
        summary_rect,
        egui::Id::unique("feedback-sent-along"),
        Sense::click(),
    );
    toggle.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::DisclosureTriangle,
            true,
            open,
            texts::context_title(),
        )
    });
    if toggle.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if toggle.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(&painter, summary_rect, 2.0, &theme);
    }
    triangle(&painter, pos2(left, y), summary.height(), open, c.text);
    summary.paint(&painter, pos2(left + marker_w, y));
    if toggle.clicked() {
        d.details_closed = open;
    }
    y += summary.height();
    if open {
        y += space::S2;
        for (i, ((dt, dd), (term, value))) in dts.iter().zip(&dds).zip(rows).enumerate() {
            let at = pos2(left + dt_w + space::S3, y);
            dt.paint(&painter, pos2(left, y));
            dd.paint(&painter, at);
            // The `<dl>` is read out: each term and its value.
            for (j, (block, s, min)) in [(dt, term, pos2(left, y)), (dd, value, at)]
                .into_iter()
                .enumerate()
            {
                let id = egui::Id::unique(("feedback-sent-along-row", i, j));
                let node = ui.interact(Rect::from_min_size(min, block.size()), id, Sense::hover());
                crate::widgets::announce(&node, egui::Role::Label, s);
            }
            y += dt.height().max(dd.height()) + space::S1;
        }
        y += space::S2 - space::S1;
        nothing.paint(&painter, pos2(left, y));
        let node = ui.interact(
            Rect::from_min_size(pos2(left, y), nothing.size()),
            egui::Id::unique("feedback-sent-along-end"),
            Sense::hover(),
        );
        crate::widgets::announce(&node, egui::Role::Label, texts::context_nothing_else());
    }
}

/// Chromium's `▼` / `▶` marker in front of a summary of `line` height.
pub(crate) fn triangle(painter: &egui::Painter, min: Pos2, line: f32, open: bool, color: Color32) {
    let cy = min.y + line / 2.0;
    let x = min.x + 1.0;
    let points = if open {
        vec![
            pos2(x, cy - 3.5),
            pos2(x + 8.0, cy - 3.5),
            pos2(x + 4.0, cy + 3.5),
        ]
    } else {
        vec![
            pos2(x + 1.0, cy - 4.0),
            pos2(x + 8.0, cy),
            pos2(x + 1.0, cy + 4.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// `<code>` in the list: the system's monospace font at the list's 12px.
fn code(ui: &Ui, s: &str, color: Color32, width: f32) -> Block {
    let style = TextStyle::new(12.0, 400.0, 1.4);
    let mut format = style.format(color);
    format.font_id = egui::FontId::new(12.0, crate::fonts::mono_family());
    let mut job = egui::text::LayoutJob::single_section(s.to_owned(), format);
    job.wrap.max_width = width;
    job.wrap.break_anywhere = true;
    Block::new(ui, job, style)
}
