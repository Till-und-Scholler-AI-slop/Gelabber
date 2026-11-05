//! One message in the list (components/MessageRow.tsx, chat.css
//! `.lr-message-row`): group head with avatar, name and time, or a
//! continued row with the time in the gutter; the body, its media and
//! reactions; the hover action bar; the send state of my own rows.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_client::Message;
use uuid::Uuid;

use super::attachments::{self, Media, MediaAction, Previews};
use super::content::{self, ContentAction, Laid, Mentions};
use super::reactions::{self, Chips, Shown};
use super::time;
use crate::icons::{self, Icon};
use crate::images::{ImageState, Images, cover_uv};
use crate::initials::{initials, person_tone};
use crate::text::{Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::tooltip::{Side, Tooltip};
use crate::widgets::{Button, ButtonVariant};

pub const PAD_X: f32 = 20.0;
const AVATAR: f32 = 36.0;
/// Text column x: padding 20, avatar 36, gap 12.
pub const COLUMN_X: f32 = PAD_X + AVATAR + 12.0;

const NAME: TextStyle = TextStyle::new(14.0, 650.0, 1.5);
const TIME: TextStyle = TextStyle::new(11.0, 400.0, 1.5);
const GUTTER: TextStyle = TextStyle::new(10.5, 400.0, 18.0 / 10.5);
const REPLY: TextStyle = TextStyle::new(12.5, 400.0, 1.6);
const SYSTEM: TextStyle = TextStyle::new(13.5, 400.0, 1.5);
const STATUS: TextStyle = TextStyle::new(12.0, 400.0, 1.4);
const FAILURE: TextStyle = TextStyle::new(13.0, 400.0, 1.35);
const EDIT: TextStyle = TextStyle::new(14.5, 400.0, 1.5);

/// The state of my own row that the server has not confirmed.
#[derive(Clone, Debug, PartialEq)]
pub enum SendState {
    /// Still on its way; the status shows after 400ms.
    Sending { visible: bool },
    /// Not sent; `uncertain`: the answer to the bind was lost, so the
    /// message may be saved and is not offered again (`"uncertain"`).
    Failed { error: String, uncertain: bool },
}

impl SendState {
    /// `SendStatus`: the words in place of the time.
    pub fn status(&self) -> &'static str {
        match self {
            Self::Sending { .. } => "Wird gesendet…",
            Self::Failed {
                uncertain: false, ..
            } => "Nicht gesendet",
            Self::Failed {
                uncertain: true, ..
            } => "Status unklar – Verlauf prüfen",
        }
    }

    /// `SendFailure`: why it was not sent; for a lost answer, what to check.
    pub fn failure(&self) -> Option<String> {
        match self {
            Self::Sending { .. } => None,
            Self::Failed { error, uncertain } => Some(if *uncertain {
                format!(
                    "{error} Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest."
                )
            } else {
                error.clone()
            }),
        }
    }
}

/// `attempt.error`: "{stage} fehlgeschlagen." and the uploads the server
/// turned down, by file name.
pub fn failure_line(pending: &gelabber_client::PendingMessage) -> String {
    let mut line = format!("{} fehlgeschlagen.", pending.stage.label());
    if let Some(error) = &pending.failed
        && let Some(rejected) = gelabber_client::store::rejected_uploads(error)
    {
        for (index, field, code) in rejected {
            if let Some(file) = index.and_then(|i| pending.files.get(i)) {
                line.push(' ');
                line.push_str(&format!(
                    "{}: {}",
                    file.filename,
                    crate::rules::field_message(&field, &code)
                ));
            }
        }
    }
    line
}

pub struct RowCx<'a> {
    pub msg: &'a Message,
    pub continued: bool,
    pub mine: bool,
    pub mentioned: bool,
    pub send: Option<SendState>,
    pub can_send: bool,
    pub can_delete: bool,
    pub me: Uuid,
    /// Lower-cased id → name, for mentions.
    pub names: &'a HashMap<String, String>,
    pub now: DateTime<Utc>,
    pub revealed: &'a HashSet<usize>,
    pub copied: Option<(usize, bool)>,
    pub images: &'a Images,
    pub reactions: Vec<Shown>,
    /// The draft while this row is being edited.
    pub edit: Option<&'a str>,
    /// 0..1: the jump highlight fading out.
    pub highlight: f32,
    /// The GIF's picture through the server's media proxy.
    pub gif_url: Option<String>,
    /// My files' upload states while the message is on its way.
    pub uploads: Option<Vec<gelabber_client::store::UploadState>>,
}

impl RowCx<'_> {
    fn pending(&self) -> bool {
        self.send.is_some()
    }

    fn system(&self) -> bool {
        self.msg.system.is_some()
    }

    fn name_of(&self, user: Uuid) -> Option<String> {
        self.names.get(&user.to_string()).cloned()
    }
}

/// What a row asks for after a frame.
pub enum RowAction {
    React(Uuid, String, bool),
    Picker(Uuid),
    Reply(Uuid),
    StartEdit(Uuid),
    Menu(Uuid, Rect),
    JumpTo(Uuid),
    Content(Uuid, ContentAction),
    Media(MediaAction),
    Retry(Uuid),
    Discard(Uuid),
    TakeBack(Uuid),
    SaveEdit(Uuid),
    CancelEdit,
}

struct Head {
    name: Block,
    time: Block,
    status: Option<Block>,
    reply: Option<ReplyLine>,
}

struct ReplyLine {
    deleted: bool,
    author: Option<Block>,
    excerpt: Block,
    clip: bool,
    target: Uuid,
}

struct Failure {
    text: Block,
    retry: bool,
    edit: bool,
}

pub struct RowLayout {
    pub height: f32,
    head: Option<Head>,
    gutter: Option<Block>,
    system: Option<Block>,
    content: Option<(f32, Laid)>,
    gif: Option<(f32, Vec2)>,
    media: Option<(f32, Media)>,
    previews: Option<(f32, Previews)>,
    reactions: Option<(f32, Chips)>,
    status_line: Option<(f32, Block)>,
    failure: Option<(f32, Failure)>,
    edit: Option<(f32, f32)>,
}

/// Vertical stacking with CSS margin collapsing between siblings.
struct Stack {
    y: f32,
    pending: f32,
}

impl Stack {
    fn add(&mut self, margin_top: f32, height: f32, margin_bottom: f32) -> f32 {
        self.y += self.pending.max(margin_top);
        let top = self.y;
        self.y += height;
        self.pending = margin_bottom;
        top
    }

    fn end(&self) -> f32 {
        self.y + self.pending
    }
}

/// Text with `<@id>` tokens as `@Name` (unknown: "@Unbekannt").
pub fn readable_mentions(text: &str, names: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let id_len = 36;
        if tail.len() > id_len
            && tail.as_bytes()[id_len] == b'>'
            && Uuid::parse_str(&tail[..id_len]).is_ok()
        {
            let id = tail[..id_len].to_ascii_lowercase();
            out.push('@');
            out.push_str(names.get(&id).map(String::as_str).unwrap_or("Unbekannt"));
            rest = &tail[id_len + 1..];
        } else {
            out.push_str("<@");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn system_text(msg: &Message) -> String {
    match &msg.system {
        Some(s) if s.kind == "live_started" => {
            let voice = s
                .params
                .get("voice_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            format!("{} ist live in {voice}.", msg.author.name)
        }
        _ => format!("Systemnachricht von {}", msg.author.name),
    }
}

/// Lays out the row for a pane `width` wide.
pub fn layout(ui: &Ui, cx: &RowCx, width: f32) -> RowLayout {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let column = (width - COLUMN_X - PAD_X).max(40.0);
    let msg = cx.msg;
    let mentions = Mentions {
        names: cx.names,
        me: Some(cx.me),
        everyone: msg.mention_everyone,
        mentions: &msg.mentions,
    };
    let mut out = RowLayout {
        height: 0.0,
        head: None,
        gutter: None,
        system: None,
        content: None,
        gif: None,
        media: None,
        previews: None,
        reactions: None,
        status_line: None,
        failure: None,
        edit: None,
    };
    let when = time::format_when(msg.created_at, cx.now);
    if cx.system() {
        let text = format!("{} ", system_text(msg));
        let mut job = crate::text::job(&text, SYSTEM, c.muted, Some(column));
        job.append(&when, 0.0, {
            let mut f = TIME.format(c.muted);
            f.line_height = Some(SYSTEM.line_px());
            f
        });
        let block = Block::new(ui, job, SYSTEM);
        let mut stack = Stack {
            y: 0.0,
            pending: 0.0,
        };
        stack.add(0.0, block.height(), 0.0);
        out.system = Some(block);
        add_extras(ui, cx, &mut out, &mut stack, column);
        out.height = 6.0 + stack.end() + 4.0;
        return out;
    }
    let mut stack = Stack {
        y: 0.0,
        pending: 0.0,
    };
    let head = !cx.continued || cx.edit.is_some();
    if head {
        let reply = msg.reply.as_ref().map(|r| {
            let deleted = r.deleted || r.author.is_none();
            if deleted {
                let style = REPLY.italic();
                ReplyLine {
                    deleted,
                    author: None,
                    excerpt: Block::single(
                        ui,
                        "Die ursprüngliche Nachricht wurde gelöscht.",
                        style,
                        c.muted,
                    ),
                    clip: false,
                    target: r.id,
                }
            } else {
                let author = r
                    .author
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_default();
                let summary = r
                    .excerpt
                    .as_deref()
                    .filter(|e| !e.is_empty())
                    .map(|e| readable_mentions(e, cx.names))
                    .unwrap_or_else(|| {
                        if r.has_gif {
                            "GIF".into()
                        } else if r.has_attachment {
                            "Anhang".into()
                        } else {
                            String::new()
                        }
                    });
                let author_block =
                    Block::single(ui, &author, REPLY.weight(600.0), mix(c.muted, c.text, 0.85));
                let room = (column - 13.0 - 6.0 - author_block.width() - 6.0).max(10.0);
                let mut job = crate::text::job(&summary, REPLY, c.muted, Some(room));
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                ReplyLine {
                    deleted,
                    author: Some(author_block),
                    excerpt: Block::new(ui, job, REPLY),
                    clip: r.excerpt.is_some() && (r.has_attachment || r.has_gif),
                    target: r.id,
                }
            }
        });
        if reply.is_some() {
            stack.add(0.0, 20.0, 2.0);
        }
        let status = cx.send.as_ref().map(|s| match s {
            SendState::Sending { .. } => Block::single(ui, s.status(), STATUS, c.muted),
            SendState::Failed { .. } => {
                Block::single(ui, s.status(), STATUS.weight(600.0), c.danger)
            }
        });
        out.head = Some(Head {
            name: Block::single(ui, &msg.author.name, NAME, c.text),
            time: Block::single(ui, &when, TIME, c.muted),
            status,
            reply,
        });
        stack.add(0.0, 21.0, 0.0);
    } else if cx.send.is_none() {
        out.gutter = Some(Block::single(
            ui,
            &time::format_clock(msg.created_at),
            GUTTER,
            c.muted,
        ));
    }
    if let Some(draft) = cx.edit {
        let lines = draft.split('\n').count().max(1) as f32;
        let editor = (lines * EDIT.line_px() + 18.0).clamp(50.75, 400.0);
        // Form: margin-top 4, gap 6, then the actions line: its buttons
        // (padding 4, 12px/1.5) make it 26 tall.
        let h = editor + 6.0 + 26.0;
        let top = stack.add(4.0, h, 0.0);
        out.edit = Some((top, editor));
    } else if !msg.content.is_empty() || msg.gif.is_none() {
        let laid = content::layout(
            ui,
            &msg.content,
            column,
            &mentions,
            cx.revealed,
            msg.edited_at.is_some(),
        );
        let top = stack.add(laid.margin_top, laid.height, laid.margin_bottom);
        out.content = Some((top, laid));
    }
    if cx.edit.is_none() {
        add_extras(ui, cx, &mut out, &mut stack, column);
    }
    if !head && let Some(send) = &cx.send {
        let block = match send {
            SendState::Failed { .. } => {
                Block::single(ui, send.status(), STATUS.weight(600.0), c.danger)
            }
            SendState::Sending { .. } => Block::single(ui, send.status(), STATUS, c.muted),
        };
        let top = stack.add(2.0, block.height(), 0.0);
        out.status_line = Some((top, block));
    }
    if let Some(send @ SendState::Failed { uncertain, .. }) = &cx.send {
        let text = Block::wrapped(
            ui,
            &send.failure().unwrap_or_default(),
            FAILURE,
            c.muted,
            column,
        );
        // Text, then the buttons (32) 6px below.
        let h = text.height() + 6.0 + 32.0;
        let top = stack.add(6.0, h, 0.0);
        out.failure = Some((
            top,
            Failure {
                text,
                // Sending again could save it twice.
                retry: !uncertain,
                edit: msg.gif.is_none(),
            },
        ));
    }
    let column_h = stack.end();
    out.height = if head {
        6.0 + column_h.max(2.0 + AVATAR) + 4.0
    } else {
        1.0 + column_h.max(18.0 + 4.0) + 1.0
    };
    out
}

fn add_extras(ui: &Ui, cx: &RowCx, out: &mut RowLayout, stack: &mut Stack, column: f32) {
    let msg = cx.msg;
    if let Some(gif) = &msg.gif {
        let size = attachments::gif_box(gif);
        let top = stack.add(4.0, size.y, 0.0);
        out.gif = Some((top, size));
    }
    if let Some(media) = attachments::layout(
        ui,
        cx.images,
        &msg.attachments,
        column,
        cx.uploads.as_deref(),
    ) {
        let top = stack.add(6.0, media.height, 0.0);
        out.media = Some((top, media));
    }
    if let Some(previews) = attachments::layout_previews(ui, &msg.previews, column) {
        let top = stack.add(6.0, previews.height, 0.0);
        out.previews = Some((top, previews));
    }
    if !cx.pending() && !cx.reactions.is_empty() {
        let chips = reactions::layout(ui, &cx.reactions, column);
        let top = stack.add(6.0, chips.height, 0.0);
        out.reactions = Some((top, chips));
    }
}

/// Paints a person's circle: the picture, or initials on their tint.
pub fn paint_avatar(
    ui: &Ui,
    images: &Images,
    rect: Rect,
    name: &str,
    url: Option<&str>,
    font: f32,
) {
    let theme = Theme::of(ui.ctx());
    let rect = crate::widgets::snap(ui.ctx(), rect);
    let painter = ui.painter();
    let texture = url
        .filter(|u| !u.is_empty())
        .and_then(|u| match images.get(ui.ctx(), u) {
            ImageState::Ready(t) => Some(t),
            _ => None,
        });
    if let Some(texture) = texture {
        painter.add(
            egui::epaint::RectShape::filled(
                rect,
                CornerRadius::same((rect.width() / 2.0).round() as u8),
                Color32::WHITE,
            )
            .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size())),
        );
    } else {
        painter.circle_filled(
            rect.center(),
            rect.width() / 2.0,
            theme.tile(person_tone(name)),
        );
        let style = TextStyle::new(font, 600.0, 1.5);
        let block = Block::single(ui, &initials(name), style, theme.c.text);
        block.paint(
            painter,
            pos2(
                rect.center().x - block.width() / 2.0,
                rect.center().y - block.height() / 2.0,
            ),
        );
    }
}

/// The row background for its state.
fn background(theme: &Theme, cx: &RowCx, hovered: bool) -> Option<Color32> {
    let c = theme.c;
    let failed = matches!(cx.send, Some(SendState::Failed { .. }));
    let base = if failed {
        Some(if hovered {
            mix(c.bg, c.danger_bg, 0.12)
        } else {
            mix(c.bg, c.danger_bg, 0.08)
        })
    } else if cx.mentioned {
        Some(if hovered {
            mix(c.bg, c.warning, 0.13)
        } else {
            mix(c.bg, c.warning, 0.09)
        })
    } else if hovered {
        Some(mix(c.bg, c.text, 0.035))
    } else {
        None
    };
    if cx.highlight > 0.0 {
        let under = base.unwrap_or(c.bg);
        return Some(mix(under, c.accent, 0.14 * f64::from(cx.highlight)));
    }
    base
}

pub struct BarButton {
    pub icon: Icon,
    pub size: f32,
    pub label: &'static str,
}

/// The hover bar's buttons for this row (`MessageRow.tsx` actions).
pub fn bar_buttons(cx: &RowCx) -> Vec<BarButton> {
    let editing = cx.edit.is_some();
    let can_react = cx.can_send && !cx.pending() && !editing;
    let show_edit =
        cx.mine && cx.can_send && !cx.pending() && !editing && cx.msg.gif.is_none() && !cx.system();
    let show_delete = cx.can_delete && !cx.pending() && !editing;
    let has_menu = !cx.pending()
        && !editing
        && (can_react || show_edit || show_delete || !cx.msg.content.is_empty());
    if !has_menu {
        return Vec::new();
    }
    let mut out = Vec::new();
    if can_react {
        out.push(BarButton {
            icon: Icon::Smile,
            size: 16.0,
            label: "Reaktion hinzufügen",
        });
        out.push(BarButton {
            icon: Icon::Reply,
            size: 14.0,
            label: "Antworten",
        });
    }
    if show_edit {
        out.push(BarButton {
            icon: Icon::Pencil,
            size: 14.0,
            label: "Nachricht bearbeiten",
        });
    }
    out.push(BarButton {
        icon: Icon::More,
        size: 14.0,
        label: "Weitere Aktionen",
    });
    out
}

/// The bar's rect for a row at `rect`.
pub fn bar_rect(cx: &RowCx, rect: Rect) -> Option<Rect> {
    let n = bar_buttons(cx).len();
    if n == 0 {
        return None;
    }
    let w = 2.0 + 2.0 + n as f32 * 30.0 + (n as f32 - 1.0) * 2.0 + 2.0;
    let top = rect.top()
        - if cx.continued && cx.edit.is_none() {
            16.0
        } else {
            14.0
        };
    Some(Rect::from_min_size(
        pos2(rect.right() - 16.0 - w, top),
        vec2(w, 34.0),
    ))
}

/// The bar's "more" button for a row at `rect`: the menu's anchor, also
/// when a right click opens it (`setMenu("more")`).
pub fn more_rect(cx: &RowCx, rect: Rect) -> Option<Rect> {
    let bar = bar_rect(cx, rect)?;
    let n = bar_buttons(cx).len() as f32;
    Some(Rect::from_min_size(
        pos2(bar.left() + 3.0 + (n - 1.0) * 32.0, bar.top() + 3.0),
        vec2(30.0, 28.0),
    ))
}

/// How the row shows: hovered (or focused), its menu open.
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub hovered: bool,
    pub menu_open: bool,
}

/// The draft of the row being edited.
pub struct Editing<'a> {
    pub draft: &'a mut String,
    /// The editor takes the keyboard this frame.
    pub autofocus: bool,
}

pub struct Painted {
    pub actions: Vec<RowAction>,
}

/// Paints the row into `rect` (its full width). `edit` is the editor's
/// draft while editing; `menu_open` keeps the "more" button lit.
pub fn paint(
    ui: &mut Ui,
    cx: &RowCx,
    layout: &RowLayout,
    rect: Rect,
    look: Look,
    edit: Option<Editing>,
) -> Painted {
    let Look { hovered, menu_open } = look;
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let msg = cx.msg;
    let id = egui::Id::unique(("gelabber-row", msg.id));
    // `.lr-message-row:focus-within` looks hovered: the editor has the keyboard.
    let hovered = hovered || (edit.is_some() && ui.memory(|m| m.has_focus(editor_id(msg.id))));
    let mut actions = Vec::new();
    if let Some(fill) = background(&theme, cx, hovered) {
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    let failed = matches!(cx.send, Some(SendState::Failed { .. }));
    if failed {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            0.0,
            c.danger_bg,
        );
    } else if cx.mentioned {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
            0.0,
            c.warning,
        );
    }
    let x = rect.left() + COLUMN_X;
    let column = rect.width() - COLUMN_X - PAD_X;
    let sending = matches!(cx.send, Some(SendState::Sending { .. }));
    let top = if layout.head.is_some() || layout.system.is_some() {
        rect.top() + 6.0
    } else {
        rect.top() + 1.0
    };
    if let Some(system) = &layout.system {
        icons::paint(
            ui.painter(),
            Icon::Live,
            pos2(rect.left() + PAD_X + 11.0, top + 3.0),
            14.0,
            c.muted,
        );
        system.paint(ui.painter(), pos2(x, top));
    }
    if let Some(head) = &layout.head {
        paint_avatar(
            ui,
            cx.images,
            Rect::from_min_size(pos2(rect.left() + PAD_X, top + 2.0), Vec2::splat(AVATAR)),
            &msg.author.name,
            msg.author.avatar_url.as_deref(),
            12.0,
        );
        let mut y = top;
        if let Some(reply) = &head.reply {
            let line = Rect::from_min_size(pos2(x, y), vec2(column, 20.0));
            let response =
                (!reply.deleted).then(|| ui.interact(line, id.with("reply"), Sense::click()));
            let hot = response.as_ref().is_some_and(|r| r.hovered());
            icons::paint(ui.painter(), Icon::Reply, pos2(x, y + 3.5), 13.0, c.muted);
            let mut tx = x + 13.0 + 6.0;
            if let Some(author) = &reply.author {
                author.paint(ui.painter(), pos2(tx, y));
                tx += author.width() + 6.0;
            }
            reply
                .excerpt
                .paint_tinted(ui.painter(), pos2(tx, y), hot.then_some(c.text));
            if reply.clip {
                icons::paint(
                    ui.painter(),
                    Icon::Paperclip,
                    pos2(tx + reply.excerpt.width() + 4.0, y + 5.0),
                    12.0,
                    if hot { c.text } else { c.muted },
                );
            }
            if let Some(response) = response {
                if hot {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    actions.push(RowAction::JumpTo(reply.target));
                }
            }
            y += 22.0;
        }
        head.name.paint(ui.painter(), pos2(x, y));
        let tx = x + head.name.width() + 8.0;
        match (&head.status, &cx.send) {
            (Some(status), Some(SendState::Sending { visible })) => {
                if *visible {
                    paint_spinner(ui, pos2(tx, y + 5.0 + 1.5), c.muted);
                    status.paint(ui.painter(), pos2(tx + 15.0, y + 3.0));
                }
            }
            (Some(status), Some(SendState::Failed { .. })) => {
                icons::paint(
                    ui.painter(),
                    Icon::Warning,
                    pos2(tx, y + 5.0),
                    13.0,
                    c.danger,
                );
                status.paint(ui.painter(), pos2(tx + 18.0, y + 3.0));
            }
            _ => {
                let time_rect = Rect::from_min_size(pos2(tx, y + 4.0), head.time.size());
                head.time.paint(ui.painter(), time_rect.min);
                let response = ui.interact(time_rect, id.with("time"), Sense::hover());
                Tooltip::new(&time::format_full(msg.created_at)).on_hover(&response, Side::Above);
            }
        }
    }
    if let Some(gutter) = &layout.gutter
        && hovered
    {
        let gx = rect.left() + PAD_X + AVATAR - gutter.width();
        let grect = Rect::from_min_size(pos2(gx, top + 4.0), gutter.size());
        gutter.paint(ui.painter(), grect.min);
        let response = ui.interact(grect, id.with("gutter"), Sense::hover());
        Tooltip::new(&time::format_full(msg.created_at)).on_hover(&response, Side::Above);
    }
    let fade = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
        if sending {
            ui.scope(|ui| {
                ui.set_opacity(0.7);
                f(ui);
            });
        } else {
            f(ui);
        }
    };
    if let Some((y, laid)) = &layout.content {
        let origin = pos2(x, top + y);
        fade(ui, &mut |ui| {
            if let Some(action) = laid.paint(ui, origin, id, true, cx.copied) {
                actions.push(RowAction::Content(msg.id, action));
            }
        });
    }
    if let Some((y, size)) = &layout.gif
        && let (Some(gif), Some(url)) = (&msg.gif, &cx.gif_url)
    {
        let r = Rect::from_min_size(pos2(x, top + y), *size);
        if let Some(a) = attachments::paint_gif(ui, cx.images, gif, url, msg.id, r, id) {
            actions.push(RowAction::Media(a));
        }
    }
    if let Some((y, media)) = &layout.media {
        let origin = pos2(x, top + y);
        fade(ui, &mut |ui| {
            if let Some(a) = media.paint(ui, cx.images, origin, id) {
                actions.push(RowAction::Media(a));
            }
        });
    }
    if let Some((y, previews)) = &layout.previews
        && let Some(a) = previews.paint(ui, cx.images, pos2(x, top + y), id)
    {
        actions.push(RowAction::Media(a));
    }
    if let Some((y, chips)) = &layout.reactions {
        let name_of = |u: Uuid| cx.name_of(u);
        if let Some((emoji, add)) = reactions::paint(
            ui,
            chips,
            pos2(x, top + y),
            id,
            cx.me,
            cx.can_send,
            &name_of,
        ) {
            actions.push(RowAction::React(msg.id, emoji, add));
        }
    }
    if let Some((y, block)) = &layout.status_line {
        let p = pos2(x, top + y);
        if matches!(cx.send, Some(SendState::Failed { .. })) {
            icons::paint(
                ui.painter(),
                Icon::Warning,
                pos2(p.x, p.y + 2.0),
                13.0,
                c.danger,
            );
            block.paint(ui.painter(), pos2(p.x + 18.0, p.y));
        } else if matches!(cx.send, Some(SendState::Sending { visible: true })) {
            paint_spinner(ui, pos2(p.x, p.y + 3.8), c.muted);
            block.paint(ui.painter(), pos2(p.x + 15.0, p.y));
        }
    }
    if let Some((y, failure)) = &layout.failure {
        let p = pos2(x, top + y);
        failure.text.paint(ui.painter(), p);
        let by = p.y + failure.text.height() + 6.0;
        let mut ui2 = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(p.x, by), vec2(column, 32.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        ui2.spacing_mut().item_spacing.x = 6.0;
        if failure.retry && ui2.add(Button::new("Erneut senden").small()).clicked() {
            actions.push(RowAction::Retry(msg.id));
        }
        if failure.edit
            && ui2
                .add(
                    Button::new("Bearbeiten")
                        .small()
                        .variant(ButtonVariant::Ghost),
                )
                .clicked()
        {
            actions.push(RowAction::TakeBack(msg.id));
        }
        if ui2
            .add(
                Button::new("Verwerfen")
                    .small()
                    .variant(ButtonVariant::Ghost),
            )
            .clicked()
        {
            actions.push(RowAction::Discard(msg.id));
        }
    }
    if let (Some((y, editor_h)), Some(edit)) = (layout.edit, edit) {
        let rect = Rect::from_min_size(pos2(x, top + y), vec2(column, editor_h));
        paint_editor(ui, msg.id, rect, edit, &mut actions);
    }
    // The hover bar sits over the row's top edge, above the previous row.
    if (hovered || menu_open)
        && let Some(bar) = bar_rect(cx, rect)
    {
        paint_bar(ui, cx, bar, id, menu_open, &mut actions);
    }
    Painted { actions }
}

fn paint_spinner(ui: &Ui, min: Pos2, color: Color32) {
    let t = ui.input(|i| i.time) as f32;
    let center = min + Vec2::splat(5.0);
    let start = (t / 0.8).fract() * std::f32::consts::TAU;
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = start + i as f32 / 24.0 * std::f32::consts::TAU * 0.75;
            center + vec2(a.cos(), a.sin()) * 4.0
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.0, color)));
    ui.ctx().request_repaint();
}

fn paint_bar(
    ui: &mut Ui,
    cx: &RowCx,
    bar: Rect,
    id: egui::Id,
    menu_open: bool,
    actions: &mut Vec<RowAction>,
) {
    let theme = Theme::of(ui.ctx());
    let bar = crate::widgets::snap(ui.ctx(), bar);
    let c = theme.c;
    let painter = ui.painter();
    let shadow = egui::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(41),
    };
    painter.add(shadow.as_shape(bar, corner(7.0)));
    painter.rect(
        bar,
        corner(7.0),
        c.panel,
        Stroke::new(1.0, c.border),
        StrokeKind::Inside,
    );
    let mut x = bar.left() + 3.0;
    for button in bar_buttons(cx) {
        let r = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(30.0, 28.0));
        let response = ui.interact(r, id.with(("bar", button.label)), Sense::click());
        let lit = response.hovered() || (menu_open && button.icon == Icon::More);
        if lit {
            ui.painter().rect_filled(r, corner(5.0), c.hover);
        }
        icons::paint_centered(
            ui.painter(),
            button.icon,
            r,
            button.size,
            if lit { c.text } else { c.muted },
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if !(menu_open && button.icon == Icon::More) {
            Tooltip::new(button.label).on_hover(&response, Side::Above);
        }
        if response.clicked() {
            actions.push(match button.icon {
                Icon::Smile => RowAction::Picker(cx.msg.id),
                Icon::Reply => RowAction::Reply(cx.msg.id),
                Icon::Pencil => RowAction::StartEdit(cx.msg.id),
                _ => RowAction::Menu(cx.msg.id, r),
            });
        }
        x += 32.0;
    }
}

fn editor_id(message: Uuid) -> egui::Id {
    egui::Id::unique(("gelabber-edit", message))
}

fn paint_editor(ui: &mut Ui, id: Uuid, rect: Rect, edit: Editing, actions: &mut Vec<RowAction>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let Editing { draft, autofocus } = edit;
    ui.painter().rect(
        rect,
        corner(7.0),
        c.panel,
        Stroke::new(1.0, c.accent),
        StrokeKind::Inside,
    );
    let edit_id = editor_id(id);
    let focused = ui.memory(|m| m.has_focus(edit_id));
    if focused && ui.input_mut(|i| super::consume_plain(i, egui::Key::Enter)) {
        actions.push(RowAction::SaveEdit(id));
    }
    // Esc drops egui's focus before the frame runs; it was the editor's.
    if (focused || ui.memory(|m| m.had_focus_last_frame(edit_id)))
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        actions.push(RowAction::CancelEdit);
    }
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = crate::text::job(buf.as_str(), EDIT, color, Some(wrap));
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let inner = rect.shrink(1.0);
    let output = ui
        .scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::TextEdit::multiline(draft)
                .id(edit_id)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    left: 9,
                    right: 9,
                    top: 7,
                    bottom: 7,
                }))
                .desired_width(inner.width())
                .min_size(inner.size())
                .layouter(&mut layouter)
                .show(ui)
        })
        .inner;
    if autofocus {
        output.response.request_focus();
        let mut state = output.state.clone();
        let end = egui::text::CCursor::new(draft.chars().count());
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(end)));
        state.store(ui.ctx(), edit_id);
    }
    // "Esc bricht ab · Enter speichert" · Abbrechen · Speichern: the
    // actions line starts 6 below the field; its text sits 4 below that.
    let line = rect.bottom() + 6.0;
    let y = line + 4.0;
    let hint = Block::single(
        ui,
        "Esc bricht ab · Enter speichert",
        TextStyle::new(12.0, 400.0, 1.5),
        c.muted,
    );
    hint.paint(ui.painter(), pos2(rect.left(), y));
    let label = TextStyle::new(12.0, 600.0, 1.5);
    let save = Block::single(ui, "Speichern", label, c.accent_ink);
    let cancel = Block::single(ui, "Abbrechen", label, c.text);
    let save_rect = Rect::from_min_size(
        pos2(rect.right() - save.width() - 20.0, line),
        vec2(save.width() + 20.0, 26.0),
    );
    let cancel_rect = Rect::from_min_size(
        pos2(save_rect.left() - 6.0 - cancel.width() - 20.0, line),
        vec2(cancel.width() + 20.0, 26.0),
    );
    let r_cancel = ui.interact(cancel_rect, edit_id.with("cancel"), Sense::click());
    if r_cancel.hovered() {
        ui.painter().rect_filled(cancel_rect, corner(5.0), c.hover);
    }
    cancel.paint(ui.painter(), pos2(cancel_rect.left() + 10.0, y));
    ui.painter().rect_filled(save_rect, corner(5.0), c.accent);
    save.paint(ui.painter(), pos2(save_rect.left() + 10.0, y));
    let r_save = ui.interact(save_rect, edit_id.with("save"), Sense::click());
    if r_cancel.clicked() {
        actions.push(RowAction::CancelEdit);
    }
    if r_save.clicked() {
        actions.push(RowAction::SaveEdit(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_answer_reads_as_unknown() {
        use gelabber_client::store::SendStage;
        let mut p =
            gelabber_client::PendingMessage::new(Uuid::nil(), "Hallo".into(), chrono::Utc::now());
        p.failed = Some(gelabber_client::ApiError::Transport("reset".into()));
        p.uncertain = true;
        let state = SendState::Failed {
            error: failure_line(&p),
            uncertain: p.uncertain,
        };
        assert_eq!(state.status(), "Status unklar – Verlauf prüfen");
        assert_eq!(
            state.failure().as_deref(),
            Some(
                "Nachricht speichern fehlgeschlagen. Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest."
            )
        );
        // A refused upload names the file.
        p.uncertain = false;
        p.stage = SendStage::Bind;
        p.files = vec![gelabber_client::OutgoingFile {
            filename: "liste.txt".into(),
            content_type: "text/plain".into(),
            data: Default::default(),
        }];
        p.failed = Some(gelabber_client::ApiError::Api {
            status: 422,
            code: "validation_failed".into(),
            message: String::new(),
            fields: serde_json::json!({"attachment_ids.0": "mismatch"}),
        });
        let state = SendState::Failed {
            error: failure_line(&p),
            uncertain: false,
        };
        assert_eq!(state.status(), "Nicht gesendet");
        assert_eq!(
            state.failure().as_deref(),
            Some(
                "Nachricht speichern fehlgeschlagen. liste.txt: Der Inhalt der Datei passt nicht zu ihrem Typ."
            )
        );
        p.stage = SendStage::Upload;
        p.failed = Some(gelabber_client::ApiError::Transport("timeout".into()));
        assert_eq!(failure_line(&p), "Dateiübertragung fehlgeschlagen.");
    }

    #[test]
    fn mentions_read_as_names() {
        let mut names = HashMap::new();
        names.insert(
            "3796349f-6f8f-4bcf-826f-d2237e2fe9d3".to_owned(),
            "Jonas".to_owned(),
        );
        assert_eq!(
            readable_mentions(
                "hi <@3796349F-6f8f-4bcf-826f-d2237e2fe9d3> und <@00000000-0000-0000-0000-000000000000>",
                &names
            ),
            "hi @Jonas und @Unbekannt"
        );
        assert_eq!(readable_mentions("a <@x> b", &names), "a <@x> b");
    }
}
