//! What the shell keeps between frames: where the user was last (web
//! servers/lastPlace.ts, lastChannel.ts, dms/lastDm.ts, per account), which
//! menu or dialog is open, and the overview's message previews. It lives
//! in the egui context under the shell's own id.

use std::collections::HashMap;
use std::sync::Arc;

use gelabber_client::{ApiError, Invite, Message};
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

pub struct InviteState {
    pub server: Uuid,
    pub name: String,
    pub terms: Terms,
    pub invite: Option<(Invite, Terms)>,
    pub creating: Option<Pending<Result<Invite, ApiError>>>,
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
    },
}

/// The newest message of a channel for the overview's unread list.
pub struct Preview {
    /// The `latest_order` it was fetched for.
    pub order: Option<i64>,
    pub shown: Option<Message>,
    pub pending: Option<Pending<Option<Message>>>,
}

#[derive(Default)]
pub struct ShellState {
    pub memory: HashMap<Uuid, Memory>,
    pub dialog: Option<Dialog>,
    pub server_menu: bool,
    pub user_menu: bool,
    /// channel → its preview.
    pub previews: HashMap<Uuid, Preview>,
    /// The location the route effects ran for last.
    pub effects_for: Option<String>,
    /// A server just created: its overview opens the invite dialog.
    pub invite_prompt: Option<(Uuid, String)>,
    /// The menu of a channel, category, server or conversation row.
    pub row_menu: Option<super::row_menu::Open>,
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
