//! What the shell keeps between frames: where the user was last (web
//! servers/lastPlace.ts, lastChannel.ts, dms/lastDm.ts, per account), which
//! menu or dialog is open, and what the overview fetched. It lives in the
//! egui context under the shell's own id.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use gelabber_client::{ApiError, Invite, Message, SharedFile};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::task::Pending;

/// `Place` of lastPlace.ts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Server(Uuid),
    Dm(Uuid),
}

/// One account's memory.
#[derive(Clone, Debug, Default)]
pub struct Memory {
    pub place: Option<Place>,
    /// server → the channel last open in it.
    pub channels: HashMap<Uuid, Uuid>,
    pub dm: Option<Uuid>,
}

/// The step of the "Server hinzufügen" dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddStep {
    Choose,
    Create,
    Join,
}

/// How long an invite link works and how often (servers/inviteTerms.ts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms {
    /// Index into `EXPIRY`.
    pub expiry: usize,
    /// Index into `USES`.
    pub uses: usize,
}

impl Default for Terms {
    fn default() -> Self {
        // "7 Tage", "Unbegrenzt".
        Self { expiry: 2, uses: 3 }
    }
}

/// A link request's answer with the terms it asked for.
pub type Created = (Terms, Result<Invite, ApiError>);

pub struct InviteState {
    pub server: Uuid,
    pub name: String,
    pub terms: Terms,
    pub invite: Option<(Invite, Terms)>,
    pub creating: Option<Pending<Created>>,
    /// The number of the newest link request; older ones revoke their link.
    pub latest: Arc<AtomicU64>,
    pub error: Option<String>,
    /// The link was copied: it is not revoked when the terms change.
    pub handed_out: bool,
    pub status: Option<(String, crate::widgets::StatusTone)>,
    pub terms_open: bool,
}

pub enum Dialog {
    Add {
        step: AddStep,
        name: String,
        code: String,
        error: Option<String>,
        /// "create_server" was sent; the answer opens the server.
        sent: bool,
    },
    Invite(Box<InviteState>),
    Channel {
        server: Uuid,
        /// The channel being edited; `None` creates one.
        editing: Option<Uuid>,
        voice: bool,
        name: String,
        category: Option<Uuid>,
        error: Option<String>,
        /// "Zugriff": `None` is "Alle Mitglieder" (`AccessSelection`).
        access: Option<Vec<Uuid>>,
        /// The name and access it opened with, for the dirty check and
        /// to send `role_ids` only when the access changed.
        initial_name: String,
        initial_access: Option<Vec<Uuid>>,
    },
    Category {
        server: Uuid,
        editing: Option<Uuid>,
        name: String,
        error: Option<String>,
    },
    NewMessage {
        picker: super::overlays::picker::PickerState,
        /// The person picked; the dialog waits for their DM.
        waiting: Option<Uuid>,
        /// The servers without a loaded member list were asked for.
        loaded: bool,
    },
}

/// A channel's newest messages for the overview's "Verpasst" card.
pub struct Missed {
    /// The `latest_order` they were fetched for.
    pub order: Option<i64>,
    pub shown: Option<Vec<Message>>,
    pub pending: Option<Pending<Option<Vec<Message>>>>,
    /// After a failed fetch: when to ask again.
    pub retry_at: Option<std::time::Instant>,
}

/// A server's newest shared files for the overview's "Geteilt".
pub struct SharedFiles {
    /// The newest `latest_order` of its channels when they were fetched.
    pub key: Option<i64>,
    pub shown: Vec<SharedFile>,
    pub pending: Option<Pending<Option<Vec<SharedFile>>>>,
    /// After a failed fetch: when to ask again.
    pub retry_at: Option<std::time::Instant>,
}

#[derive(Default)]
pub struct ShellState {
    /// The account the rest belongs to; `memory` is kept per account.
    pub account: Option<Uuid>,
    pub memory: HashMap<Uuid, Memory>,
    pub dialog: Option<Dialog>,
    pub server_menu: bool,
    /// channel → the overview card's messages.
    pub missed: HashMap<Uuid, Missed>,
    /// server → the overview's shared files.
    pub shared: HashMap<Uuid, SharedFiles>,
    /// channel → the text in the overview's reply field.
    pub replies: HashMap<Uuid, String>,
    /// A picture of the overview's cards shown large.
    pub lightbox: Option<crate::screens::chat::attachments::Lightbox>,
    /// The location the route effects ran for last.
    pub effects_for: Option<String>,
    /// A server just created: its overview opens the invite dialog.
    pub invite_prompt: Option<(Uuid, String)>,
    /// The menu of a channel, category, server or conversation row.
    pub row_menu: Option<super::row_menu::Open>,
    /// Servers being left or deleted: still listed until the request ends.
    pub left: HashSet<Uuid>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<ShellState>>);

/// Runs `f` on the shell's state.
pub fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut ShellState) -> R) -> R {
    let shared = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Shared>(egui::Id::unique("gelabber-shell-state"))
            .clone()
    });
    let mut state = shared.0.lock();
    f(&mut state)
}

/// Another account signed in: its menus, dialogs and overview data start
/// empty; the remembered places stay, each under its own account.
pub fn switch_account(ctx: &egui::Context, user: Uuid) {
    with(ctx, |s| {
        if s.account != Some(user) {
            *s = ShellState {
                account: Some(user),
                memory: std::mem::take(&mut s.memory),
                ..ShellState::default()
            };
        }
    });
}

/// The memory of `user`.
pub fn memory(ctx: &egui::Context, user: Option<Uuid>) -> Memory {
    match user {
        Some(user) => with(ctx, |s| s.memory.get(&user).cloned().unwrap_or_default()),
        None => Memory::default(),
    }
}

pub fn remember(ctx: &egui::Context, user: Option<Uuid>, f: impl FnOnce(&mut Memory)) {
    if let Some(user) = user {
        with(ctx, |s| f(s.memory.entry(user).or_default()));
    }
}
