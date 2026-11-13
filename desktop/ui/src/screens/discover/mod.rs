//! "Entdecken" (pages/DiscoverPage.tsx, pages/discover.css; reference
//! docs/design/v0.10/directory.png): the servers their owners put in the
//! directory, the language filter in the sidebar column, a search kept in
//! the address, one row per server with its button and "…" menu, and the
//! request and report dialogs. Only while the instance runs a directory
//! (`GET /api/directory`).

mod dialogs;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Id, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{
    ApiError, DirectoryLanguage, DirectoryStatus, JoinMode, Listing, ListingPage, OpState,
    RequestState,
};
use gelabber_i18n::discovery as texts;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::images::ImageState;
use crate::initials::{tile_initials, tile_tone};
use crate::route::{DiscoverSearch, Route};
use crate::screens::settings::controls;
use crate::screens::shell::row_menu::{self, Owner};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{
    Button, IconButton, Tooltip, eased, focus_ring_inset, keyboard_focus, snap, tooltip::Side,
};

/// `QUERY_MAX` of web/src/discover/rules.ts.
pub const QUERY_MAX: usize = DiscoverSearch::QUERY_MAX;
/// `SEARCH_DEBOUNCE_MS`: typing waits this long before the search moves.
const DEBOUNCE: f64 = 0.25;
/// `STALE_MS`: the status and the listings are read again after a minute.
const STALE: Duration = Duration::from_secs(60);

type Answer<T> = Pending<Result<T, ApiError>>;

/// `GET /api/directory` for the signed-in account.
#[derive(Default)]
struct StatusCache {
    value: Option<DirectoryStatus>,
    /// The last read failed: without an earlier answer the tile stays
    /// away (`retry: false`) until a read after a minute, the window's
    /// focus or a visit of /discover succeeds.
    failed: bool,
    read_at: Option<Instant>,
    request: Option<Answer<DirectoryStatus>>,
}

/// The rows of one search, 30 at a time.
#[derive(Default)]
struct List {
    /// The search being shown (`q`, `lang`).
    key: Option<DiscoverSearch>,
    /// The search `rows` and `next` came from: until a new search has its
    /// own, the previous one's rows stay (`keepPreviousData`).
    rows_key: Option<DiscoverSearch>,
    rows: Option<Vec<Listing>>,
    next: Option<u32>,
    read_at: Option<Instant>,
    /// The search being read, with its offset (0: from the top).
    request: Option<(DiscoverSearch, u32, Answer<ListingPage>)>,
    /// The last read failed; `more` when it was a further page.
    failed: Option<bool>,
    /// Read again on the next frame (`jn`, a refused join).
    stale: bool,
}

#[derive(Default)]
struct State {
    account: Option<Uuid>,
    status: StatusCache,
    list: List,
    /// The search field, what it settled on, and when it last changed.
    input: String,
    typed: String,
    changed_at: f64,
    /// The `q` of the address last seen.
    seen: Option<String>,
    /// The pass the page was last drawn in: a new visit reads again.
    drawn: u64,
    /// The pass the route last checked the status in (`requireDirectory`).
    guarded: u64,
    /// "Beitreten" in flight (`join_listing:<id>` ops).
    joining: HashSet<Uuid>,
    withdrawing: Option<(Uuid, Answer<()>)>,
    request: Option<dialogs::RequestForm>,
    /// A request whose dialog closed while it was being sent.
    request_sent: Option<(Uuid, Answer<gelabber_client::RequestAnswer>)>,
    report: Option<dialogs::ReportForm>,
}

fn shared(ctx: &egui::Context) -> Arc<Mutex<State>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(Id::unique("gelabber-discover"), || {
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

/// Everything of another account is dropped.
fn follow_account(s: &mut State, account: Option<Uuid>) {
    if s.account != account {
        *s = State {
            account,
            ..State::default()
        };
    }
}

// --------------------------------------------------------------- status

/// Reads the status when nothing is known for the account yet, or (`fresh`)
/// when the last read is older than a minute; takes a finished read.
fn poll_status(ctx: &egui::Context, app: &AppCtx, s: &mut State, fresh: bool) {
    let status = &mut s.status;
    if let Some(result) = poll(&mut status.request) {
        status.read_at = Some(Instant::now());
        match result {
            Ok(value) => {
                status.value = Some(value);
                status.failed = false;
            }
            Err(error) => {
                tracing::warn!("directory status: {error}");
                status.failed = true;
            }
        }
    }
    let unread = status.read_at.is_none();
    let old = fresh && status.read_at.is_some_and(|at| at.elapsed() >= STALE);
    if status.request.is_none() && (unread || old) {
        let http = app.client.http().clone();
        status.request = Some(Pending::spawn(app.client, async move {
            http.directory_status().await
        }));
        ctx.request_repaint();
    }
}

/// The last answer's `enabled`, kept through a failed refresh as the
/// web's query keeps its data; with no answer a failed read counts as off.
fn shown(status: &StatusCache) -> Option<bool> {
    match (&status.value, status.failed) {
        (Some(value), _) => Some(value.enabled),
        (None, true) => Some(false),
        (None, false) => None,
    }
}

/// Whether the instance runs a directory for the signed-in account:
/// `None` until the first answer (`useDirectoryStatus().data?.enabled`).
/// Once a minute old the status is read again when the window gets the
/// focus back (`refetchOnWindowFocus`), and after a failed read on any
/// frame, so
/// a blip or a directory turned on later does not hide the tile for good.
pub fn enabled(ctx: &egui::Context, app: &AppCtx) -> Option<bool> {
    let account = account(app)?;
    let focused = ctx.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(true)))
    });
    with(ctx, |s| {
        follow_account(s, Some(account));
        // No timer of its own: an idle app stays idle, and the next frame
        // after the minute reads again.
        poll_status(ctx, app, s, focused || s.status.failed);
        shown(&s.status)
    })
}

/// The /discover route's check (`requireDirectory`): as [`enabled`], but
/// on the way in a failed read with no answer is read again first, as
/// `ensureQueryData` fetches what it does not have.
pub fn guard(ctx: &egui::Context, app: &AppCtx) -> Option<bool> {
    account(app)?;
    with(ctx, |s| {
        let frame = ctx.cumulative_frame_nr();
        let visit = s.guarded + 1 < frame;
        s.guarded = frame;
        let status = &mut s.status;
        if visit && status.failed && status.value.is_none() && status.request.is_none() {
            status.failed = false;
            status.read_at = None;
        }
    });
    enabled(ctx, app)
}

/// `jn`: a request was accepted, so the rows read again.
pub fn joined(ctx: &egui::Context) {
    with(ctx, |s| s.list.stale = true);
    ctx.request_repaint();
}

/// A listing was saved in the server settings (`refreshDirectory`): the
/// rows and the status, with its language counts, read again.
pub fn listing_saved(ctx: &egui::Context) {
    with(ctx, |s| {
        s.list.stale = true;
        s.status.read_at = None;
    });
    ctx.request_repaint();
}

// -------------------------------------------------------------- sidebar

/// `.sidebar-brand`: 700 16px, letter-spacing −0.3px.
const BRAND: TextStyle = TextStyle::new(16.0, 700.0, 1.5).tracking(-0.3 / 16.0);

/// `DiscoverSidebar`: the brand line and "SPRACHE" with "Alle" and every
/// language that has listings, in the settings sidebar's rows.
pub fn sidebar(ui: &mut Ui, app: &mut AppCtx, search: &DiscoverSearch) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let column = ui.max_rect();
    let languages: Vec<DirectoryLanguage> = with(ui.ctx(), |s| {
        s.status
            .value
            .as_ref()
            .map(|v| {
                v.languages
                    .iter()
                    .filter(|l| l.listings > 0)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    });
    let fill = Rect::from_min_max(column.min, pos2(column.right() - 1.0, column.bottom()));
    ui.painter().rect_filled(fill, 0.0, c.panel);
    // The <nav aria-label>; hover-only, so its rows keep their clicks.
    let region = ui.interact(
        fill,
        Id::unique("gelabber-discover-sidebar"),
        Sense::hover(),
    );
    crate::widgets::announce(&region, egui::Role::Navigation, texts::sidebar_label());
    crate::widgets::rule_h(
        ui.painter(),
        c.border,
        column.left()..=column.right() - 1.0,
        column.top() + 51.0,
    );
    controls::paint(
        ui,
        pos2(column.left() + 16.0, column.top() + 13.5),
        texts::page_title(),
        BRAND,
        c.text,
    );
    let x = column.left() + 8.0;
    let width = column.width() - 1.0 - 16.0;
    let mut y = column.top() + 52.0 + 4.0;
    let caps = texts::sidebar_language().to_uppercase();
    controls::paint(
        ui,
        pos2(x + 10.0, y + 34.0 - 7.0 - text::CAPS.line_px()),
        &caps,
        text::CAPS,
        c.muted,
    );
    // The <h2> the language list is labelled by.
    let heading = ui.interact(
        Rect::from_min_size(pos2(x, y), vec2(width, 34.0)),
        Id::unique("gelabber-discover-language-heading"),
        Sense::hover(),
    );
    crate::widgets::announce(&heading, egui::Role::Heading, texts::sidebar_language());
    y += 34.0;
    let mut picked: Option<Option<String>> = None;
    let rows = std::iter::once((None, texts::sidebar_all().to_owned())).chain(
        languages
            .iter()
            .map(|l| (Some(l.code.clone()), l.name.clone())),
    );
    for (code, label) in rows {
        let rect = Rect::from_min_size(pos2(x, y), vec2(width, 34.0));
        let on = search.lang == code;
        if language_row(ui, rect, code.as_deref(), &label, on) && !on {
            picked = Some(code);
        }
        y += 34.0;
    }
    if let Some(lang) = picked {
        let route = Route::Discover(DiscoverSearch {
            q: search.q.clone(),
            lang,
        });
        app.nav.replace(&route.path());
    }
}

/// One language button (`aria-pressed`), drawn as a settings nav row.
fn language_row(ui: &mut Ui, rect: Rect, code: Option<&str>, label: &str, on: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let id = Id::unique(("gelabber-discover-language", code));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
    let hover = eased(
        ui.ctx(),
        id.with("hover"),
        response.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        corner(theme.radius.md),
        if on { c.selected } else { hover },
    );
    if on {
        let bar = Rect::from_min_max(
            pos2(rect.left(), rect.top() + 7.0),
            pos2(rect.left() + 3.0, rect.bottom() - 7.0),
        );
        painter.rect_filled(
            bar,
            egui::CornerRadius {
                nw: 0,
                sw: 0,
                ne: 3,
                se: 3,
            },
            c.accent,
        );
    }
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring_inset(painter, rect, theme.radius.md, &theme);
    }
    let style = if on {
        text::BODY.weight(600.0)
    } else {
        text::BODY
    };
    let color = if on || response.hovered() {
        c.text
    } else {
        c.muted
    };
    Block::single(ui, label, style, color)
        .paint(painter, pos2(rect.left() + 10.0, rect.top() + 6.5));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

// ----------------------------------------------------------------- page

/// `.discover-page-head h1`: 650 20px, letter-spacing −0.2px.
const TITLE: TextStyle = TextStyle::new(20.0, 650.0, 1.5).tracking(-0.2 / 20.0);
/// `.discover-row-text h2`: 600 16px / 20px.
const NAME: TextStyle = TextStyle::new(16.0, 600.0, 20.0 / 16.0);
/// `.discover-row-description`: 13px / 18px.
const DESCRIPTION: TextStyle = TextStyle::new(13.0, 400.0, 18.0 / 13.0);
/// `.discover-row-meta`: `--lr-type-meta` at 16px.
const META: TextStyle = TextStyle::new(12.0, 400.0, 16.0 / 12.0);
/// `.discover-tile`: 600 18px.
const LETTERS: TextStyle = TextStyle::new(18.0, 600.0, 1.0);

/// What a row's button offers (`listingAction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A member: "Öffnen".
    Open,
    /// An open server: "Beitreten".
    Join,
    /// "Beitritt anfragen".
    Request,
    /// Pending: "Angefragt", disabled.
    Requested,
    /// The cooldown: "Nicht angenommen", disabled.
    Declined,
}

pub fn action(listing: &Listing) -> Action {
    if listing.member {
        return Action::Open;
    }
    // An open server lets anyone in, also someone who asked while it was not.
    if listing.join_mode == JoinMode::Open {
        return Action::Join;
    }
    match listing.request {
        Some(RequestState::Pending) => Action::Requested,
        Some(RequestState::Declined) => Action::Declined,
        None => Action::Request,
    }
}

/// `validateQuery`: one line of at most [`QUERY_MAX`] characters.
fn query_ok(raw: &str) -> bool {
    let value = raw.trim();
    value.chars().count() <= QUERY_MAX && !value.chars().any(char::is_control)
}

/// Rewrites one server's row.
fn patch(s: &mut State, server: Uuid, update: impl Fn(&mut Listing)) {
    for listing in s.list.rows.iter_mut().flatten() {
        if listing.server_id == server {
            update(listing);
        }
    }
}

/// Reads `search` from `offset` (0: anew, keeping the old rows on screen
/// until the new ones arrive, as `keepPreviousData`).
fn read(app: &AppCtx, s: &mut State, search: &DiscoverSearch, offset: u32) {
    let http = app.client.http().clone();
    let (q, lang) = (search.q.clone(), search.lang.clone());
    let pending = Pending::spawn(app.client, async move {
        http.listings(q.as_deref(), lang.as_deref(), offset).await
    });
    s.list.request = Some((search.clone(), offset, pending));
    s.list.stale = false;
}

/// The search field follows the address and the address the field
/// (`DiscoverPage`'s `seen`, `typed` and the effect after it).
fn follow_search(ui: &Ui, app: &mut AppCtx, s: &mut State, search: &DiscoverSearch) {
    let q = search.q.clone().unwrap_or_default();
    let now = ui.input(|i| i.time);
    if s.seen.as_deref() != Some(q.as_str()) {
        let first = s.seen.is_none();
        s.seen = Some(q.clone());
        // A search the page did not write (back, the rail's tile) replaces
        // the field.
        if first || (q != s.typed && q != s.input.trim()) {
            s.input = q.clone();
            s.typed = q.clone();
        }
    }
    let trimmed = s.input.trim().to_owned();
    if trimmed != s.typed {
        let left = DEBOUNCE - (now - s.changed_at);
        if left <= 0.0 {
            s.typed = trimmed.clone();
        } else {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(left.max(0.01)));
        }
    }
    if s.typed == trimmed && s.typed != q && query_ok(&s.typed) {
        let route = Route::Discover(DiscoverSearch::new(Some(&s.typed), search.lang.as_deref()));
        app.nav.replace(&route.path());
    }
}

/// The page column (`DiscoverPage`): the empty header line, the title,
/// the search and the rows; the dialogs over it.
pub fn page(ui: &mut Ui, app: &mut AppCtx, search: &DiscoverSearch) {
    let ctx = ui.ctx().clone();
    let state = shared(&ctx);
    let mut guard = state.lock();
    let s = &mut *guard;
    follow_account(s, account(app));
    let frame = ctx.cumulative_frame_nr();
    let visit = s.drawn + 1 < frame;
    s.drawn = frame;
    poll_status(&ctx, app, s, visit);
    if visit {
        s.seen = None;
        s.list.stale |= s.list.read_at.is_none_or(|at| at.elapsed() >= STALE);
    }
    follow_search(ui, app, s, search);
    take_answers(&ctx, app, s, search);
    // The rows wait for the status, as the route's `beforeLoad` does: off,
    // the page is not found and reads nothing.
    let on = s.status.value.as_ref().is_some_and(|v| v.enabled);
    let new_search = s.list.key.as_ref() != Some(search);
    if on && (new_search || (s.list.stale && s.list.request.is_none())) {
        if new_search {
            s.list.key = Some(search.clone());
            s.list.failed = None;
        }
        read(app, s, search, 0);
    }

    let rect = ui.max_rect();
    // The shell's header line, empty as in the reference.
    controls::header(ui, rect, "", None);
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let languages = s
        .status
        .value
        .as_ref()
        .map(|v| v.languages.clone())
        .unwrap_or_default();
    controls::scroll(ui, body, "discover", None, |ui| {
        let left = ui.cursor().min.x;
        let width = (ui.available_width().min(940.0) - 80.0).max(0.0);
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(left + 40.0, ui.cursor().min.y + 27.0),
                    vec2(width, f32::INFINITY),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        content(&mut child, app, s, &languages, search);
        let used = child.min_rect();
        ui.advance_cursor_after_rect(Rect::from_min_max(
            pos2(left, body.top()),
            pos2(left + width + 80.0, used.bottom() + 40.0),
        ));
    });
    dialogs::show(&ctx, app, s);
}

/// What finished since the last frame: the rows, a join, a withdrawal.
fn take_answers(ctx: &egui::Context, app: &mut AppCtx, s: &mut State, search: &DiscoverSearch) {
    let done = s
        .list
        .request
        .as_mut()
        .and_then(|(key, offset, pending)| pending.take().map(|r| (key.clone(), *offset, r)));
    if let Some((key, offset, result)) = done {
        s.list.request = None;
        if &key == search {
            s.list.read_at = Some(Instant::now());
            match result {
                Ok(page) => {
                    s.list.failed = None;
                    s.list.next = page.next_offset;
                    match (&mut s.list.rows, offset) {
                        (Some(rows), o) if o > 0 => rows.extend(page.listings),
                        (rows, _) => *rows = Some(page.listings),
                    }
                    s.list.rows_key = Some(key);
                }
                Err(error) => {
                    tracing::warn!("directory listings: {error}");
                    s.list.failed = Some(offset > 0);
                    // Another search's rows are not this one's: the error
                    // takes their place.
                    if s.list.rows_key.as_ref() != Some(&key) {
                        s.list.rows = None;
                        s.list.next = None;
                        s.list.rows_key = None;
                    }
                }
            }
        }
    }
    for server in s.joining.clone() {
        let key = format!("join_listing:{server}");
        let state = app.client.with_state(|st| st.op(&key).cloned());
        match state {
            Some(OpState::Done) => {
                s.joining.remove(&server);
                app.client.clear_op(key);
                patch(s, server, |l| {
                    if !l.member {
                        l.member_count += 1;
                    }
                    l.member = true;
                    l.request = None;
                    l.retry_at = None;
                });
                app.nav.navigate(&Route::Server(server).path());
            }
            Some(OpState::Failed(error)) => {
                s.joining.remove(&server);
                app.client.clear_op(key);
                if let Some(text) = crate::toasts::error_text(&error) {
                    app.toasts.error(text);
                }
                s.list.stale = true;
            }
            Some(OpState::Pending) => {}
            None => {
                s.joining.remove(&server);
            }
        }
    }
    let withdrawn = s
        .withdrawing
        .as_mut()
        .and_then(|(server, pending)| pending.take().map(|r| (*server, r)));
    if let Some((server, result)) = withdrawn {
        s.withdrawing = None;
        match result {
            Ok(()) => patch(s, server, |l| {
                l.request = None;
                l.retry_at = None;
            }),
            Err(error) => {
                if let Some(text) = crate::toasts::error_text(&error) {
                    app.toasts.error(text);
                }
                s.list.stale = true;
            }
        }
    }
    dialogs::take_answers(ctx, app, s);
}

fn content(
    ui: &mut Ui,
    app: &mut AppCtx,
    s: &mut State,
    languages: &[DirectoryLanguage],
    search: &DiscoverSearch,
) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let title = Block::single(ui, texts::page_title(), TITLE, c.text);
    let (at, response) = ui.allocate_exact_size(vec2(width, title.height()), Sense::hover());
    title.paint(ui.painter(), at.min);
    crate::widgets::announce(&response, egui::Role::Heading, texts::page_title());
    ui.add_space(4.0);
    controls::paragraph(ui, texts::page_lead(), text::BODY, c.muted, width);
    ui.add_space(20.0);
    search_field(ui, s, width.min(420.0));
    ui.add_space(18.0);

    let loading = s.list.request.as_ref().is_some_and(|(_, o, _)| *o == 0);
    match (&s.list.rows, s.list.failed) {
        (None, Some(_)) => {
            if crate::widgets::load_error(ui, texts::page_load_failed(), loading) && !loading {
                read(app, s, search, 0);
            }
        }
        (None, None) => crate::widgets::skeleton::rows(ui, 4),
        (Some(rows), _) if rows.is_empty() => {
            let block = Block::wrapped(ui, texts::page_empty(), text::BODY, c.muted, width);
            let (rect, response) = ui.allocate_exact_size(block.size(), Sense::hover());
            block.paint(ui.painter(), rect.min);
            crate::widgets::announce(&response, egui::Role::Label, texts::page_empty());
        }
        (Some(rows), _) => {
            let rows = rows.clone();
            let top = ui.cursor().min.y;
            crate::widgets::rule_h(
                ui.painter(),
                c.border,
                ui.cursor().min.x..=ui.cursor().min.x + width,
                top,
            );
            ui.add_space(1.0);
            for listing in &rows {
                row(ui, app, s, listing, languages, width);
            }
            // The <ul aria-label>; hover-only, so its rows keep their clicks.
            let list = Rect::from_min_max(
                pos2(ui.cursor().min.x, top),
                pos2(ui.cursor().min.x + width, ui.cursor().min.y),
            );
            let list = ui.interact(list, Id::unique("gelabber-discover-list"), Sense::hover());
            crate::widgets::announce(&list, egui::Role::List, texts::page_list_label());
            let more = s.list.request.as_ref().is_some_and(|(_, o, _)| *o > 0);
            // A failed read shows under the rows: a further page tries that
            // page again, a refresh the first.
            if let Some(further) = s.list.failed {
                ui.add_space(20.0);
                let busy = more || loading;
                if crate::widgets::load_error(ui, texts::page_load_failed(), busy) && !busy {
                    match (further, s.list.next) {
                        (true, Some(next)) => read(app, s, search, next),
                        _ => read(app, s, search, 0),
                    }
                }
            } else if let Some(next) = s.list.next
                // The previous search's offset is not this one's, nor one
                // from before a refresh still on its way.
                && !loading
                && s.list.rows_key.as_ref() == Some(search)
            {
                ui.add_space(20.0);
                let clicked = ui
                    .allocate_ui_with_layout(
                        vec2(width, 36.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.add(
                                Button::new(texts::page_more())
                                    .pending(more.then_some(texts::page_more())),
                            )
                            .clicked()
                        },
                    )
                    .inner;
                if clicked && !more {
                    read(app, s, search, next);
                }
            }
        }
    }
}

/// `.discover-search`: 40px, the magnifier 12px in, the text after 38px.
fn search_field(ui: &mut Ui, s: &mut State, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let id = crate::widgets::field::field_id("discover-search");
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, 40.0));
    let focused = ui.memory(|m| m.has_focus(id));
    let shown = snap(ui.ctx(), rect);
    let painter = ui.painter().clone();
    painter.rect(
        shown,
        corner(theme.radius.sm),
        c.bg,
        egui::Stroke::new(1.0, if focused { c.focus } else { c.border_strong }),
        egui::StrokeKind::Inside,
    );
    if focused {
        crate::widgets::field_focus_ring(&painter, shown, theme.radius.sm, c.focus);
    }
    icons::paint_centered(
        &painter,
        Icon::Search,
        Rect::from_center_size(
            pos2(rect.left() + 12.0 + 8.0, rect.center().y),
            vec2(16.0, 16.0),
        ),
        16.0,
        c.muted,
    );
    let color = c.text;
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            buf.as_str().to_owned(),
            text::BODY.format(color),
        );
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let inner = rect.shrink(1.0);
    let edit = egui::TextEdit::singleline(&mut s.input)
        .id(id)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: 38,
            right: 11,
            top: 9,
            bottom: 7,
        }))
        .desired_width(inner.width())
        .min_size(inner.size())
        .align(egui::Align2::LEFT_TOP)
        .char_limit(QUERY_MAX)
        .layouter(&mut layouter)
        .hint_text(egui::text::LayoutJob::single_section(
            texts::page_search().to_owned(),
            text::BODY.format(c.subtle),
        ));
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| edit.show(ui))
        .inner
        .response
        .response
        .accessible_name(texts::page_search());
    if response.changed() {
        s.changed_at = ui.input(|i| i.time);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    ui.advance_cursor_after_rect(rect);
}

/// `DiscoverRow`: the tile, name, description and "Deutsch · 48
/// Mitglieder", the button and the "…".
fn row(
    ui: &mut Ui,
    app: &mut AppCtx,
    s: &mut State,
    listing: &Listing,
    languages: &[DirectoryLanguage],
    width: f32,
) {
    let action = action(listing);
    let joining = s.joining.contains(&listing.server_id);
    let (response, more) = draw_row(ui, app, listing, languages, width, joining, true);
    if let Some((more, more_response)) = more
        && (more_response.clicked() || row_menu::menu_key(ui, &more_response))
    {
        row_menu::open(
            ui.ctx(),
            Owner::Listing(
                listing.server_id,
                listing.name.clone(),
                listing.request == Some(RequestState::Pending),
            ),
            more,
            Some(more_response.id),
        );
    }
    if response.clicked() {
        match action {
            Action::Open => app.nav.navigate(&Route::Server(listing.server_id).path()),
            Action::Join if !joining => {
                s.joining.insert(listing.server_id);
                app.client.join_listing(listing.server_id);
            }
            Action::Request => dialogs::open_request(s, listing),
            _ => {}
        }
    }
}

/// A row as others see it in Entdecken, for the server settings'
/// "Vorschau": drawn as on the page, but its buttons take no input and
/// screen readers skip it (`inert` on the web).
pub fn preview(ui: &mut Ui, app: &AppCtx, listing: &Listing, languages: &[DirectoryLanguage]) {
    let width = ui.available_width();
    let opacity = ui.opacity();
    ui.scope(|ui| {
        ui.disable();
        ui.ctx()
            .accesskit_node_builder(ui.unique_id(), |node| node.set_hidden());
        // Disabled, but painted as it is.
        ui.set_opacity(opacity);
        draw_row(ui, app, listing, languages, width, false, false);
    });
}

/// The languages `GET /api/directory` names, once it answered.
pub fn languages(ctx: &egui::Context, app: &AppCtx) -> Vec<DirectoryLanguage> {
    enabled(ctx, app);
    with(ctx, |s| {
        s.status
            .value
            .as_ref()
            .map(|v| v.languages.clone())
            .unwrap_or_default()
    })
}

/// Paints a row and places its button and, with `menu`, the "…" (a row
/// without `onReport` has none): the button's response, and where the "…"
/// sits with its response.
fn draw_row(
    ui: &mut Ui,
    app: &AppCtx,
    listing: &Listing,
    languages: &[DirectoryLanguage],
    width: f32,
    joining: bool,
    menu: bool,
) -> (egui::Response, Option<(Rect, egui::Response)>) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let left = ui.cursor().min.x;
    let top = ui.cursor().min.y;
    let action = action(listing);
    let label = match action {
        Action::Open => texts::actions_open(),
        Action::Join => texts::actions_join(),
        Action::Request => texts::actions_request(),
        Action::Requested => texts::actions_requested(),
        Action::Declined => texts::actions_declined(),
    };
    let mut widget = Button::new(label);
    widget = match action {
        Action::Join => widget
            .primary()
            .pending(joining.then_some(texts::actions_joining())),
        Action::Requested => widget.disabled(true),
        // Focusable, so its tooltip shows on keyboard focus too.
        Action::Declined => widget.unavailable_focusable(),
        _ => widget,
    };
    let button_w = widget.natural_width(ui);
    let menu_w = if menu { 16.0 + 32.0 } else { 0.0 };
    let text_w = (width - 56.0 - 16.0 - 16.0 - button_w - menu_w).max(40.0);
    let language = languages
        .iter()
        .find(|l| l.code == listing.language)
        .map_or(listing.language.as_str(), |l| l.name.as_str());
    let members = texts::row_members(listing.member_count.max(0) as u64);
    let meta = if language.is_empty() {
        members
    } else {
        texts::row_meta(language, &members)
    };
    let name = Block::wrapped(ui, &listing.name, NAME, c.text, text_w);
    let description = Block::wrapped(ui, &listing.description, DESCRIPTION, c.text, text_w);
    let meta = Block::wrapped(ui, &meta, META, c.muted, text_w);
    let text_h = name.height() + 1.0 + description.height() + 1.0 + meta.height();
    let inner_h = text_h.max(56.0);
    let height = 14.0 + inner_h + 13.0 + 1.0;
    let rect = Rect::from_min_size(pos2(left, top), vec2(width, height));
    let mid = top + 14.0 + inner_h / 2.0;
    let painter = ui.painter().clone();
    crate::widgets::rule_h(&painter, c.border, left..=left + width, rect.bottom() - 1.0);

    // A server tile as in the rail, but square.
    let tile = Rect::from_min_size(pos2(left, mid - 28.0), vec2(56.0, 56.0));
    let texture = listing
        .icon_url
        .as_deref()
        .and_then(|url| match app.images.get(ui.ctx(), url) {
            ImageState::Ready(texture) => Some(texture),
            _ => None,
        });
    match texture {
        Some(texture) => {
            painter.add(
                egui::epaint::RectShape::filled(tile, corner(theme.radius.lg), Color32::WHITE)
                    .with_texture(
                        texture.id(),
                        crate::images::cover_uv(texture.size_vec2(), tile.size()),
                    ),
            );
        }
        None => {
            let id = listing.server_id.to_string();
            painter.rect_filled(tile, corner(theme.radius.lg), theme.tile(tile_tone(&id)));
            let letters = Block::single(ui, &tile_initials(&listing.name), LETTERS, c.text);
            letters.paint(
                &painter,
                pos2(
                    tile.center().x - letters.width() / 2.0,
                    tile.center().y - letters.height() / 2.0,
                ),
            );
        }
    }

    let x = left + 56.0 + 16.0;
    let mut y = mid - text_h / 2.0;
    let heading = ui.interact(
        Rect::from_min_size(pos2(x, y), name.size()),
        Id::unique(("gelabber-discover-name", listing.server_id)),
        Sense::hover(),
    );
    crate::widgets::announce(&heading, egui::Role::Heading, &listing.name);
    name.paint(&painter, pos2(x, y));
    y += name.height() + 1.0;
    description.paint(&painter, pos2(x, y));
    y += description.height() + 1.0;
    meta.paint(&painter, pos2(x, y));

    // The "…" at the right edge, the button 16px before it.
    let more = menu.then(|| {
        let more = Rect::from_min_size(pos2(left + width - 32.0, mid - 16.0), vec2(32.0, 32.0));
        let menu_label = texts::actions_more(&listing.name);
        let response = ui.add(
            IconButton::new(Icon::More, &menu_label)
                .size(32.0)
                .icon_size(18.0)
                .at(more),
        );
        (more, response)
    });
    let button = Rect::from_min_size(
        pos2(left + width - menu_w - button_w, mid - 18.0),
        vec2(button_w, 36.0),
    );
    let response = ui.put(button, widget.width(button_w));
    if action == Action::Declined
        && let Some(at) = listing.retry_at
    {
        let until = texts::actions_declined_until(&crate::rules::format_day(at));
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_description(until.clone()));
        Tooltip::new(texts::actions_declined())
            .detail(Some(&until))
            .on_hover_or_focus(&response, Side::Above);
    }
    // Last: the placed button and "…" moved the cursor to their bottom.
    ui.allocate_rect(rect, Sense::hover());
    (response, more)
}

/// The row menu's "Melden".
pub fn open_report(ctx: &egui::Context, server: Uuid, name: &str) {
    with(ctx, |s| dialogs::open_report(s, server, name));
    ctx.request_repaint();
}

/// The row menu's "Anfrage zurückziehen"; one at a time, as the web's
/// entry is disabled while one runs.
pub fn withdraw(ctx: &egui::Context, app: &AppCtx, server: Uuid) {
    with(ctx, |s| {
        if s.withdrawing.is_some() {
            return;
        }
        let http = app.client.http().clone();
        let pending = Pending::spawn(
            app.client,
            async move { http.withdraw_request(server).await },
        );
        s.withdrawing = Some((server, pending));
    });
}
