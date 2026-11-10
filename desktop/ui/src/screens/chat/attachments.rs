//! What a message carries besides text (components/Attachments.tsx,
//! imageBox.ts, download.ts, GifPicker.tsx `GifMessageButton`,
//! LinkPreviews.tsx; chat.css `.lr-attachments`, `.lr-gif`, `.lr-preview`):
//! pictures (or the box saying one did not load), text files as a preview
//! to open, every other file as a download, GIFs and link previews, each
//! sized before its image arrives. Video and audio have no player here:
//! they are downloads, as on the web where the browser cannot play them.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_client::store::UploadState;
use gelabber_client::{Attachment, Gif, LinkPreview};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::icons::{self, Icon};
use crate::images::{Fit, ImageState, Images, TextState, cover_uv};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};

const IMAGE_BOX_W: f32 = 360.0;
const IMAGE_BOX_H: f32 = 240.0;
/// `TEXT_PREVIEW_MAX`: larger text files are plain downloads.
const TEXT_PREVIEW_MAX: i64 = 100 * 1024;
/// `DOWNLOAD_STARTED_MS`.
const DOWNLOAD_STARTED: f64 = 4.0;
/// `.lr-attachment-file` and the text summary: padding 7, a 13px line.
const FILE_H: f32 = 35.5;
const FILE_STYLE: TextStyle = TextStyle::new(13.0, 600.0, 1.5);
const SIZE_STYLE: TextStyle = TextStyle::new(11.0, 400.0, 1.5);
const STATE_STYLE: TextStyle = TextStyle::new(12.0, 400.0, 1.5);
/// `line-height: 1.45` on the `<pre>`; the `<code>` in it, in another monospace
/// font, makes each line 19.2px in Chromium.
const PRE_STYLE: TextStyle = TextStyle::new(12.5, 400.0, 19.2 / 12.5);

pub fn is_image(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

/// `isTextType`.
pub fn is_text(content_type: &str) -> bool {
    content_type.starts_with("text/") || content_type == "application/json"
}

/// `formatSize`: "1,5 MB" or "12 KB".
pub fn format_size(bytes: i64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)).replace('.', ",")
    } else {
        format!("{} KB", ((bytes as f64 / 1024.0).round() as i64).max(1))
    }
}

pub fn attachment_url(id: Uuid) -> String {
    format!("/api/attachments/{id}")
}

/// `attachmentDownloadUrl`.
pub fn download_url(id: Uuid) -> String {
    format!("{}?download=1", attachment_url(id))
}

/// The upload state's words (`UploadState`), none once it arrived.
pub fn upload_text(state: Option<UploadState>) -> Option<&'static str> {
    match state? {
        UploadState::Uploading => Some("Wird hochgeladen…"),
        UploadState::Waiting => Some("Wartet"),
        UploadState::Done => None,
    }
}

/// What a click on a piece asks for.
pub enum MediaAction {
    /// Open a picture large (a URL the image cache knows).
    Open(String, String),
    /// Open in the browser (a link preview).
    External(String),
    /// Fetch through the app's session and save where the user says, as `name`.
    Download { url: String, name: String },
}

impl MediaAction {
    fn download(att: &Attachment) -> Self {
        Self::Download {
            url: download_url(att.id),
            name: att.filename.clone(),
        }
    }
}

/// A file name from the server as the save dialog's suggestion: no path,
/// nothing Windows refuses, never empty.
pub fn safe_file_name(name: &str) -> String {
    // 200 bytes, not characters: file systems count bytes (255 at most),
    // and " (2)" may still follow.
    let mut bytes = 0;
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 200
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim_start_matches('.');
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() {
        "Download".to_owned()
    } else if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_owned()
    }
}

type DownloadErrors = Arc<Mutex<Vec<String>>>;

fn download_errors(ctx: &egui::Context) -> DownloadErrors {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<DownloadErrors>(egui::Id::unique("gelabber-download-errors"))
            .clone()
    })
}

/// Fetches `url` with the app's session (a browser would have none) and
/// asks where to save it, suggesting `name`; a failed fetch is a toast.
pub fn download(ctx: &egui::Context, client: &gelabber_client::Client, url: &str, name: &str) {
    let ctx = ctx.clone();
    let http = client.http().clone();
    let (url, name) = (url.to_owned(), safe_file_name(name));
    client.spawn(async move {
        match http.bytes(&url, gelabber_client::Download::File).await {
            Ok(bytes) => crate::platform::files::save(&ctx, "Datei speichern", &name, bytes),
            Err(e) => {
                tracing::warn!("download {url}: {e}");
                // A lost session sends the app to the login page instead.
                if let Some(text) = crate::toasts::error_text(&e) {
                    download_errors(&ctx)
                        .lock()
                        .push(format!("Datei nicht heruntergeladen. {text}"));
                }
                ctx.request_repaint();
            }
        }
    });
}

/// Downloads that failed since the last call, for the toasts.
pub fn take_download_errors(ctx: &egui::Context) -> Vec<String> {
    std::mem::take(&mut *download_errors(ctx).lock())
}

/// What the attachments remember between frames: open text previews, and
/// when a download link was last pressed.
#[derive(Default)]
struct Shown {
    open: HashMap<Uuid, bool>,
    started: HashMap<Uuid, f64>,
    /// GIF messages the reader started or stopped (`chosen`).
    gifs: HashMap<Uuid, bool>,
    /// How far each open text preview is scrolled.
    scroll: HashMap<Uuid, Vec2>,
    /// The text preview under the pointer and the frame it was drawn in.
    wheel: Option<(Rect, bool, bool, u64)>,
}

/// Whether a text preview under `pointer` takes a wheel of `delta_y`:
/// it scrolls until its end, then the list does (scroll chaining).
pub fn takes_wheel(ctx: &egui::Context, pointer: Pos2, delta_y: f32) -> bool {
    let frame = ctx.cumulative_frame_nr();
    shown(ctx).lock().wheel.is_some_and(|(rect, up, down, f)| {
        f + 1 >= frame
            && rect.contains(pointer)
            && ((delta_y > 0.0 && up) || (delta_y < 0.0 && down))
    })
}

fn shown(ctx: &egui::Context) -> Arc<Mutex<Shown>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("gelabber-attachments"), || {
            Arc::new(Mutex::new(Shown::default()))
        })
        .clone()
    })
}

/// `useDownload`: whether the link of `id` says "Download gestartet".
fn download_started(ctx: &egui::Context, id: Uuid) -> bool {
    let now = ctx.input(|i| i.time);
    let at = shown(ctx).lock().started.get(&id).copied();
    match at {
        Some(at) if now - at < DOWNLOAD_STARTED => {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                DOWNLOAD_STARTED - (now - at),
            ));
            true
        }
        _ => false,
    }
}

/// A press on a download link: it starts once, then rests for 4s.
fn start_download(ctx: &egui::Context, id: Uuid) -> bool {
    if download_started(ctx, id) {
        return false;
    }
    let now = ctx.input(|i| i.time);
    shown(ctx).lock().started.insert(id, now);
    true
}

enum Piece {
    /// A single picture: its box (border included).
    Image {
        att: Attachment,
        url: String,
        size: Vec2,
        upload: Option<UploadState>,
    },
    Grid {
        cells: Vec<(Attachment, String)>,
    },
    /// A picture that did not load (`AttachmentImageFailed`).
    Failed {
        att: Attachment,
        size: Vec2,
    },
    File(Box<FilePiece>),
    Text(Box<TextPiece>),
}

struct FilePiece {
    att: Attachment,
    name: Block,
    size: Block,
    upload: Option<(Block, &'static str)>,
    started: bool,
    width: f32,
    /// The link's name: what it shows.
    label: String,
}

struct TextPiece {
    att: Attachment,
    name: Block,
    size: Block,
    upload: Option<(Block, &'static str)>,
    width: f32,
    open: bool,
    body: TextBody,
}

enum TextBody {
    Closed,
    /// "Vorschau wird geladen…" or "Vorschau nicht verfügbar." with the link.
    State(Block, bool),
    /// The text (at most 320 high, scrolling) and "Herunterladen".
    Text(Block, f32),
}

/// Laid-out attachments: pieces at y offsets inside a column.
pub struct Media {
    pieces: Vec<(f32, Piece)>,
    pub height: f32,
}

/// The box of a single picture: `imageFit` plus the 1px border.
pub fn image_box(natural: Option<Vec2>, width: f32) -> Vec2 {
    match natural {
        Some(n) if n.x > 0.0 && n.y > 0.0 => {
            let scale = 1f32
                .min((IMAGE_BOX_W - 2.0) / n.x)
                .min((IMAGE_BOX_H - 2.0) / n.y)
                .min((width - 2.0).max(1.0) / n.x);
            vec2((n.x * scale + 2.0).max(24.0), (n.y * scale + 2.0).max(24.0))
        }
        _ => vec2(IMAGE_BOX_W.min(width), IMAGE_BOX_H),
    }
}

fn natural(ui: &Ui, images: &Images, url: &str) -> Option<Vec2> {
    images.natural(ui.ctx(), url, Fit::Inline)
}

/// A picture that failed, not one still on its way up (`isPendingId`).
fn failed(ui: &Ui, images: &Images, att: &Attachment, url: &str) -> bool {
    !att.id.is_nil() && images.failed(ui.ctx(), url, Fit::Inline)
}

/// Lays out `attachments` in a column `width` wide; `uploads` are the
/// files' states while my message is on its way.
pub fn layout(
    ui: &Ui,
    images: &Images,
    attachments: &[Attachment],
    width: f32,
    uploads: Option<&[UploadState]>,
) -> Option<Media> {
    if attachments.is_empty() {
        return None;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let upload_of = |a: &Attachment| {
        let i = attachments.iter().position(|x| std::ptr::eq(x, a))?;
        uploads.and_then(|u| u.get(i).copied())
    };
    let pictures: Vec<&Attachment> = attachments
        .iter()
        .filter(|a| is_image(&a.content_type))
        .collect();
    let mut pieces = Vec::new();
    let mut y = 0.0;
    let mut push = |pieces: &mut Vec<(f32, Piece)>, h: f32, piece: Piece| {
        if !pieces.is_empty() {
            y += 6.0;
        }
        pieces.push((y, piece));
        y += h;
    };
    if pictures.len() == 1 {
        let a = pictures[0];
        let url = attachment_url(a.id);
        if failed(ui, images, a, &url) {
            // The box of a picture still loading: 240 wide, 240 high.
            let size = vec2(240.0_f32.min(width), IMAGE_BOX_H);
            push(
                &mut pieces,
                size.y,
                Piece::Failed {
                    att: a.clone(),
                    size,
                },
            );
        } else {
            let size = image_box(natural(ui, images, &url), width);
            push(
                &mut pieces,
                size.y,
                Piece::Image {
                    att: a.clone(),
                    url,
                    size,
                    upload: upload_of(a),
                },
            );
        }
    } else if pictures.len() > 1 {
        let rows = pictures.len().div_ceil(2) as f32;
        let cells = pictures
            .iter()
            .map(|a| ((*a).clone(), attachment_url(a.id)))
            .collect();
        push(
            &mut pieces,
            rows * 160.0 + (rows - 1.0) * 4.0,
            Piece::Grid { cells },
        );
    }
    let state = shown(ui.ctx());
    for a in attachments.iter().filter(|a| !is_image(&a.content_type)) {
        let upload =
            upload_text(upload_of(a)).map(|t| (Block::single(ui, t, SIZE_STYLE, c.muted), t));
        let name = Block::single(ui, &a.filename, FILE_STYLE, c.text);
        if is_text(&a.content_type) && a.size <= TEXT_PREVIEW_MAX {
            let size = Block::single(ui, &format_size(a.size), SIZE_STYLE, c.muted);
            let open = state.lock().open.get(&a.id).copied().unwrap_or(false);
            let width = width.min(560.0);
            let inner = width - 2.0 - 24.0;
            let body = if !open {
                TextBody::Closed
            } else {
                match (a.id.is_nil(), images.text(ui.ctx(), &attachment_url(a.id))) {
                    (false, TextState::Ready(text)) => {
                        let mut format = PRE_STYLE.format(c.text);
                        format.font_id = egui::FontId::new(12.5, crate::fonts::mono_family());
                        format.line_height = Some(PRE_STYLE.line_px());
                        let mut job = egui::text::LayoutJob::single_section(
                            text.trim_end_matches('\n').to_owned(),
                            format,
                        );
                        job.wrap.max_width = f32::INFINITY;
                        let block = Block::new(ui, job, PRE_STYLE);
                        let shown_h = block.height().min(320.0 - 20.0);
                        TextBody::Text(block, shown_h)
                    }
                    (false, TextState::Failed) => TextBody::State(
                        Block::wrapped(
                            ui,
                            "Vorschau nicht verfügbar.",
                            STATE_STYLE,
                            c.muted,
                            inner,
                        ),
                        true,
                    ),
                    _ => TextBody::State(
                        Block::wrapped(ui, "Vorschau wird geladen…", STATE_STYLE, c.muted, inner),
                        false,
                    ),
                }
            };
            let body_h = match &body {
                TextBody::Closed => 0.0,
                TextBody::State(block, _) => 6.0 + block.height() + 8.0,
                TextBody::Text(_, shown_h) => 1.0 + 10.0 + shown_h + 10.0 + 6.0 + 18.0 + 8.0,
            };
            push(
                &mut pieces,
                FILE_H + body_h,
                Piece::Text(Box::new(TextPiece {
                    att: a.clone(),
                    name,
                    size,
                    upload,
                    width,
                    open,
                    body,
                })),
            );
            continue;
        }
        let started = download_started(ui.ctx(), a.id);
        // With an upload state last, the size is a plain 13px span.
        let size = if started {
            Block::single(ui, "Download gestartet", SIZE_STYLE, c.muted)
        } else if upload.is_some() {
            Block::single(ui, &format_size(a.size), FILE_STYLE.weight(400.0), c.text)
        } else {
            Block::single(ui, &format_size(a.size), SIZE_STYLE, c.muted)
        };
        let upload_w = upload.as_ref().map_or(0.0, |(u, _)| 8.0 + u.width());
        let w = (10.0 + 15.0 + 8.0 + name.width() + 8.0 + size.width() + upload_w + 12.0 + 2.0)
            .min(width);
        push(
            &mut pieces,
            FILE_H,
            Piece::File(Box::new(FilePiece {
                att: a.clone(),
                name,
                size,
                upload,
                started,
                width: w,
                label: format!(
                    "{} {}",
                    a.filename,
                    if started {
                        "Download gestartet".to_owned()
                    } else {
                        format_size(a.size)
                    }
                ),
            })),
        );
    }
    Some(Media { pieces, height: y })
}

fn paint_image(ui: &Ui, images: &Images, url: &str, rect: Rect, cover: bool, play: bool) {
    let theme = Theme::of(ui.ctx());
    let painter = ui.painter();
    if let ImageState::Ready(texture) = images.frame(ui.ctx(), url, play, Fit::Inline) {
        let inner = rect.shrink(1.0);
        let uv = if cover {
            cover_uv(texture.size_vec2(), inner.size())
        } else {
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
        };
        painter.add(
            egui::epaint::RectShape::filled(inner, CornerRadius::same(6), Color32::WHITE)
                .with_texture(texture.id(), uv),
        );
    }
    painter.rect_stroke(
        rect,
        corner(7.0),
        Stroke::new(1.0, theme.c.border),
        StrokeKind::Inside,
    );
}

/// The global `:focus-visible` ring around a picture link, for example
/// once the lightbox it opened hands focus back: square, as the link box
/// has no radius of its own.
fn picture_ring(ui: &Ui, response: &egui::Response, rect: Rect) {
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        let theme = Theme::of(ui.ctx());
        let painter = ui.painter();
        painter.rect_stroke(
            rect.expand(1.0),
            0.0,
            Stroke::new(2.0, theme.c.bg),
            StrokeKind::Middle,
        );
        painter.rect_stroke(
            rect.expand(3.0),
            0.0,
            Stroke::new(2.0, theme.c.focus),
            StrokeKind::Middle,
        );
    }
}

/// The browser animates pictures unless motion is reduced.
fn animate(ui: &Ui) -> bool {
    !crate::screens::settings::prefs::get(ui.ctx()).reduced_motion
}

/// A 1px dashed rounded box (`border: 1px dashed`), dashes 3 and gaps 3.
fn dashed(painter: &egui::Painter, rect: Rect, color: Color32) {
    let r = rect.shrink(0.5);
    let stroke = Stroke::new(1.0, color);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ];
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let dir = (b - a).normalized();
        // Leave the rounded corners to the arcs.
        let (a, b) = (a + dir * 7.0, b - dir * 7.0);
        painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 3.0, 3.0));
    }
    for (center, start) in [
        (r.left_top() + vec2(7.0, 7.0), std::f32::consts::PI),
        (r.right_top() + vec2(-7.0, 7.0), 1.5 * std::f32::consts::PI),
        (r.right_bottom() + vec2(-7.0, -7.0), 0.0),
        (
            r.left_bottom() + vec2(7.0, -7.0),
            0.5 * std::f32::consts::PI,
        ),
    ] {
        let points: Vec<Pos2> = (0..=6)
            .map(|k| {
                let a = start + k as f32 / 6.0 * std::f32::consts::FRAC_PI_2;
                center + vec2(a.cos(), a.sin()) * 7.0
            })
            .collect();
        painter.add(egui::Shape::line(points, stroke));
    }
}

/// An underlined 12px text button in the accent text colour
/// (`.lr-attachment-image-actions > button, > a`).
fn text_action(
    ui: &mut Ui,
    at: Pos2,
    label: &str,
    role: egui::Role,
    id: egui::Id,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, label, STATE_STYLE, c.accent_text);
    let rect = Rect::from_min_size(at, block.size());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(role, true, label));
    block.paint(ui.painter(), rect.min);
    crate::widgets::link::underline(
        ui.painter(),
        STATE_STYLE,
        rect.top(),
        rect.left(),
        rect.right(),
        c.accent_text,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// `AttachmentImageFailed`: name, "Bild konnte nicht geladen werden." (or
/// why it never shows), "Erneut laden" and "Herunterladen", centred in a
/// dashed box.
fn paint_failed(
    ui: &mut Ui,
    images: &Images,
    att: &Attachment,
    rect: Rect,
    id: egui::Id,
) -> Option<MediaAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.panel);
    dashed(ui.painter(), rect, c.border);
    let inner = rect.shrink(12.0 + 1.0);
    let mut name_job = crate::text::job(
        &att.filename,
        TextStyle::new(13.0, 400.0, 1.5),
        c.text,
        Some(inner.width()),
    );
    name_job.wrap.max_rows = 1;
    let name = Block::new(ui, name_job, TextStyle::new(13.0, 400.0, 1.5));
    let said = if images.too_large(&attachment_url(att.id), Fit::Inline) {
        "Bild ist zu groß zum Anzeigen."
    } else {
        "Bild konnte nicht geladen werden."
    };
    let status = Block::wrapped(ui, said, STATE_STYLE, c.muted, inner.width());
    let actions_h = STATE_STYLE.line_px();
    let total = 15.0 + 6.0 + name.height() + 6.0 + status.height() + 6.0 + actions_h;
    let mut y = rect.center().y - total / 2.0;
    icons::paint(
        ui.painter(),
        Icon::Paperclip,
        pos2(inner.left(), y),
        15.0,
        c.accent_text,
    );
    y += 15.0 + 6.0;
    name.paint(ui.painter(), pos2(inner.left(), y));
    y += name.height() + 6.0;
    status.paint(ui.painter(), pos2(inner.left(), y));
    self::status(
        ui,
        Rect::from_min_size(pos2(inner.left(), y), status.size()),
        said,
    );
    y += status.height() + 6.0;
    let mut action = None;
    let retry = text_action(
        ui,
        pos2(inner.left(), y),
        "Erneut laden",
        egui::Role::Button,
        id.with("retry"),
    );
    if retry.clicked() {
        images.retry(&attachment_url(att.id));
        ui.ctx().request_repaint();
    }
    let download = text_action(
        ui,
        pos2(retry.rect.right() + 12.0, y),
        if download_started(ui.ctx(), att.id) {
            "Download gestartet"
        } else {
            "Herunterladen"
        },
        egui::Role::Link,
        id.with("download"),
    );
    if download.clicked() && start_download(ui.ctx(), att.id) {
        action = Some(MediaAction::download(att));
    }
    action
}

/// The upload state over a picture's bottom-left corner.
fn paint_upload_badge(ui: &Ui, rect: Rect, text: &str) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, text, SIZE_STYLE, c.muted);
    let badge = Rect::from_min_size(
        pos2(
            rect.left() + 6.0,
            rect.bottom() - 6.0 - block.height() - 4.0,
        ),
        vec2(block.width() + 12.0, block.height() + 4.0),
    );
    ui.painter()
        .rect_filled(badge, corner(theme.radius.xs), c.panel.gamma_multiply(0.85));
    block.paint(ui.painter(), pos2(badge.left() + 6.0, badge.top() + 2.0));
    status(ui, badge, text);
}

/// `role="status"`: the words are read out (and found by tests).
fn status(ui: &Ui, rect: Rect, text: &str) {
    let id = egui::Id::unique((
        "gelabber-attachment-status",
        text,
        rect.min.x as i32,
        rect.min.y as i32,
    ));
    ui.interact(rect, id, Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
}

/// Icon, name and size (and the upload state) of a file line.
fn paint_file_line(
    ui: &Ui,
    rect: Rect,
    icon: Icon,
    name: &Block,
    size: &Block,
    upload: Option<&(Block, &'static str)>,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 11.0, rect.top() + FILE_H / 2.0 - 7.5),
        15.0,
        c.accent_text,
    );
    let x = rect.left() + 11.0 + 15.0 + 8.0;
    let clip = ui
        .painter()
        .with_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
    let mid = rect.top() + FILE_H / 2.0;
    name.paint(&clip, pos2(x, mid - name.height() / 2.0));
    let sx = x + name.width() + 8.0;
    size.paint(&clip, pos2(sx, mid - size.height() / 2.0 + 0.5));
    if let Some((upload, text)) = upload {
        let at = pos2(sx + size.width() + 8.0, mid - upload.height() / 2.0 + 0.5);
        upload.paint(&clip, at);
        status(ui, Rect::from_min_size(at, upload.size()), text);
    }
}

impl Media {
    /// Paints the attachments; a text file's shown text is selectable,
    /// added to `text`.
    pub fn paint(
        &self,
        ui: &mut Ui,
        images: &Images,
        origin: Pos2,
        id: egui::Id,
        text: &mut Option<egui::Response>,
    ) -> Option<MediaAction> {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let play = animate(ui);
        let mut action = None;
        for (n, (y, piece)) in self.pieces.iter().enumerate() {
            let top = origin.y + y;
            match piece {
                Piece::Image {
                    att,
                    url,
                    size,
                    upload,
                } => {
                    let rect = Rect::from_min_size(pos2(origin.x, top), *size);
                    paint_image(ui, images, url, rect, false, play);
                    if let Some(text) = upload_text(*upload) {
                        paint_upload_badge(ui, rect, text);
                    }
                    let response = ui.interact(rect, id.with(("image", n)), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Link,
                            true,
                            format!("{} vergrößern", att.filename),
                        )
                    });
                    picture_ring(ui, &response, rect);
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
                    }
                    if response.clicked() && !att.id.is_nil() {
                        action = Some(MediaAction::Open(url.clone(), att.filename.clone()));
                    }
                }
                Piece::Failed { att, size } => {
                    let rect = Rect::from_min_size(pos2(origin.x, top), *size);
                    if let Some(a) = paint_failed(ui, images, att, rect, id.with(("failed", n))) {
                        action = Some(a);
                    }
                }
                Piece::Grid { cells } => {
                    for (i, (att, url)) in cells.iter().enumerate() {
                        let col = (i % 2) as f32;
                        let row = (i / 2) as f32;
                        let cell_w = 198.0;
                        let rect = Rect::from_min_size(
                            pos2(origin.x + col * (cell_w + 4.0), top + row * 164.0),
                            vec2(cell_w, 160.0),
                        );
                        if failed(ui, images, att, url) {
                            let cell_id = id.with(("cell-failed", n, i));
                            if let Some(a) = paint_failed(ui, images, att, rect, cell_id) {
                                action = Some(a);
                            }
                            continue;
                        }
                        paint_image(ui, images, url, rect, true, play);
                        let response = ui.interact(rect, id.with(("cell", n, i)), Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Link,
                                true,
                                format!("{} vergrößern", att.filename),
                            )
                        });
                        picture_ring(ui, &response, rect);
                        if response.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ZoomIn);
                        }
                        if response.clicked() && !att.id.is_nil() {
                            action = Some(MediaAction::Open(url.clone(), att.filename.clone()));
                        }
                    }
                }
                Piece::File(file) => {
                    let FilePiece {
                        att,
                        name,
                        size,
                        upload,
                        started,
                        width,
                        label,
                    } = &**file;
                    let rect = Rect::from_min_size(pos2(origin.x, top), vec2(*width, FILE_H));
                    let response = ui.interact(rect, id.with(("file", n)), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Link, true, label.clone())
                    });
                    let fill = if response.hovered() { c.hover } else { c.panel };
                    ui.painter().rect(
                        rect,
                        corner(7.0),
                        fill,
                        Stroke::new(1.0, c.border),
                        StrokeKind::Inside,
                    );
                    let icon = if *started {
                        Icon::Check
                    } else {
                        Icon::Paperclip
                    };
                    paint_file_line(ui, rect, icon, name, size, upload.as_ref());
                    if response.hovered() && !started {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() && !att.id.is_nil() && start_download(ui.ctx(), att.id) {
                        action = Some(MediaAction::download(att));
                    }
                }
                Piece::Text(piece) => {
                    let at = pos2(origin.x, top);
                    if let Some(a) = paint_text(ui, piece, at, id.with(("text", n)), text) {
                        action = Some(a);
                    }
                }
            }
        }
        action
    }
}

/// `AttachmentText`: a `<details>` whose summary is the file line; open,
/// the text (or its state) and "Herunterladen". The file's text is a
/// `<pre>`: selectable, added to `text`.
fn paint_text(
    ui: &mut Ui,
    piece: &TextPiece,
    at: Pos2,
    id: egui::Id,
    text: &mut Option<egui::Response>,
) -> Option<MediaAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let body_h = match &piece.body {
        TextBody::Closed => 0.0,
        TextBody::State(block, _) => 6.0 + block.height() + 8.0,
        TextBody::Text(_, shown_h) => 1.0 + 10.0 + shown_h + 10.0 + 6.0 + 18.0 + 8.0,
    };
    let rect = Rect::from_min_size(at, vec2(piece.width, FILE_H + body_h));
    ui.painter().rect(
        rect,
        corner(7.0),
        c.panel,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let summary = Rect::from_min_size(at, vec2(piece.width, FILE_H));
    let response = ui.interact(summary, id.with("summary"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, &piece.att.filename)
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        shown(ui.ctx())
            .lock()
            .open
            .insert(piece.att.id, !piece.open);
        // A clicked `<summary>` keeps the focus: its row looks hovered.
        response.request_focus();
        ui.ctx().request_repaint();
    }
    paint_file_line(
        ui,
        summary,
        Icon::Paperclip,
        &piece.name,
        &piece.size,
        piece.upload.as_ref(),
    );
    let mut action = None;
    let link = |ui: &mut Ui, pos: Pos2, action: &mut Option<MediaAction>| {
        let started = download_started(ui.ctx(), piece.att.id);
        let label = if started {
            "Download gestartet"
        } else {
            "Herunterladen"
        };
        let block = Block::single(ui, label, STATE_STYLE, c.muted);
        let r = Rect::from_min_size(pos, block.size());
        let response = ui.interact(r, id.with("download"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, label));
        block.paint(ui.painter(), r.min);
        if response.hovered() && !started {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && start_download(ui.ctx(), piece.att.id) {
            *action = Some(MediaAction::download(&piece.att));
        }
    };
    match &piece.body {
        TextBody::Closed => {}
        TextBody::State(block, failed) => {
            let p = pos2(at.x + 1.0 + 12.0, summary.bottom() + 6.0);
            block.paint(ui.painter(), p);
            if *failed {
                // "Vorschau nicht verfügbar. " and the link on its line.
                let space = Block::single(ui, " ", STATE_STYLE, c.muted).width();
                link(ui, pos2(p.x + block.width() + space, p.y), &mut action);
            }
        }
        TextBody::Text(block, shown_h) => {
            let line = summary.bottom();
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(at.x + 1.0, line), vec2(piece.width - 2.0, 1.0)),
                0.0,
                c.border,
            );
            let pre = Rect::from_min_size(
                pos2(at.x + 1.0, line + 1.0),
                vec2(piece.width - 2.0, 10.0 + shown_h + 10.0),
            );
            // The text scrolls both ways inside its box (`overflow: auto`).
            let seen = vec2(pre.width() - 24.0, *shown_h);
            let max = (block.size() - seen).max(Vec2::ZERO);
            let state = shown(ui.ctx());
            let mut offset = state
                .lock()
                .scroll
                .get(&piece.att.id)
                .copied()
                .unwrap_or_default();
            if ui.rect_contains_pointer(pre) {
                let delta = ui.input(|i| i.smooth_scroll_delta);
                let next = (offset - delta).clamp(Vec2::ZERO, max);
                if next != offset {
                    offset = next;
                    ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                }
                state.lock().wheel = Some((
                    pre,
                    offset.y > 0.0,
                    offset.y < max.y,
                    ui.ctx().cumulative_frame_nr(),
                ));
            }
            state.lock().scroll.insert(piece.att.id, offset.min(max));
            let lines = crate::text::selectable_in(
                ui,
                block,
                id.with("preview"),
                pos2(pre.left() + 12.0, pre.top() + 10.0) - offset,
                pre,
            );
            crate::text::join(text, lines);
            link(ui, pos2(at.x + 1.0 + 12.0, pre.bottom() + 6.0), &mut action);
        }
    }
    action
}
/// The picture large over the window (components/ImageLightbox.tsx):
/// filename, "Herunterladen", close; Esc or a click beside it closes.
/// Returns (close, download).
pub fn lightbox(ctx: &egui::Context, images: &Images, url: &str, name: &str) -> (bool, bool) {
    let screen = ctx.content_rect();
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut download = false;
    let area = egui::Area::new(egui::Id::unique("gelabber-lightbox"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min);
    // A modal <dialog>: the chat under it takes no clicks or Tab.
    ctx.memory_mut(|m| m.set_modal_layer(area.layer()));
    crate::widgets::opener::dialog_shown(ctx, area.layer());
    let first = crate::widgets::field::mounted(ctx, egui::Id::unique("gelabber-lightbox-open"));
    area.show(ctx, |ui| {
        let backdrop = ui.allocate_rect(screen, Sense::click());
        ui.painter()
            .rect_filled(screen, 0.0, Color32::from_black_alpha(217));
        let area = Rect::from_min_max(
            pos2(screen.left() + 16.0, screen.top() + 56.0),
            pos2(screen.right() - 16.0, screen.bottom() - 16.0),
        );
        let mut image_rect = None;
        if let ImageState::Ready(t) = images.get(ctx, url) {
            let size = t.size_vec2();
            let scale = (area.width() / size.x).min(area.height() / size.y).min(1.0);
            let r = Rect::from_center_size(area.center(), size * scale);
            ui.painter().add(
                egui::epaint::RectShape::filled(r, CornerRadius::same(7), Color32::WHITE)
                    .with_texture(t.id(), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
            );
            image_rect = Some(r);
        }
        let white = Color32::WHITE;
        let style = TextStyle::new(14.0, 400.0, 1.5);
        let close_rect = Rect::from_min_size(
            pos2(screen.right() - 16.0 - 40.0, screen.top() + 10.0),
            vec2(40.0, 40.0),
        );
        let dl = Block::single(ui, "Herunterladen", style, white);
        let dl_rect = Rect::from_min_size(
            pos2(
                close_rect.left() - 16.0 - dl.width(),
                close_rect.center().y - dl.height() / 2.0,
            ),
            dl.size(),
        );
        let title_w = (dl_rect.left() - 16.0 - screen.left() - 16.0).max(10.0);
        let mut job = crate::text::job(name, style, white, Some(title_w));
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        Block::new(ui, job, style).paint(ui.painter(), pos2(screen.left() + 16.0, dl_rect.top()));
        dl.paint(ui.painter(), dl_rect.min);
        crate::widgets::link::underline(
            ui.painter(),
            style,
            dl_rect.top(),
            dl_rect.left(),
            dl_rect.right(),
            white,
        );
        let dl_resp = ui.interact(
            dl_rect,
            egui::Id::unique("gelabber-lightbox-dl"),
            Sense::click(),
        );
        let close_resp = ui.interact(
            close_rect,
            egui::Id::unique("gelabber-lightbox-close"),
            Sense::click(),
        );
        dl_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, "Herunterladen"));
        close_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Schließen"));
        // The close button has the focus, not the download link.
        if first {
            close_resp.request_focus();
        }
        ui.painter()
            .circle_filled(close_rect.center(), 20.0, Color32::from_white_alpha(31));
        icons::paint_centered(ui.painter(), Icon::Close, close_rect, 18.0, white);
        let keyboard = crate::widgets::keyboard_focus(ui.ctx());
        for (r, focused, radius) in [
            (close_rect, close_resp.has_focus(), 20.0),
            (dl_rect, dl_resp.has_focus(), 3.0),
        ] {
            if focused && keyboard {
                crate::widgets::focus_ring(ui.painter(), r, radius, &Theme::of(ui.ctx()));
            }
        }
        if dl_resp.hovered() || close_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        download = dl_resp.clicked();
        let on_image = ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| image_rect.is_some_and(|r| r.contains(p)));
        if close_resp.clicked() || (backdrop.clicked() && !on_image) {
            close = true;
        }
    });
    (close, download)
}

/// `gifBox`: at most 320×240, rounded.
pub fn gif_box(gif: &Gif) -> Vec2 {
    let (w, h) = (gif.width.max(1) as f32, gif.height.max(1) as f32);
    let scale = 1f32.min(320.0 / w).min(240.0 / h);
    vec2((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// A GIF message (`GifMessageButton`): it plays, unless motion is reduced,
/// then the still with the "GIF" badge plays while hovered; a click
/// starts or stops it for good.
pub fn paint_gif(
    ui: &mut Ui,
    images: &Images,
    gif: &Gif,
    still: &str,
    message: Uuid,
    rect: Rect,
    id: egui::Id,
) -> Option<MediaAction> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let response = ui.interact(rect, id.with("gif"), Sense::click());
    let label = if gif.title.is_empty() {
        "GIF".to_owned()
    } else {
        format!("GIF: {}", gif.title)
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}, Animation"))
    });
    let state = shown(ui.ctx());
    let chosen = state.lock().gifs.get(&message).copied();
    let playing = chosen.unwrap_or(animate(ui) || response.hovered());
    if response.clicked() {
        state.lock().gifs.insert(message, !playing);
    }
    let played = images.client().http().gif_media_url(&gif.url);
    let url = if playing { played.as_str() } else { still };
    ui.painter().rect_filled(rect, corner(7.0), c.panel);
    match images.frame(ui.ctx(), url, playing, Fit::Inline) {
        ImageState::Ready(texture) => {
            ui.painter().add(
                egui::epaint::RectShape::filled(
                    rect.shrink(1.0),
                    CornerRadius::same(6),
                    Color32::WHITE,
                )
                .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size())),
            );
        }
        _ => {
            let block = Block::single(
                ui,
                "GIF",
                TextStyle::new(13.0, 700.0, 1.5).tracking(0.04),
                c.muted,
            );
            block.paint(ui.painter(), rect.center() - block.size() / 2.0);
        }
    }
    ui.painter().rect_stroke(
        rect,
        corner(7.0),
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    if !playing {
        // Paused: the "GIF" badge bottom-left.
        let badge_text = Block::single(
            ui,
            "GIF",
            TextStyle::new(11.0, 700.0, 1.5).tracking(0.04),
            Color32::WHITE,
        );
        let badge = Rect::from_min_size(
            pos2(
                rect.left() + 6.0,
                rect.bottom() - 6.0 - badge_text.height() - 2.0,
            ),
            vec2(badge_text.width() + 12.0, badge_text.height() + 2.0),
        );
        ui.painter()
            .rect_filled(badge, CornerRadius::same(5), Color32::from_black_alpha(166));
        badge_text.paint(ui.painter(), pos2(badge.left() + 6.0, badge.top() + 1.0));
    }
    if !gif.title.is_empty() {
        crate::widgets::Tooltip::new(&gif.title)
            .on_hover(&response, crate::widgets::tooltip::Side::Above);
    }
    None
}

/// Laid-out link previews.
pub struct Previews {
    cards: Vec<(f32, Card)>,
    pub height: f32,
}

struct Card {
    url: String,
    site: Block,
    title: Option<Block>,
    description: Option<Block>,
    image: Option<(String, bool)>,
    youtube: Option<String>,
    size: Vec2,
    text_h: f32,
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_default()
}

pub fn layout_previews(ui: &Ui, previews: &[LinkPreview], width: f32) -> Option<Previews> {
    if previews.is_empty() {
        return None;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let card_w = width.min(432.0);
    let mut cards = Vec::new();
    let mut y = 0.0;
    for p in previews.iter().take(3) {
        let youtube = p.youtube_id.clone();
        let large = youtube.is_some() || p.image.as_ref().is_some_and(|i| i.width >= 400);
        let thumb = p.image.is_some() && !large;
        let inner = card_w - 3.0 - 1.0 - 24.0;
        let text_w = if thumb { inner - 80.0 - 10.0 } else { inner };
        let site_name = p.site_name.clone().unwrap_or_else(|| host(&p.url));
        let site = Block::wrapped(
            ui,
            &site_name,
            TextStyle::new(12.0, 400.0, 1.5),
            c.muted,
            text_w,
        );
        let title = p.title.as_deref().map(|t| {
            Block::wrapped(
                ui,
                t,
                TextStyle::new(14.0, 600.0, 1.5),
                c.accent_text,
                text_w,
            )
        });
        let description = p.description.as_deref().map(|d| {
            let mut job = crate::text::job(
                d,
                TextStyle::new(13.0, 400.0, 1.4),
                mix(c.muted, c.text, 0.85),
                Some(text_w),
            );
            job.wrap.max_rows = 4;
            Block::new(ui, job, TextStyle::new(13.0, 400.0, 1.4))
        });
        let mut text_h = site.height();
        if let Some(t) = &title {
            text_h += 2.0 + t.height();
        }
        if let Some(d) = &description {
            text_h += 2.0 + d.height();
        }
        let media_h = if youtube.is_some() {
            inner * 9.0 / 16.0
        } else if large {
            p.image
                .as_ref()
                .map(|i| (inner * i.height as f32 / i.width.max(1) as f32).min(160.0))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let body = if thumb {
            text_h.max(80.0)
        } else if large {
            text_h + 10.0 + media_h
        } else {
            text_h
        };
        let size = vec2(card_w, body + 22.0);
        if !cards.is_empty() {
            y += 6.0;
        }
        cards.push((
            y,
            Card {
                url: p.url.clone(),
                site,
                title,
                description,
                image: p.image.as_ref().map(|i| (i.url.clone(), large)),
                youtube,
                size,
                text_h,
            },
        ));
        y += size.y;
    }
    Some(Previews { cards, height: y })
}

impl Previews {
    /// Paints the cards; their site, title and description are selectable
    /// text, added to `text`.
    pub fn paint(
        &self,
        ui: &mut Ui,
        images: &Images,
        origin: Pos2,
        id: egui::Id,
        text: &mut Option<egui::Response>,
    ) -> Option<MediaAction> {
        let theme = Theme::of(ui.ctx());
        let c = theme.c;
        let mut action = None;
        for (n, (y, card)) in self.cards.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(origin.x, origin.y + y), card.size);
            let painter = ui.painter();
            painter.rect(
                rect,
                corner(5.0),
                c.panel,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            painter.rect_filled(
                Rect::from_min_max(rect.min, pos2(rect.left() + 3.0, rect.bottom())),
                CornerRadius {
                    nw: 5,
                    sw: 5,
                    ne: 0,
                    se: 0,
                },
                mix(c.border, c.accent, 0.7),
            );
            let x = rect.left() + 3.0 + 12.0;
            let mut ty = rect.top() + 11.0;
            let at = pos2(x, ty);
            let site = card.site.select(ui, id.with(("preview-site", n)), at);
            card.site.paint_selectable(ui, &site, at);
            crate::text::join(text, site);
            ty += card.site.height() + 2.0;
            if let Some(title) = &card.title {
                let title_rect = Rect::from_min_size(pos2(x, ty), title.size());
                // Selectable like a link in a message: the link on top.
                let selection =
                    title.select(ui, id.with(("preview-title-text", n)), title_rect.min);
                let response =
                    ui.interact(title_rect, id.with(("preview-title", n)), Sense::click());
                title.paint_selectable(ui, &selection, title_rect.min);
                crate::text::join(text, selection);
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    crate::widgets::link::underline(
                        ui.painter(),
                        TextStyle::new(14.0, 600.0, 1.5),
                        ty,
                        x,
                        x + title.width(),
                        c.accent_text,
                    );
                }
                if response.clicked() {
                    action = Some(MediaAction::External(card.url.clone()));
                }
                ty += title.height() + 2.0;
            }
            if let Some(d) = &card.description {
                let at = pos2(x, ty);
                let description = d.select(ui, id.with(("preview-description", n)), at);
                d.paint_selectable(ui, &description, at);
                crate::text::join(text, description);
            }
            if let Some((url, large)) = &card.image {
                let inner_w = card.size.x - 3.0 - 1.0 - 24.0;
                let img_rect = if *large {
                    Rect::from_min_max(
                        pos2(x, rect.top() + 11.0 + card.text_h + 10.0),
                        pos2(x + inner_w, rect.bottom() - 11.0),
                    )
                } else {
                    Rect::from_min_size(
                        pos2(rect.right() - 13.0 - 80.0, rect.top() + 11.0),
                        vec2(80.0, 80.0),
                    )
                };
                if card.youtube.is_some() {
                    ui.painter()
                        .rect_filled(img_rect, corner(5.0), Color32::BLACK);
                }
                if let ImageState::Ready(t) = images.shown(ui.ctx(), url, Fit::Inline) {
                    ui.painter().add(
                        egui::epaint::RectShape::filled(
                            img_rect,
                            CornerRadius::same(5),
                            Color32::WHITE,
                        )
                        .with_texture(t.id(), cover_uv(t.size_vec2(), img_rect.size())),
                    );
                }
                let response = ui.interact(img_rect, id.with(("preview-image", n)), Sense::click());
                if card.youtube.is_some() {
                    let badge = Rect::from_center_size(img_rect.center(), vec2(56.0, 40.0));
                    let fill = if response.hovered() {
                        Color32::from_rgb(0xcc, 0, 0)
                    } else {
                        Color32::from_black_alpha(178)
                    };
                    ui.painter()
                        .rect_filled(badge, CornerRadius::same(10), fill);
                    let play =
                        Block::single(ui, "▶", TextStyle::new(18.0, 400.0, 1.5), Color32::WHITE);
                    play.paint(ui.painter(), badge.center() - play.size() / 2.0);
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(if card.youtube.is_some() {
                        egui::CursorIcon::PointingHand
                    } else {
                        egui::CursorIcon::ZoomIn
                    });
                }
                if response.clicked() {
                    action = Some(if card.youtube.is_some() {
                        MediaAction::External(card.url.clone())
                    } else {
                        MediaAction::Open(
                            url.clone(),
                            card.title
                                .as_ref()
                                .map(|_| String::new())
                                .unwrap_or_default(),
                        )
                    });
                }
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;

    fn attachment(id: u128, name: &str, content_type: &str) -> Attachment {
        Attachment {
            id: Uuid::from_u128(id),
            filename: name.into(),
            content_type: content_type.into(),
            size: 2048,
        }
    }

    /// Draws `attachments` as a message row does, against a server that
    /// is not there: every picture fails.
    fn harness<'a>(
        attachments: Vec<Attachment>,
        uploads: Option<Vec<UploadState>>,
    ) -> (egui_kittest::Harness<'a>, Images) {
        let client = gelabber_client::Client::new("http://127.0.0.1:9").expect("client");
        let images = Images::new(client);
        let drawn = images.clone();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui(move |ui| {
                crate::fonts::install(ui.ctx());
                crate::theme::set(ui.ctx(), Theme::dark());
                let media = layout(ui, &drawn, &attachments, 600.0, uploads.as_deref());
                if let Some(media) = media {
                    let mut text = None;
                    let _ = media.paint(
                        ui,
                        &drawn,
                        pos2(10.0, 10.0),
                        egui::Id::unique("t"),
                        &mut text,
                    );
                }
            });
        (harness, images)
    }

    fn settle(harness: &mut egui_kittest::Harness<'_>, images: &Images) {
        for _ in 0..200 {
            harness.step();
            if !images.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.run_steps(3);
    }

    #[test]
    fn a_picture_that_did_not_load_says_so() {
        let (mut harness, images) = harness(vec![attachment(1, "banner.png", "image/png")], None);
        settle(&mut harness, &images);
        harness.get_by_label("Bild konnte nicht geladen werden.");
        harness.get_by_label("Herunterladen");
        // "Erneut laden" asks again; here it fails again.
        harness.get_by_label("Erneut laden").click();
        for _ in 0..300 {
            harness.step();
            if harness
                .query_by_label("Bild konnte nicht geladen werden.")
                .is_some()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.get_by_label("Bild konnte nicht geladen werden.");
        let url = attachment_url(Uuid::from_u128(1));
        images.retry(&url);
        assert!(matches!(
            images.get(&harness.ctx, &url),
            ImageState::Loading
        ));
    }

    #[test]
    fn files_on_their_way_show_their_upload_state() {
        let files = vec![
            attachment(0, "a.pdf", "application/pdf"),
            attachment(0, "b.pdf", "application/pdf"),
            attachment(0, "c.png", "image/png"),
        ];
        let uploads = vec![
            UploadState::Uploading,
            UploadState::Waiting,
            UploadState::Waiting,
        ];
        let (mut harness, images) = harness(files, Some(uploads));
        settle(&mut harness, &images);
        harness.get_by_label("Wird hochgeladen…");
        assert_eq!(harness.query_all_by_label("Wartet").count(), 2);
        // A picture still uploading is no failed picture.
        assert!(
            harness
                .query_by_label("Bild konnte nicht geladen werden.")
                .is_none()
        );
        assert_eq!(upload_text(Some(UploadState::Done)), None);
    }

    #[test]
    fn server_file_names_are_safe_suggestions() {
        assert_eq!(safe_file_name("Bericht 2026.pdf"), "Bericht 2026.pdf");
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("a\\b:c*?.txt"), "a_b_c__.txt");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("com1"), "_com1");
        assert_eq!(safe_file_name("..."), "Download");
        assert_eq!(safe_file_name("bild.png. "), "bild.png");
        assert_eq!(safe_file_name("x\u{0}y"), "x_y");
        assert_eq!(safe_file_name(&"a".repeat(300)).len(), 200);
        // Four bytes each: the cap is in bytes, at a character's end.
        let wide = safe_file_name(&"😀".repeat(100));
        assert_eq!(wide.len(), 200);
        assert_eq!(safe_file_name(&format!("a{}", "😀".repeat(100))).len(), 197);
    }

    #[test]
    fn a_download_comes_through_the_session_into_a_file() {
        let id = Uuid::from_u128(5);
        let path: &'static str =
            Box::leak(format!("{}?download=1", attachment_url(id)).into_boxed_str());
        let origin = crate::images::tests::serve(vec![(path, b"%PDF-1.7".to_vec())]);
        let client = gelabber_client::Client::new(&origin).expect("client");
        let dir = std::env::temp_dir().join(format!("gelabber-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let ctx = egui::Context::default();
        crate::platform::files::set_choosers(
            &ctx,
            crate::platform::files::Choosers {
                bus: None,
                portal: false,
                native: false,
                zenity: String::new(),
                kdialog: String::new(),
                downloads: Some(dir.clone()),
            },
        );
        let att = attachment(5, "../Rechnung?.pdf", "application/pdf");
        let MediaAction::Download { url, name } = MediaAction::download(&att) else {
            panic!("a download");
        };
        download(&ctx, &client, &url, &name);
        let saved = dir.join("_Rechnung_.pdf");
        for _ in 0..300 {
            if saved.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(&saved).expect("saved"), b"%PDF-1.7");
        let _ = std::fs::remove_dir_all(&dir);
        // A refused fetch says so.
        download(&ctx, &client, "/api/attachments/gone?download=1", "x.pdf");
        let mut errors = Vec::new();
        for _ in 0..300 {
            errors = take_download_errors(&ctx);
            if !errors.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("Datei nicht heruntergeladen."));
    }

    #[test]
    fn a_long_text_preview_scrolls() {
        let id = Uuid::from_u128(7);
        let path: &'static str = Box::leak(attachment_url(id).into_boxed_str());
        let text: String = (1..=60).map(|n| format!("Zeile {n}\n")).collect();
        let origin = crate::images::tests::serve(vec![(path, text.into_bytes())]);
        let images = Images::new(gelabber_client::Client::new(&origin).expect("client"));
        let drawn = images.clone();
        let file = Attachment {
            size: 600,
            ..attachment(7, "log.txt", "text/plain")
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui(move |ui| {
                crate::fonts::install(ui.ctx());
                crate::theme::set(ui.ctx(), Theme::dark());
                shown(ui.ctx()).lock().open.insert(id, true);
                if let Some(media) = layout(ui, &drawn, std::slice::from_ref(&file), 600.0, None) {
                    let mut text = None;
                    let _ = media.paint(
                        ui,
                        &drawn,
                        pos2(10.0, 10.0),
                        egui::Id::unique("t"),
                        &mut text,
                    );
                }
            });
        settle(&mut harness, &images);
        assert!(matches!(
            images.text(&harness.ctx, path),
            TextState::Ready(_)
        ));
        let offset = |h: &egui_kittest::Harness<'_>| shown(&h.ctx).lock().scroll.get(&id).copied();
        assert_eq!(offset(&harness), Some(Vec2::ZERO));
        harness.event(egui::Event::PointerMoved(pos2(100.0, 150.0)));
        for _ in 0..10 {
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -200.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
        }
        harness.run_steps(30);
        // 60 lines of 19.2 in a 300 high box: scrolled to its end, no further.
        let y = offset(&harness).expect("scrolled").y;
        assert!((y - (60.0 * 19.2 - 300.0)).abs() < 20.0, "{y}");
        // The list under it gets the wheel back at the end.
        assert!(!takes_wheel(&harness.ctx, pos2(100.0, 150.0), -10.0));
        assert!(takes_wheel(&harness.ctx, pos2(100.0, 150.0), 10.0));
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(1536 * 1024), "1,5 MB");
        assert_eq!(format_size(100), "1 KB");
        assert_eq!(format_size(12 * 1024 + 300), "12 KB");
        let b = image_box(Some(vec2(1920.0, 1080.0)), 656.0);
        assert!(
            (b.x - 360.0).abs() < 0.01 && (b.y - 203.37).abs() < 0.05,
            "{b:?}"
        );
        assert_eq!(image_box(None, 656.0), vec2(360.0, 240.0));
    }
}
