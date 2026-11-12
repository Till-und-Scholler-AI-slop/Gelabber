//! Feedback to the instance's operators (#542; web/src/feedback): the
//! status of `GET /api/feedback/status` behind "Feedback geben" and its
//! dot, and the dialog with its draft. The status is read when the account
//! is first shown, every five minutes, when the window gets the focus back
//! and after the person's own changes; there is no gateway frame for it.

pub mod dialog;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Id;
use gelabber_client::{ApiError, FeedbackStatus};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::task::{Pending, poll};

/// `STATUS_REFETCH_MS`: a reply hours later needs no push, the mail tells
/// the person anyway.
const STALE: Duration = Duration::from_secs(5 * 60);

type Answer<T> = Pending<Result<T, ApiError>>;

/// `GET /api/feedback/status` for the signed-in account.
#[derive(Default)]
struct StatusCache {
    value: Option<FeedbackStatus>,
    /// The last read failed: without an earlier answer there is no item
    /// (`retry: false`) until a later read succeeds.
    failed: bool,
    read_at: Option<Instant>,
    request: Option<Answer<FeedbackStatus>>,
    /// Asked to read again while a read was on its way.
    again: bool,
}

#[derive(Default)]
struct State {
    account: Option<Uuid>,
    status: StatusCache,
    dialog: dialog::Dialog,
}

fn shared(ctx: &egui::Context) -> Arc<Mutex<State>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(Id::unique("gelabber-feedback"), || {
            Arc::new(Mutex::new(State::default()))
        })
        .clone()
    })
}

fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut State) -> R) -> R {
    let state = shared(ctx);
    let mut s = state.lock();
    f(&mut s)
}

fn account(app: &AppCtx) -> Option<Uuid> {
    app.client.with_state(|s| s.me().map(|m| m.id))
}

/// Another account: its status and its draft are not this one's.
fn follow_account(s: &mut State, account: Option<Uuid>) {
    if s.account != account {
        *s = State {
            account,
            ..State::default()
        };
    }
}

/// Takes a finished read, and reads when nothing is known yet, when the
/// last read is five minutes old (`refetchInterval`) or on `focused`
/// (`refetchOnWindowFocus`).
fn poll_status(ctx: &egui::Context, app: &AppCtx, s: &mut State, focused: bool) {
    let status = &mut s.status;
    if let Some(result) = poll(&mut status.request) {
        status.read_at = Some(Instant::now());
        match result {
            Ok(value) => {
                status.value = Some(value);
                status.failed = false;
            }
            Err(error) => {
                tracing::warn!("feedback status: {error}");
                status.failed = true;
            }
        }
        if std::mem::take(&mut status.again) {
            status.read_at = None;
        }
    }
    let due = match status.read_at {
        None => true,
        Some(at) => focused || at.elapsed() >= STALE,
    };
    if status.request.is_none() && due {
        let http = app.client.http().clone();
        status.request = Some(Pending::spawn(app.client, async move {
            http.feedback_status().await
        }));
        ctx.request_repaint();
    }
    // The interval runs while the app idles, as the web's timer does.
    if status.request.is_none()
        && let Some(at) = status.read_at
    {
        ctx.request_repaint_after(STALE.saturating_sub(at.elapsed()));
    }
}

/// The last answer, kept through a failed refresh as the web's query keeps
/// its data; with no answer a failed read counts as off.
fn shown(status: &StatusCache) -> Option<FeedbackStatus> {
    match (&status.value, status.failed) {
        (Some(value), _) => Some(*value),
        (None, true) => Some(FeedbackStatus::OFF),
        (None, false) => None,
    }
}

/// The status for the signed-in account, read as [`poll_status`] says;
/// `None` until the first answer.
fn status(ctx: &egui::Context, app: &AppCtx) -> Option<FeedbackStatus> {
    let account = account(app)?;
    let focused = ctx.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(true)))
    });
    with(ctx, |s| {
        follow_account(s, Some(account));
        poll_status(ctx, app, s, focused);
        shown(&s.status)
    })
}

/// Whether the instance takes feedback (`useFeedbackStatus().data?.enabled`):
/// `None` until the first answer.
pub fn enabled(ctx: &egui::Context, app: &AppCtx) -> Option<bool> {
    status(ctx, app).map(|s| s.enabled)
}

/// Entries with an operator reply the person has not read; 0 while off or
/// unknown.
pub fn unread(ctx: &egui::Context, app: &AppCtx) -> u32 {
    status(ctx, app)
        .filter(|s| s.enabled)
        .map_or(0, |s| s.unread)
}

/// After the person's own change (an entry sent, replies read): the status
/// is read again on the next frame.
pub fn refresh(ctx: &egui::Context) {
    with(ctx, |s| stale(&mut s.status));
    ctx.request_repaint();
}

/// The status is out of date: read again now, or after the read on its way.
fn stale(status: &mut StatusCache) {
    if status.request.is_some() {
        status.again = true;
    } else {
        status.read_at = None;
    }
}

/// "Feedback geben": opens the dialog on the page shown now, with the draft
/// as it was left (`openFeedbackDialog`).
pub fn open_dialog(ctx: &egui::Context, app: &AppCtx) {
    let path = app.nav.route().path();
    with(ctx, |s| s.dialog.open(&path));
    ctx.request_repaint();
}

/// Whether the dialog is open.
pub fn dialog_open(ctx: &egui::Context) -> bool {
    with(ctx, |s| s.dialog.is_open())
}
