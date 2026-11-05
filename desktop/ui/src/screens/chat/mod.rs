//! Text channels and DMs: the page header and the message pane with its
//! composer (web components/MessagePane.tsx, pages/ChannelPage.tsx,
//! pages/DmChannelPage.tsx).

mod attachments;
mod composer;
mod content;
mod emoji;
mod gif;
mod header;
pub mod markdown;
mod mentions;
mod pane;
mod reactions;
mod row;
mod search;
pub mod time;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Rect, Ui, pos2, vec2};
use gelabber_client::{ChannelKind, Client, Permission};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::text::{Block, TextStyle};
use crate::theme::Theme;
use crate::widgets::{Button, Modal, Presence};
use composer::{ComposerAction, ComposerInput, ComposerState};
use content::ContentAction;
use mentions::Member;
use pane::{ListAction, ListInput, PaneState, Start};
use row::RowAction;

/// `TYPING_BAR_PX` (ws/live.ts).
const TYPING_BAR: f32 = 28.0;

#[derive(Default)]
struct ChatState {
    panes: HashMap<Uuid, PaneState>,
    composers: HashMap<Uuid, ComposerState>,
    searches: HashMap<Uuid, search::SearchState>,
    picker: emoji::PickerState,
    gif: gif::GifState,
    requests: Vec<ChatRequest>,
    bootstrapped: bool,
    requested_dm: HashSet<Uuid>,
    gifs_requested: bool,
    /// The conversation shown last and the frame it was shown in.
    shown: Option<(Uuid, u64)>,
}

fn chat_state(ctx: &egui::Context) -> Arc<Mutex<ChatState>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("gelabber-chat-state"), || {
            Arc::new(Mutex::new(ChatState::default()))
        })
        .clone()
    })
}

/// The workspace data the chat needs: started once, like the shell does.
fn ensure_workspace(client: &Client, chat: &mut ChatState) {
    if !chat.bootstrapped {
        chat.bootstrapped = true;
        // The client core keeps its own session: restoring it also loads
        // the workspace (and starts the gateway).
        let (unknown, idle) = client.with_state(|s| {
            (
                s.session == gelabber_client::Session::Unknown && s.op("session").is_none(),
                !s.bootstrapped && s.op("bootstrap").is_none(),
            )
        });
        if unknown {
            client.restore_session();
        } else if idle {
            client.bootstrap();
        }
    }
    if !chat.gifs_requested {
        chat.gifs_requested = true;
        if client.with_state(|s| s.gifs.is_none() && s.op("gifs").is_none()) {
            client.load_gif_status();
        }
    }
}

/// The unsent texts per conversation (messages/drafts.ts keeps them across
/// restarts; the desktop app stores them).
pub fn drafts(ctx: &egui::Context) -> HashMap<Uuid, String> {
    chat_state(ctx)
        .lock()
        .composers
        .iter()
        .filter(|(_, c)| !c.draft.is_empty())
        .map(|(id, c)| (*id, c.draft.clone()))
        .collect()
}

/// The conversations with an unsent draft (`hasDraft`: text beyond
/// whitespace, or files), for the sidebars' pencil.
pub fn with_drafts(ctx: &egui::Context) -> std::collections::HashSet<Uuid> {
    chat_state(ctx)
        .lock()
        .composers
        .iter()
        .filter(|(_, c)| !c.draft.trim().is_empty() || !c.files.is_empty())
        .map(|(id, _)| *id)
        .collect()
}

/// Puts stored texts back where this run has none yet.
pub fn restore_drafts(ctx: &egui::Context, drafts: HashMap<Uuid, String>) {
    let state = chat_state(ctx);
    let mut chat = state.lock();
    for (channel, text) in drafts {
        let composer = chat.composers.entry(channel).or_default();
        if composer.draft.is_empty() {
            composer.draft = text;
        }
    }
}

/// What the chat asks the rest of the app to open: dialogs it does not own.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatRequest {
    /// The header's bell (`NotificationSettingsDialog`).
    Notifications {
        channel: Uuid,
        server: Option<Uuid>,
        name: String,
    },
    /// The DM header's person (`UserProfileDialog`).
    Profile { user: Uuid },
}

/// Requests since the last call, for whoever draws those dialogs.
pub fn take_requests(ctx: &egui::Context) -> Vec<ChatRequest> {
    std::mem::take(&mut chat_state(ctx).lock().requests)
}

/// What differs between a server channel and a DM.
struct Conversation {
    channel: Uuid,
    server: Option<Uuid>,
    title: String,
    /// `#` or `@`.
    mention: char,
    start: Start,
    can_send: bool,
    can_send_files: bool,
    can_moderate: bool,
    can_everyone: bool,
    members: Vec<Member>,
    peer: Option<(String, Option<String>, Presence)>,
    muted: bool,
}

/// A server text channel: header and body, drawn into `ui.max_rect()`.
pub fn channel_page(ui: &mut Ui, app: &mut AppCtx, server: Uuid, channel: Uuid) {
    let chat = chat_state(ui.ctx());
    ensure_workspace(app.client, &mut chat.lock());
    app.client.open_server_channel(server, channel);
    let now = app.client.http().server_now();
    // None: the server is not loaded yet; Some(None): no such text channel.
    let conversation = app.client.with_state(|s| {
        let detail = s.detail(server)?;
        let ch = detail
            .channels
            .iter()
            .find(|c| c.id == channel && c.kind == ChannelKind::Text);
        let can = |p| detail.server.can(p);
        Some(ch.map(|ch| {
            Conversation {
                channel,
                server: Some(server),
                title: ch.name.clone(),
                mention: '#',
                start: Start::Channel(ch.name.clone()),
                can_send: can(Permission::SendMessages),
                can_send_files: can(Permission::SendFiles),
                can_moderate: can(Permission::ManageMessages),
                can_everyone: can(Permission::MentionEveryone),
                members: detail
                    .members
                    .iter()
                    .map(|m| Member {
                        id: m.user_id,
                        name: m.name.clone(),
                    })
                    .collect(),
                peer: None,
                muted: s.read.get(&channel).is_some_and(|r| r.is_muted(now)),
            }
        }))
    });
    match conversation {
        // Not loaded yet: the page background, like MainSkeleton's.
        None => {
            let theme = Theme::of(ui.ctx());
            ui.painter().rect_filled(ui.max_rect(), 0.0, theme.c.bg);
        }
        // The channel is gone (or is a voice channel, which the voice page draws).
        Some(None) => {
            let gone = app.client.with_state(|s| {
                s.detail(server)
                    .is_some_and(|d| !d.channels.iter().any(|c| c.id == channel))
            });
            if gone {
                app.nav.replace(&format!("/s/{server}"));
            }
        }
        Some(Some(c)) => show_conversation(ui, app, c),
    }
}

/// A DM conversation: header and body, drawn into `ui.max_rect()`.
pub fn dm_page(ui: &mut Ui, app: &mut AppCtx, channel: Uuid) {
    let chat = chat_state(ui.ctx());
    ensure_workspace(app.client, &mut chat.lock());
    let now = app.client.http().server_now();
    let found = app.client.with_state(|s| {
        let dm = s.dm(channel)?;
        let me = s.me();
        let presence = match s.presence_of(channel, dm.peer.id) {
            gelabber_client::PresenceStatus::Online => Presence::Online,
            gelabber_client::PresenceStatus::Idle => Presence::Away,
            _ => Presence::Offline,
        };
        let mut members = vec![Member {
            id: dm.peer.id,
            name: dm.peer.name.clone(),
        }];
        if let Some(me) = me {
            members.push(Member {
                id: me.id,
                name: me.name.clone(),
            });
        }
        Some(Conversation {
            channel,
            server: None,
            title: dm.peer.name.clone(),
            mention: '@',
            start: Start::Dm {
                name: dm.peer.name.clone(),
                avatar: dm.peer.avatar_url.clone(),
            },
            can_send: true,
            can_send_files: true,
            can_moderate: false,
            can_everyone: false,
            members,
            peer: Some((dm.peer.name.clone(), dm.peer.avatar_url.clone(), presence)),
            muted: s.read.get(&channel).is_some_and(|r| r.is_muted(now)),
        })
    });
    match found {
        Some(c) => {
            app.client.open_dm(channel);
            show_conversation(ui, app, c);
        }
        None => {
            // Not in the list (yet): ask for this one DM once.
            let loaded = app.client.with_state(|s| s.dms_loaded);
            let mut chat = chat.lock();
            if loaded && chat.requested_dm.insert(channel) {
                drop(chat);
                app.client.open_channel(channel);
            }
            let theme = Theme::of(ui.ctx());
            ui.painter().rect_filled(ui.max_rect(), 0.0, theme.c.bg);
        }
    }
}

fn show_conversation(ui: &mut Ui, app: &mut AppCtx, conv: Conversation) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let page = ui.max_rect();
    ui.painter().rect_filled(page, 0.0, c.bg);
    let chat = chat_state(ui.ctx());
    let mut chat = chat.lock();
    let chat = &mut *chat;
    let channel = conv.channel;
    let client = app.client;
    // Opening a conversation starts its list afresh (the web remounts the
    // pane): at the end or the first unread row, no menu, no edit.
    let frame = ui.ctx().cumulative_frame_nr();
    if chat
        .shown
        .is_none_or(|(c, f)| c != channel || f + 1 < frame)
    {
        chat.panes.insert(channel, PaneState::default());
    }
    chat.shown = Some((channel, frame));
    let me = client
        .with_state(|s| s.me().map(|u| u.id))
        .unwrap_or_default();
    let names: HashMap<String, String> = conv
        .members
        .iter()
        .map(|m| (m.id.to_string(), m.name.clone()))
        .collect();
    let gifs = client.with_state(|s| s.gifs.as_ref().is_some_and(|g| g.enabled));
    let unsent = client.with_state(|s| {
        s.messages(channel).map_or(0, |h| {
            h.pending.iter().filter(|p| p.failed.is_some()).count()
        })
    });

    // Header.
    let search_state = chat.searches.entry(channel).or_default();
    let title = match &conv.peer {
        Some((name, avatar, presence)) => header::Title::Dm {
            name,
            avatar: avatar.as_deref(),
            presence: *presence,
        },
        None => header::Title::Channel {
            name: &conv.title,
            voice: false,
        },
    };
    let head = header::show(
        ui,
        page,
        channel,
        title,
        conv.server.is_some(),
        conv.muted,
        &mut search_state.text,
        app.images,
    );
    if head.search_changed {
        search_state.changed(ui.input(|i| i.time));
    }
    if head.bell_clicked {
        chat.requests.push(ChatRequest::Notifications {
            channel,
            server: conv.server,
            name: if conv.server.is_some() {
                format!("#{}", conv.title)
            } else {
                conv.title.clone()
            },
        });
    }
    if head.peer_clicked
        && let Some(peer) = conv.members.first()
    {
        chat.requests.push(ChatRequest::Profile { user: peer.id });
    }

    // Composer height first: the list gives way to it.
    let composer_input = ComposerInput {
        channel,
        placeholder: format!("{}{}", conv.mention, conv.title),
        can_send: conv.can_send,
        can_send_files: conv.can_send_files,
        gifs,
        members: &conv.members,
        me: Some(me),
        keywords: conv.server.is_some() && conv.can_everyone,
        unsent,
    };
    let composer_state = chat.composers.entry(channel).or_default();
    let composer_h = composer::height(composer_state, &composer_input);
    let list_rect = Rect::from_min_max(
        pos2(page.left(), page.top() + header::HEIGHT),
        pos2(page.right(), page.bottom() - composer_h - TYPING_BAR),
    );
    let typing_rect = Rect::from_min_size(
        pos2(page.left(), list_rect.bottom()),
        vec2(page.width(), TYPING_BAR),
    );

    // The list, or the search hits over it.
    let now = client.http().server_now();
    let pane_state = chat.panes.entry(channel).or_default();
    let searching = chat
        .searches
        .get_mut(&channel)
        .is_some_and(|s| s.active(ui.input(|i| i.time)));
    let mut list_actions = Vec::new();
    if searching {
        let s = chat.searches.get_mut(&channel).expect("search state");
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        child.set_clip_rect(list_rect.intersect(ui.clip_rect()));
        match search::show(&mut child, list_rect, client, channel, s, &names, me, now) {
            Some(search::SearchAction::Close) => {
                s.text.clear();
                ui.memory_mut(|m| m.request_focus(header::search_id(channel)));
            }
            Some(search::SearchAction::Open(id)) => {
                s.text.clear();
                pane_state.focus = Some(id);
            }
            None => {}
        }
    } else {
        let state = client.state();
        let input = ListInput {
            channel,
            state: &state,
            me,
            names: &names,
            can_send: conv.can_send,
            can_moderate: conv.can_moderate,
            start: conv.start.clone(),
            now,
            images: app.images,
            http: client.http(),
        };
        list_actions = pane::show(ui, list_rect, &input, pane_state);
    }

    // Typing bar.
    let typing = client.with_state(|s| s.typing_text(channel, std::time::Instant::now()));
    paint_typing(ui, typing_rect, &typing);

    // Composer.
    let composer_rect = Rect::from_min_max(pos2(page.left(), page.bottom() - composer_h), page.max);
    let composer_state = chat.composers.entry(channel).or_default();
    let composer_actions = composer::show(ui, composer_rect, composer_state, &composer_input);

    // The rows' requests, now that the state lock is released.
    let pane_state = chat.panes.get_mut(&channel).expect("pane state");
    let now_t = ui.input(|i| i.time);
    for action in list_actions {
        match action {
            ListAction::LoadOlder => client.load_older(channel),
            ListAction::MarkRead => {
                if ui.ctx().input(|i| i.focused) {
                    client.mark_read(channel);
                }
            }
            ListAction::MarkReadNow => client.mark_read(channel),
            ListAction::Reload => client.reload_channel(channel),
            ListAction::Row(action) => {
                let composer_state = chat.composers.entry(channel).or_default();
                row_action(ui, app, &conv, pane_state, composer_state, action, now_t);
            }
        }
    }
    let composer_state = chat.composers.entry(channel).or_default();
    for action in composer_actions {
        match action {
            ComposerAction::Send { text, reply, files } => {
                if files.is_empty() {
                    client.send_message(channel, &text, reply);
                } else {
                    client.send_files(channel, &text, files, reply);
                }
                pane_state.to_latest = true;
            }
            ComposerAction::Typing(text) => client.typing_input(channel, &text),
            ComposerAction::StopTyping => client.stop_typing(),
            ComposerAction::EditLast => {
                let last = client.with_state(|s| {
                    s.messages(channel).and_then(|h| {
                        h.messages
                            .iter()
                            .rev()
                            .find(|m| m.author.id == me && m.gif.is_none() && m.system.is_none())
                            .map(|m| (m.id, m.content.clone()))
                    })
                });
                if let Some((id, content)) = last
                    && conv.can_send
                {
                    pane_state.start_edit(id, mentions::from_wire(&content, &conv.members));
                    pane_state.focus = Some(id);
                }
            }
            ComposerAction::ShowUnsent => {
                let first = client.with_state(|s| {
                    s.messages(channel).and_then(|h| {
                        h.pending
                            .iter()
                            .find(|p| p.failed.is_some())
                            .map(|p| p.nonce)
                    })
                });
                if let Some(id) = first {
                    pane_state.focus = Some(id);
                }
            }
        }
    }
    let _ = composer_state;

    let overlay_open = pane_state.menu.is_some()
        || pane_state.lightbox.is_some()
        || chat.composers.get(&channel).is_some_and(|c| c.gif_open)
        || pane_state.picker.is_some()
        || pane_state.confirm_delete.is_some()
        || chat.composers.get(&channel).is_some_and(|c| c.emoji_open)
        // A dialog over the page (quick switcher, profile…) owns Esc.
        || ui.ctx().memory(|m| m.top_modal_layer().is_some());
    overlays(ui, app, &conv, chat, me);
    // Esc nobody else used marks the conversation read; Shift+Esc the server.
    // Not while the search hits are shown, as on the web.
    if !overlay_open && !searching {
        let (esc, shift) = ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Escape) {
                (true, true)
            } else {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    false,
                )
            }
        });
        if esc {
            match (shift, conv.server) {
                (true, Some(server)) => client.mark_server_read(server),
                _ => client.mark_read(channel),
            }
            if let Some(pane) = chat.panes.get_mut(&channel) {
                pane.unread_count = None;
            }
        }
    }
}

fn row_action(
    ui: &mut Ui,
    app: &mut AppCtx,
    conv: &Conversation,
    pane: &mut PaneState,
    composer: &mut ComposerState,
    action: RowAction,
    now_t: f64,
) {
    let client = app.client;
    match action {
        RowAction::React(id, emoji, add) => {
            pane.intents.insert((id, emoji.clone()), (add, now_t));
            client.set_reaction(id, &emoji, add);
        }
        RowAction::Picker(id) => pane.picker = Some(id),
        RowAction::Reply(id) => {
            let author = client.with_state(|s| s.find_message(id).map(|m| m.author.name.clone()));
            if let Some(author) = author {
                composer.reply = Some((id, author));
                composer.focus = true;
            }
        }
        RowAction::StartEdit(id) => {
            let content = client.with_state(|s| s.find_message(id).map(|m| m.content.clone()));
            if let Some(content) = content {
                pane.start_edit(id, mentions::from_wire(&content, &conv.members));
            }
        }
        RowAction::Menu(id, anchor) => {
            pane.menu = if pane.menu.is_some_and(|(m, _)| m == id) {
                None
            } else {
                Some((id, anchor))
            };
        }
        RowAction::JumpTo(id) => {
            if client.with_state(|s| {
                s.messages(conv.channel)
                    .is_some_and(|h| h.get(id).is_some())
            }) {
                pane.focus = Some(id);
            } else {
                app.toasts
                    .notify("Die ursprüngliche Nachricht ist nicht geladen.");
            }
        }
        RowAction::Content(id, content) => match content {
            ContentAction::OpenLink(href) => open_external(ui.ctx(), &href),
            ContentAction::RevealSpoiler(n) => {
                pane.revealed.entry(id).or_default().insert(n);
            }
            ContentAction::CopyCode(n, text) => {
                ui.ctx().copy_text(text);
                pane.copied = Some((id, n, true, now_t));
            }
        },
        RowAction::Media(media) => match media {
            attachments::MediaAction::Open(url, name) => pane.lightbox = Some((url, name)),
            attachments::MediaAction::External(url) => {
                let full = if url.starts_with('/') {
                    client.http().url(&url)
                } else {
                    url
                };
                open_external(ui.ctx(), &full);
            }
        },
        RowAction::Retry(nonce) => client.retry_send(conv.channel, nonce),
        RowAction::Discard(nonce) => client.discard_pending(conv.channel, nonce),
        RowAction::TakeBack(nonce) => {
            let content = client.with_state(|s| {
                s.messages(conv.channel).and_then(|h| {
                    h.pending
                        .iter()
                        .find(|p| p.nonce == nonce)
                        .map(|p| (p.content.clone(), p.files.clone()))
                })
            });
            if let Some((text, files)) = content {
                let text = mentions::from_wire(&text, &conv.members);
                composer.draft = if composer.draft.trim().is_empty() {
                    text
                } else {
                    format!("{}\n{text}", composer.draft)
                };
                composer.files.extend(files);
                composer.focus = true;
                client.discard_pending(conv.channel, nonce);
            }
        }
        RowAction::SaveEdit(id) => {
            if let Some((_, draft)) = pane.editing.take() {
                let wire = mentions::to_wire(&draft, &conv.members, &HashMap::new());
                if !wire.trim().is_empty() {
                    client.edit_message(id, &wire);
                }
            }
        }
        RowAction::CancelEdit => pane.editing = None,
    }
}

/// Consumes `key` pressed without any modifier. egui's `consume_key`
/// ignores an extra Shift, so Shift+Enter would send instead of breaking the line.
fn consume_plain(input: &mut egui::InputState, key: egui::Key) -> bool {
    let before = input.events.len();
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::Key { key: k, modifiers, pressed: true, .. }
                if *k == key && modifiers.matches_exact(egui::Modifiers::NONE)
        )
    });
    input.events.len() != before
}

/// Opens a link in the system browser.
pub fn open_external(ctx: &egui::Context, url: &str) {
    ctx.open_url(egui::OpenUrl::new_tab(url));
    #[cfg(target_os = "linux")]
    let opener = Some("xdg-open");
    #[cfg(target_os = "macos")]
    let opener = Some("open");
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let opener: Option<&str> = None;
    if let Some(opener) = opener {
        let _ = std::process::Command::new(opener).arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
}

fn paint_typing(ui: &Ui, rect: Rect, text: &str) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    if text.is_empty() {
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let mut x = rect.left() + 22.0;
    for i in 0..3 {
        // 1.2s: 0.35 → 1 and 2px up at 30%, staggered by 0.15s.
        let phase = ((t - i as f32 * 0.15) / 1.2).rem_euclid(1.0);
        let k = if phase < 0.3 {
            phase / 0.3
        } else if phase < 0.6 {
            1.0 - (phase - 0.3) / 0.3
        } else {
            0.0
        };
        let alpha = 0.35 + 0.65 * k;
        ui.painter().circle_filled(
            pos2(x + 2.0, rect.center().y - 2.0 * k),
            2.0,
            c.muted.gamma_multiply(alpha),
        );
        x += 7.0;
    }
    x += 7.0 - 3.0;
    let mut job = crate::text::job(
        text,
        TextStyle::new(12.0, 400.0, 1.5),
        c.muted,
        Some(rect.right() - 22.0 - x),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let block = Block::new(ui, job, TextStyle::new(12.0, 400.0, 1.5));
    block.paint(ui.painter(), pos2(x, rect.center().y - 9.0));
    ui.ctx().request_repaint();
}

/// The menu of a row, the pickers and the delete confirmation.
fn overlays(ui: &mut Ui, app: &mut AppCtx, conv: &Conversation, chat: &mut ChatState, me: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let ctx = ui.ctx().clone();
    let client = app.client;
    let channel = conv.channel;
    let now_t = ui.input(|i| i.time);
    let pane = chat.panes.entry(channel).or_default();

    // Message menu: quick reactions, reply, edit, copy, delete.
    if let Some((id, anchor)) = pane.menu {
        let message = client.with_state(|s| s.messages(channel).and_then(|h| h.get(id).cloned()));
        let Some(message) = message else {
            pane.menu = None;
            return;
        };
        let mine = message.author.id == me;
        let can_react = conv.can_send;
        let can_edit = mine && conv.can_send && message.gif.is_none() && message.system.is_none();
        let can_delete = mine || conv.can_moderate;
        let reacted: Vec<String> = message
            .reactions
            .iter()
            .filter(|r| r.user_ids.contains(&me))
            .map(|r| r.emoji.clone())
            .collect();
        let mut chosen: Option<MenuChoice> = None;
        let open = crate::widgets::Popover::new(("message-menu", id), anchor)
            .align_end()
            .min_width(if can_react { 280.0 } else { 220.0 })
            .show(&ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                if can_react {
                    let (row, _) =
                        ui.allocate_exact_size(vec2(280.0, 2.0 + 36.0 + 8.0), egui::Sense::hover());
                    let mut x = row.left() + 2.0;
                    for emoji in ["👍", "❤️", "😂", "😮", "😢", "🎉"] {
                        let r = Rect::from_min_size(pos2(x, row.top() + 2.0), vec2(36.0, 36.0));
                        let resp = ui.interact(
                            r,
                            egui::Id::unique(("gelabber-quick", id, emoji)),
                            egui::Sense::click(),
                        );
                        let given = reacted.iter().any(|e| e == emoji);
                        let fill = if given {
                            c.selected
                        } else if resp.hovered() {
                            c.hover
                        } else {
                            c.surface
                        };
                        ui.painter().circle_filled(r.center(), 18.0, fill);
                        if given {
                            ui.painter().circle_stroke(
                                r.center(),
                                17.5,
                                egui::Stroke::new(1.0, c.accent),
                            );
                        }
                        let glyph =
                            Block::single(ui, emoji, TextStyle::new(18.0, 400.0, 1.5), c.text);
                        glyph.paint(ui.painter(), r.center() - glyph.size() / 2.0);
                        let label = if given {
                            format!("Reaktion {emoji} entfernen")
                        } else {
                            format!("Mit {emoji} reagieren")
                        };
                        crate::widgets::Tooltip::new(&label)
                            .on_hover(&resp, crate::widgets::tooltip::Side::Above);
                        if resp.clicked() {
                            chosen = Some(MenuChoice::React(emoji.to_owned(), !given));
                        }
                        x += 40.0;
                    }
                    let r = Rect::from_min_size(pos2(x, row.top() + 2.0), vec2(36.0, 36.0));
                    let resp = ui.interact(
                        r,
                        egui::Id::unique(("gelabber-quick-more", id)),
                        egui::Sense::click(),
                    );
                    ui.painter().circle_filled(
                        r.center(),
                        18.0,
                        if resp.hovered() { c.hover } else { c.surface },
                    );
                    crate::icons::paint_centered(
                        ui.painter(),
                        crate::icons::Icon::Plus,
                        r,
                        18.0,
                        if resp.hovered() { c.text } else { c.muted },
                    );
                    crate::widgets::Tooltip::new("Andere Reaktion auswählen")
                        .on_hover(&resp, crate::widgets::tooltip::Side::Above);
                    if resp.clicked() {
                        chosen = Some(MenuChoice::Picker);
                    }
                    ui.painter().rect_filled(
                        Rect::from_min_size(pos2(row.left(), row.bottom() - 1.0), vec2(280.0, 1.0)),
                        0.0,
                        c.border,
                    );
                    ui.add_space(4.0);
                }
                let item = |ui: &mut Ui, icon, label: &str, danger: bool| -> bool {
                    menu_row(ui, icon, label, danger)
                };
                if conv.can_send && item(ui, crate::icons::Icon::Reply, "Antworten", false) {
                    chosen = Some(MenuChoice::Reply);
                }
                if can_edit && item(ui, crate::icons::Icon::Pencil, "Bearbeiten", false) {
                    chosen = Some(MenuChoice::Edit);
                }
                if !message.content.is_empty()
                    && item(ui, crate::icons::Icon::Copy, "Text kopieren", false)
                {
                    chosen = Some(MenuChoice::Copy);
                }
                if can_delete {
                    ui.add_space(4.0);
                    let (line, _) = ui
                        .allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                    ui.painter().rect_filled(line, 0.0, c.border);
                    ui.add_space(4.0);
                    if item(ui, crate::icons::Icon::Trash, "Löschen", true) {
                        let shift = ui.input(|i| i.modifiers.shift);
                        chosen = Some(MenuChoice::Delete(shift));
                    }
                }
            });
        if open.is_none() {
            pane.menu = None;
        }
        if let Some(choice) = chosen {
            pane.menu = None;
            match choice {
                MenuChoice::React(emoji, add) => {
                    pane.intents.insert((id, emoji.clone()), (add, now_t));
                    client.set_reaction(id, &emoji, add);
                }
                MenuChoice::Picker => pane.picker = Some(id),
                MenuChoice::Reply => {
                    let composer = chat.composers.entry(channel).or_default();
                    composer.reply = Some((id, message.author.name.clone()));
                    composer.focus = true;
                }
                MenuChoice::Edit => {
                    pane.start_edit(id, mentions::from_wire(&message.content, &conv.members));
                }
                MenuChoice::Copy => {
                    ctx.copy_text(mentions::from_wire(&message.content, &conv.members));
                    app.toasts.notify("Text kopiert.");
                }
                MenuChoice::Delete(skip) => {
                    if skip {
                        client.delete_message(id);
                    } else {
                        pane.confirm_delete = Some(id);
                    }
                }
            }
        }
    }
    let pane = chat.panes.entry(channel).or_default();

    // Delete confirmation.
    if let Some(id) = pane.confirm_delete {
        let author = client.with_state(|s| {
            s.find_message(id)
                .map(|m| (m.author.id, m.author.name.clone()))
        });
        match author {
            None => pane.confirm_delete = None,
            Some((author_id, name)) => {
                let title = if author_id == me {
                    "Nachricht löschen?".to_owned()
                } else {
                    format!("Nachricht von {name} löschen?")
                };
                let mut confirm = false;
                let mut cancel = false;
                let output = Modal::new(("delete", id), &title).show(
                    &ctx,
                    |ui| {
                        crate::text::label(
                            ui,
                            "Das lässt sich nicht rückgängig machen.",
                            crate::text::BODY,
                            c.muted,
                        );
                    },
                    Some(&mut |ui: &mut Ui| {
                        if ui.add(Button::new("Löschen").danger()).clicked() {
                            confirm = true;
                        }
                        if ui.add(Button::new("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    }),
                );
                if confirm {
                    client.delete_message(id);
                }
                if confirm || cancel || output.close_requested {
                    pane.confirm_delete = None;
                }
            }
        }
    }

    // The reaction picker.
    if let Some(id) = pane.picker {
        match emoji::show(
            &ctx,
            ("reaction", id),
            "Reaktion auswählen",
            &mut chat.picker,
        ) {
            Some(Some(emoji)) => {
                let pane = chat.panes.entry(channel).or_default();
                pane.picker = None;
                let add = !client
                    .with_state(|s| s.find_message(id).is_some_and(|m| m.reacted(&emoji, me)));
                pane.intents.insert((id, emoji.clone()), (add, now_t));
                client.set_reaction(id, &emoji, add);
                chat.picker = Default::default();
            }
            Some(None) => {
                chat.panes.entry(channel).or_default().picker = None;
                chat.picker = Default::default();
            }
            None => {}
        }
    }
    // The composer's emoji picker.
    let open = chat.composers.get(&channel).is_some_and(|c| c.emoji_open);
    if open {
        match emoji::show(
            &ctx,
            ("composer", channel),
            "Emoji einfügen",
            &mut chat.picker,
        ) {
            Some(Some(emoji)) => {
                let composer = chat.composers.entry(channel).or_default();
                composer.emoji_open = false;
                composer.insert(&emoji);
                chat.picker = Default::default();
            }
            Some(None) => {
                chat.composers.entry(channel).or_default().emoji_open = false;
                chat.picker = Default::default();
            }
            None => {}
        }
    }
    // A picture shown large.
    let pane = chat.panes.entry(channel).or_default();
    if let Some((url, name)) = pane.lightbox.clone() {
        let (close, download) = attachments::lightbox(&ctx, app.images, &url, &name);
        if download {
            let full = if url.starts_with('/') {
                client.http().url(&url)
            } else {
                url.clone()
            };
            open_external(&ctx, &format!("{full}?download=1"));
        }
        if close {
            pane.lightbox = None;
        }
    }
    // The composer's GIF picker.
    let open = chat.composers.get(&channel).is_some_and(|c| c.gif_open);
    if open {
        match gif::show(&ctx, client, app.images, &mut chat.gif) {
            Some(Some(pick)) => {
                let composer = chat.composers.entry(channel).or_default();
                composer.gif_open = false;
                let reply = composer.reply.take().map(|(id, _)| id);
                composer.focus = true;
                client.send_gif(channel, pick, reply);
                chat.panes.entry(channel).or_default().to_latest = true;
                chat.gif = Default::default();
            }
            Some(None) => {
                chat.composers.entry(channel).or_default().gif_open = false;
                chat.gif = Default::default();
            }
            None => {}
        }
    }
    // My optimistic votes end once the server agrees, or after a while.
    let pane = chat.panes.entry(channel).or_default();
    pane.intents.retain(|(id, emoji), (add, since)| {
        let settled = client.with_state(|s| {
            s.find_message(*id)
                .is_some_and(|m| m.reacted(emoji, me) == *add)
        });
        !settled && now_t - *since < 8.0
    });
}

enum MenuChoice {
    React(String, bool),
    Picker,
    Reply,
    Edit,
    Copy,
    Delete(bool),
}

/// `.lr-message-menu-item`: gap 10, min-height 32, padding 0 10, icon 18.
fn menu_row(ui: &mut Ui, icon: crate::icons::Icon, label: &str, danger: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width().max(220.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, crate::theme::corner(5.0), c.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fg = if danger { c.danger } else { c.text };
    crate::icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 10.0, rect.center().y - 9.0),
        18.0,
        if danger { c.danger } else { c.muted },
    );
    let block = Block::single(ui, label, crate::text::UI, fg);
    block.paint(
        ui.painter(),
        pos2(
            rect.left() + 10.0 + 18.0 + 10.0,
            rect.center().y - block.height() / 2.0,
        ),
    );
    if danger {
        crate::widgets::Tooltip::new("Löschen (Umschalt: ohne Nachfrage)")
            .on_hover(&response, crate::widgets::tooltip::Side::Above);
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter(modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn shift_enter_is_not_a_plain_enter() {
        let mut input = egui::InputState::default();
        input.events = vec![enter(egui::Modifiers::SHIFT)];
        assert!(!consume_plain(&mut input, egui::Key::Enter));
        // It stays for the text field, which breaks the line.
        assert_eq!(input.events.len(), 1);
        input.events = vec![enter(egui::Modifiers::NONE)];
        assert!(consume_plain(&mut input, egui::Key::Enter));
        assert!(input.events.is_empty());
    }
}
