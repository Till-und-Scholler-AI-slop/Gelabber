//! Sicherheit → "Anmeldung" (pages/SecurityArea.tsx `SignInSection`): the
//! password row and one row per sign-in provider, as settings action rows
//! (components/ActionRow.tsx), with their dialogs and confirmations. The
//! rows keep their own state so the section can move as a whole.
//!
//! "Verbinden" and "Mit Google bestätigen" run the provider flow through
//! the system browser (screens/auth/oidc.rs); the web leaves the page for
//! the provider and comes back with `?connected=` or `?oidc_error=`, the
//! app stays and shows the answer on the row or in the dialog.

use std::collections::HashMap;

use egui::{Rect, Sense, Ui, Vec2, pos2, vec2};
use gelabber_client::{
    ApiError, AuthProvider, OidcPurpose, OidcStart, SessionResponse, SignInMethods, SignInProvider,
};
use gelabber_i18n::{common, settings as texts};

use super::controls::{self, Confirm, Status};
use super::settled;
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, error_message, field_message, validate_password};
use crate::screens::auth::oidc::{self, Done, Flow, poll_flow};
use crate::task::{Pending, poll};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, FormError, InlineStatus, Modal, StatusTone, TextField};

/// Where a provider flow started from Sicherheit comes back to.
pub const SECURITY_PATH: &str = "/settings/konto";

type Request<T> = Option<Pending<Result<T, ApiError>>>;

/// What a provider flow reported on arrival (`signInFeedback`):
/// `?connected={provider}` or `?oidc_error={code}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    pub connected: Option<String>,
    pub oidc_error: Option<&'static str>,
}

impl Feedback {
    pub fn read(connected: Option<&str>, oidc_error: Option<&str>) -> Self {
        let connected = connected
            .filter(|c| {
                (1..=32).contains(&c.len())
                    && c.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
            })
            .map(str::to_owned);
        Self {
            connected,
            oidc_error: crate::rules::oidc_error(oidc_error),
        }
    }
}

/// `signInProvider`: the provider a password-less account signs in with.
pub fn sign_in_provider(methods: &SignInMethods) -> Option<AuthProvider> {
    let connected: Vec<&SignInProvider> = methods
        .providers
        .iter()
        .filter(|p| p.connected.is_some())
        .collect();
    connected
        .iter()
        .find(|p| p.configured)
        .or_else(|| connected.first())
        .map(|p| AuthProvider {
            id: p.id.clone(),
            name: p.name.clone(),
        })
}

/// `oidcErrorRows`: an `oidc_error` shows on the rows that are not
/// connected (only those can have started a link), failing that the first.
pub fn oidc_error_rows(methods: &SignInMethods) -> Vec<String> {
    let open: Vec<String> = methods
        .providers
        .iter()
        .filter(|p| p.connected.is_none())
        .map(|p| p.id.clone())
        .collect();
    if !open.is_empty() {
        return open;
    }
    methods
        .providers
        .first()
        .map(|p| vec![p.id.clone()])
        .unwrap_or_default()
}

/// The section's state, per signed-in account.
#[derive(Default)]
pub struct SignInState {
    started: bool,
    load: Request<SignInMethods>,
    pub methods: Option<SignInMethods>,
    load_failed: bool,
    feedback: Feedback,
    password: PasswordRow,
    rows: HashMap<String, ProviderRow>,
}

impl SignInState {
    /// For tests and previews: the section with these methods.
    pub fn with_methods(methods: SignInMethods) -> Self {
        Self {
            started: true,
            methods: Some(methods),
            ..Self::default()
        }
    }
}

fn load(app: &AppCtx) -> Request<SignInMethods> {
    let http = app.client.http().clone();
    Some(Pending::spawn(app.client, async move {
        http.sign_in_methods().await
    }))
}

/// `SignInSection`: the title, then the rows once the methods are known,
/// a load error with "Erneut versuchen", or two skeleton rows.
pub fn section(ui: &mut Ui, app: &mut AppCtx, s: &mut SignInState) {
    let theme = Theme::of(ui.ctx());
    if !s.started {
        s.started = true;
        let location = app.nav.location();
        s.feedback = Feedback::read(location.query("connected"), location.query("oidc_error"));
        s.load = load(app);
    }
    if let Some(result) = poll(&mut s.load) {
        match result {
            Ok(methods) => {
                s.methods = Some(methods);
                s.load_failed = false;
            }
            Err(_) => s.load_failed = true,
        }
    }
    // `.settings-action-section`: margin-top 24; the title 4px over the list.
    ui.add_space(24.0);
    let top = ui.cursor().min;
    controls::paint(
        ui,
        top,
        texts::security_sign_in_title(),
        text::TITLE_SM,
        theme.c.text,
    );
    let (title, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), text::TITLE_SM.line_px()),
        Sense::hover(),
    );
    let heading = ui.interact(title, ui.scope_id().with("sign-in-heading"), Sense::hover());
    crate::widgets::announce(
        &heading,
        egui::Role::Heading,
        texts::security_sign_in_title(),
    );
    ui.add_space(4.0);
    let Some(methods) = s.methods.clone() else {
        if s.load_failed || s.load.is_none() {
            let pending = s.load.is_some();
            if crate::widgets::load_error(ui, texts::security_sign_in_load_failed(), pending)
                && !pending
            {
                s.load = load(app);
            }
        } else {
            skeleton_rows(ui, 2);
        }
        return;
    };
    let width = ui.available_width();
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        ui.cursor().min.x..=ui.cursor().min.x + width,
        ui.cursor().min.y,
    );
    ui.add_space(1.0);
    password_row(ui, app, s, &methods);
    let error_rows = if s.feedback.oidc_error.is_some() {
        oidc_error_rows(&methods)
    } else {
        Vec::new()
    };
    for provider in &methods.providers {
        let feedback = if s.feedback.connected.as_deref() == Some(provider.id.as_str()) {
            Some((
                texts::security_sign_in_provider_just_connected(&provider.name),
                StatusTone::Success,
            ))
        } else {
            s.feedback
                .oidc_error
                .filter(|_| error_rows.contains(&provider.id))
                .map(|code| {
                    (
                        crate::rules::oidc_error_message(code).to_owned(),
                        StatusTone::Error,
                    )
                })
        };
        provider_row(ui, app, s, provider, methods.password, feedback);
    }
}

/// `SkeletonRows`: lines of 10px, 8px apart, 16px between the rows.
fn skeleton_rows(ui: &mut Ui, count: usize) {
    let theme = Theme::of(ui.ctx());
    let width = ui.available_width();
    let height = count as f32 * (10.0 + 8.0 + 10.0) + (count - 1) as f32 * 16.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    crate::widgets::announce(&response, egui::Role::Status, common::loading());
    let mut y = rect.top();
    for _ in 0..count {
        for share in [0.4, 0.7] {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(rect.left(), y), vec2(width * share, 10.0)),
                corner(theme.radius.sm),
                theme.c.surface,
            );
            y += 10.0 + 8.0;
        }
        y += 16.0 - 8.0;
    }
}

// ----------------------------------------------------------- action row

/// The button on the right of an [`action_row`].
pub struct RowButton<'a> {
    pub label: &'a str,
    pub pending: Option<&'a str>,
    pub disabled: bool,
}

impl<'a> RowButton<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            pending: None,
            disabled: false,
        }
    }
}

/// `.settings-action-row__title`: `--lr-type-ui` at weight 600.
const ROW_TITLE: text::TextStyle = text::UI.weight(600.0);

/// `ActionRow`: title and hint on the left, the status line under them,
/// a small button on the right; min-height 52, padding 8, bottom hairline.
/// True when the button was clicked.
pub fn action_row(
    ui: &mut Ui,
    title: &str,
    hint: &str,
    status: Option<(&str, StatusTone)>,
    button: Option<RowButton<'_>>,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let button_w = button.as_ref().map_or(0.0, |b| {
        let label = b.pending.unwrap_or(b.label);
        Block::single(ui, label, text::UI, c.text).width() + 2.0 * 12.0 + 2.0
    });
    let gap = if button.is_some() { 16.0 } else { 0.0 };
    let text_w = (width - button_w - gap).max(1.0);
    let title_block = Block::single(ui, title, ROW_TITLE, c.text);
    let hint_block = Block::wrapped(ui, hint, controls::HINT, c.muted, text_w);
    let status_h = status.map_or(0.0, |(message, _)| {
        let block = Block::wrapped(ui, message, text::META, c.muted, text_w - 20.0);
        4.0 + block.height().max(20.0)
    });
    let content_h = ROW_TITLE.line_px() + hint_block.height() + status_h;
    let height = (content_h + 17.0).max(52.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    crate::widgets::rule_h(
        ui.painter(),
        c.border,
        rect.left()..=rect.right(),
        rect.bottom() - 1.0,
    );
    // The content box is 8px under the top and over the hairline.
    let middle = rect.top() + 8.0 + (height - 17.0) / 2.0;
    let mut y = middle - content_h / 2.0;
    title_block.paint(ui.painter(), pos2(rect.left(), y));
    let label = ui.interact(
        Rect::from_min_size(pos2(rect.left(), y), title_block.size()),
        ui.scope_id().with(("action-row-title", title)),
        Sense::hover(),
    );
    crate::widgets::announce(&label, egui::Role::Label, title);
    y += ROW_TITLE.line_px();
    hint_block.paint(ui.painter(), pos2(rect.left(), y));
    let label = ui.interact(
        Rect::from_min_size(pos2(rect.left(), y), hint_block.size()),
        ui.scope_id().with(("action-row-hint", title)),
        Sense::hover(),
    );
    crate::widgets::announce(&label, egui::Role::Label, hint);
    y += hint_block.height();
    if let Some((message, tone)) = status {
        let at = Rect::from_min_size(pos2(rect.left(), y + 4.0), vec2(text_w, status_h - 4.0));
        ui.put(at, InlineStatus::new(Some(message), tone));
    }
    let Some(button) = button else {
        // `put` leaves the cursor under what it placed: back under the row.
        ui.advance_cursor_after_rect(rect);
        return false;
    };
    let at = Rect::from_min_size(
        pos2(rect.right() - button_w, middle - 16.0),
        vec2(button_w, 32.0),
    );
    let clicked = ui
        .put(
            at,
            Button::new(button.label)
                .small()
                .width(button_w)
                .disabled(button.disabled)
                .pending(button.pending),
        )
        .clicked();
    // `put` leaves the cursor under what it placed: back under the row.
    ui.advance_cursor_after_rect(rect);
    clicked
}

// --------------------------------------------------------- password row

#[derive(Default)]
struct PasswordRow {
    status: Status,
    open: bool,
    current: String,
    password: String,
    errors: FormErrors,
    /// "Ändern" runs the client's `change_password` (op `password`).
    changing: bool,
    /// "Festlegen".
    setting: Request<SessionResponse>,
    /// The API wants a fresh sign-in first: "Mit … bestätigen".
    reauth: bool,
    confirm: Option<Flow>,
}

impl PasswordRow {
    fn close(&mut self) {
        let status = std::mem::take(&mut self.status);
        *self = Self {
            status,
            ..Self::default()
        };
    }
}

const CHANGE_ORDER: [&str; 2] = ["current_password", "password"];

fn password_row(ui: &mut Ui, app: &mut AppCtx, s: &mut SignInState, methods: &SignInMethods) {
    let row = &mut s.password;
    // Answers of the running requests.
    if row.changing
        && let Some(result) = settled(app, "password")
    {
        row.changing = false;
        match result {
            Ok(()) => {
                if let Some(updated) = app.client.with_state(|st| st.me().cloned()) {
                    super::adopt_user(app, &updated);
                }
                row.close();
                row.status.success(ui, texts::security_changed());
            }
            Err(e) => row
                .errors
                .from_failure(&Failure::from_api(&e), &CHANGE_ORDER),
        }
    }
    if let Some(result) = poll(&mut row.setting) {
        match result {
            Ok(_) => {
                if let Some(methods) = &mut s.methods {
                    methods.password = true;
                }
                row.close();
                row.status
                    .success(ui, texts::security_sign_in_password_set_done());
            }
            Err(e) if Failure::from_api(&e).code == "reauth_required" => {
                row.errors = FormErrors::default();
                row.reauth = true;
            }
            Err(e) => row
                .errors
                .from_failure(&Failure::from_api(&e), &["password"]),
        }
    }
    if let Some(result) = poll_flow(&mut row.confirm) {
        match result {
            // Signed in afresh: "Festlegen" goes through now.
            Ok(Done::SignedIn(session)) => {
                app.session.adopt(&session);
                row.reauth = false;
                row.errors = FormErrors::default();
            }
            Ok(_) => {}
            Err(error) => oidc::fail(&mut row.errors, &error, &["password"]),
        }
    }

    let provider = sign_in_provider(methods);
    let none = provider
        .as_ref()
        .map(|p| texts::security_sign_in_password_none(&p.name));
    let hint = if methods.password {
        texts::security_sign_in_password_hint()
    } else {
        none.as_deref()
            .unwrap_or(texts::security_sign_in_password_unset())
    };
    let label = if methods.password {
        texts::security_sign_in_password_change()
    } else {
        texts::security_sign_in_password_set()
    };
    let status = row.status.current(ui);
    if action_row(
        ui,
        texts::security_sign_in_password(),
        hint,
        status.as_ref().map(|(m, t)| (m.as_str(), *t)),
        Some(RowButton::new(label)),
    ) {
        row.status.clear();
        row.open = true;
    }
    if row.open {
        if methods.password {
            change_dialog(ui, app, row);
        } else {
            set_dialog(ui, app, row, provider.as_ref());
        }
    }
}

/// The dialog's actions: "Abbrechen" and the submit button, right-aligned.
fn dialog_actions(ui: &mut Ui, submit: Button<'_>) -> (bool, bool) {
    let mut out = (false, false);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            out.1 = ui.add(submit.primary()).clicked();
            out.0 = ui.add(Button::new(common::cancel()).ghost()).clicked();
        },
    );
    out
}

fn dialog_error(ui: &mut Ui, errors: &FormErrors) {
    if let Some(message) = &errors.form {
        let theme = Theme::of(ui.ctx());
        ui.add(FormError::new(message).on(theme.c.elevated));
        ui.add_space(20.0);
    }
}

/// "Ändern": today's current and new password form, in a dialog.
fn change_dialog(ui: &mut Ui, app: &mut AppCtx, row: &mut PasswordRow) {
    let ctx = ui.ctx().clone();
    let dirty = !row.current.is_empty() || !row.password.is_empty();
    let output = Modal::new("change-password", texts::security_change())
        .dirty(dirty)
        .show(
            &ctx,
            |ui| {
                let current = TextField::new("current_password", &mut row.current)
                    .label(texts::security_current_password())
                    .password()
                    .autofocus(true)
                    .error(row.errors.field("current_password"))
                    .show(ui);
                if current.response.changed() {
                    row.errors.clear_field("current_password");
                }
                ui.add_space(20.0);
                let new = TextField::new("new_password", &mut row.password)
                    .label(texts::security_new_password())
                    .password()
                    .error(row.errors.field("password"))
                    .hint(Some(texts::security_new_password_hint()))
                    .show(ui);
                if new.response.changed() {
                    row.errors.clear_field("password");
                }
                ui.add_space(20.0);
                dialog_error(ui, &row.errors);
                let (cancel, clicked) = dialog_actions(
                    ui,
                    Button::new(texts::security_change())
                        .pending(row.changing.then_some(texts::security_changing())),
                );
                (cancel, clicked || current.submitted || new.submitted)
            },
            None,
        );
    let (cancel, submit) = output.inner;
    if cancel || output.close_requested {
        row.close();
        return;
    }
    if submit && !row.changing {
        let mut fields = Vec::new();
        if row.current.is_empty() {
            fields.push((
                "current_password",
                field_message("current_password", "required"),
            ));
        }
        if let Some(code) = validate_password(&row.password) {
            fields.push(("password", field_message("password", code)));
        }
        if fields.is_empty() {
            row.errors = FormErrors::default();
            row.changing = true;
            app.client.change_password(&row.current, &row.password);
        } else {
            row.errors.set_fields(fields);
        }
    }
    row.errors.focus = None;
}

/// "Festlegen": the first password of an account that signs in through a
/// provider. A refusal for an old sign-in offers "Mit … bestätigen".
fn set_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    row: &mut PasswordRow,
    provider: Option<&AuthProvider>,
) {
    let ctx = ui.ctx().clone();
    let setting = row.setting.is_some();
    let confirm_label = row.confirm.as_ref().map(Flow::label);
    let mut confirm = false;
    let output = Modal::new("set-password", texts::security_sign_in_password_set_title())
        .dirty(!row.password.is_empty())
        .show(
            &ctx,
            |ui| {
                let new = TextField::new("new_password", &mut row.password)
                    .label(texts::security_new_password())
                    .password()
                    .autofocus(true)
                    .error(row.errors.field("password"))
                    .hint(Some(texts::security_new_password_hint()))
                    .show(ui);
                if new.response.changed() {
                    row.errors.clear_field("password");
                }
                ui.add_space(20.0);
                if row.reauth {
                    confirm = reauth_note(ui, provider, confirm_label.as_deref());
                    ui.add_space(20.0);
                }
                dialog_error(ui, &row.errors);
                let (cancel, clicked) = dialog_actions(
                    ui,
                    Button::new(texts::security_sign_in_password_set_submit())
                        .pending(setting.then_some(texts::security_sign_in_password_setting())),
                );
                (cancel, clicked || new.submitted)
            },
            None,
        );
    let (cancel, submit) = output.inner;
    if cancel || output.close_requested {
        row.close();
        return;
    }
    if confirm
        && row.confirm.is_none()
        && let Some(provider) = provider
    {
        row.errors = FormErrors::default();
        let start = OidcStart {
            redirect: Some(SECURITY_PATH.to_owned()),
            ..OidcStart::new(OidcPurpose::SignIn)
        };
        row.confirm = Some(Flow::start(&ctx, app.client, provider, start));
    }
    if submit && !setting {
        if let Some(code) = validate_password(&row.password) {
            row.errors
                .set_fields(vec![("password", field_message("password", code))]);
        } else {
            row.errors = FormErrors::default();
            let http = app.client.http().clone();
            let password = row.password.clone();
            row.setting = Some(Pending::spawn(app.client, async move {
                http.set_password(&password).await
            }));
        }
    }
    row.errors.focus = None;
}

/// `.settings-row-note` with `role="alert"`: why, and "Mit … bestätigen".
/// True when that was clicked.
fn reauth_note(ui: &mut Ui, provider: Option<&AuthProvider>, pending: Option<&str>) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let message = error_message("reauth_required");
    let style = controls::HINT;
    let block = Block::wrapped(ui, message, style, c.muted, width - 24.0);
    let button_label = provider.map(|p| texts::security_sign_in_reauth(&p.name));
    let button_h = if button_label.is_some() {
        10.0 + 32.0
    } else {
        0.0
    };
    let height = 10.0 + block.height() + button_h + 10.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    crate::widgets::announce(&response, egui::Role::Alert, message);
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.surface);
    block.paint(ui.painter(), pos2(rect.left() + 12.0, rect.top() + 10.0));
    let Some(label) = button_label else {
        return false;
    };
    let shown = pending.unwrap_or(&label);
    let w = Block::single(ui, shown, text::UI, c.text).width() + 2.0 * 12.0 + 2.0;
    let at = Rect::from_min_size(
        pos2(
            rect.left() + 12.0,
            rect.top() + 10.0 + block.height() + 10.0,
        ),
        vec2(w, 32.0),
    );
    let clicked = ui
        .put(at, Button::new(&label).small().width(w).pending(pending))
        .clicked();
    ui.advance_cursor_after_rect(rect);
    clicked
}

// --------------------------------------------------------- provider row

#[derive(Default)]
struct ProviderRow {
    status: Status,
    /// The row did something since the page opened: its own status shows
    /// instead of what the address reported.
    acted: bool,
    disconnect: Request<SignInMethods>,
    /// A link flow started from the row (no password to confirm with).
    flow: Option<Flow>,
    dialog: Option<ConnectDialog>,
}

#[derive(Default)]
struct ConnectDialog {
    current: String,
    errors: FormErrors,
    flow: Option<Flow>,
}

fn disconnect_key(id: &str) -> String {
    format!("disconnect-provider-{id}")
}

fn provider_row(
    ui: &mut Ui,
    app: &mut AppCtx,
    s: &mut SignInState,
    provider: &SignInProvider,
    has_password: bool,
    feedback: Option<(String, StatusTone)>,
) {
    let ctx = ui.ctx().clone();
    let row = s.rows.entry(provider.id.clone()).or_default();
    let own = AuthProvider {
        id: provider.id.clone(),
        name: provider.name.clone(),
    };
    // Answers.
    let mut methods_now = None;
    if let Some(result) = poll(&mut row.disconnect) {
        row.acted = true;
        match result {
            Ok(methods) => {
                methods_now = Some(methods);
                row.status
                    .success(ui, texts::security_sign_in_provider_disconnected());
            }
            Err(e) => row
                .status
                .error(ui, error_message(&Failure::from_api(&e).code)),
        }
    }
    let linked = |row: &mut ProviderRow, ui: &Ui, result| match result {
        Ok(Done::Linked(methods)) => {
            row.acted = true;
            row.status.success(
                ui,
                texts::security_sign_in_provider_just_connected(&provider.name),
            );
            Some(methods)
        }
        Ok(_) => None,
        Err(error) => {
            row.acted = true;
            row.status.error(ui, oidc::message(&error));
            None
        }
    };
    if let Some(result) = poll_flow(&mut row.flow) {
        methods_now = linked(row, ui, result).or(methods_now);
    }
    if let Some(dialog) = &mut row.dialog
        && let Some(result) = poll_flow(&mut dialog.flow)
    {
        match result {
            Ok(Done::Linked(methods)) => {
                row.dialog = None;
                methods_now = linked(row, ui, Ok(Done::Linked(methods)));
            }
            Ok(_) => {}
            Err(error) => oidc::fail(&mut dialog.errors, &error, &["current_password"]),
        }
    }
    if let Some(true) = controls::answer(&ctx, &disconnect_key(&provider.id)) {
        let http = app.client.http().clone();
        let id = provider.id.clone();
        row.disconnect = Some(Pending::spawn(app.client, async move {
            http.disconnect_provider(&id).await
        }));
    }

    let hint = match &provider.connected {
        Some(connection) => texts::security_sign_in_provider_connected(&connection.email),
        None => texts::security_sign_in_provider_not_connected().to_owned(),
    };
    let flow_label = row.flow.as_ref().map(Flow::label);
    let own_status = row.status.current(ui);
    let status = if row.acted {
        own_status
    } else {
        feedback.or(own_status)
    };
    let disconnecting = row.disconnect.is_some();
    let button = if provider.connected.is_some() {
        Some(RowButton {
            label: texts::security_sign_in_provider_disconnect(),
            pending: disconnecting.then_some(texts::security_sign_in_provider_disconnecting()),
            disabled: !has_password,
        })
    } else if row.flow.is_some() {
        // The browser has the provider's page: the row can give up.
        Some(RowButton::new(common::cancel()))
    } else if provider.configured {
        Some(RowButton::new(texts::security_sign_in_provider_connect()))
    } else {
        None
    };
    let status = match &flow_label {
        Some(label) => Some((label.clone(), StatusTone::Neutral)),
        None => status,
    };
    let clicked = action_row(
        ui,
        &provider.name,
        &hint,
        status.as_ref().map(|(m, t)| (m.as_str(), *t)),
        button,
    );
    if clicked {
        row.acted = true;
        row.status.clear();
        if provider.connected.is_some() {
            controls::ask(
                &ctx,
                Confirm::danger(
                    &disconnect_key(&provider.id),
                    texts::security_sign_in_disconnect_confirm_title(&provider.name),
                    Some(&texts::security_sign_in_disconnect_confirm_message(
                        &provider.name,
                    )),
                    texts::security_sign_in_disconnect_confirm_label(),
                ),
            );
        } else if row.flow.is_some() {
            row.flow = None;
        } else if has_password {
            row.dialog = Some(ConnectDialog::default());
        } else {
            // Without a password the provider is left at once: there is
            // nothing to confirm with.
            row.flow = Some(Flow::start(
                &ctx,
                app.client,
                &own,
                OidcStart::new(OidcPurpose::Link),
            ));
        }
    }
    if row.dialog.is_some() && (provider.connected.is_some() || !has_password) {
        row.dialog = None;
    }
    if let Some(dialog) = &mut row.dialog
        && !connect_dialog(ui, app, dialog, &own)
    {
        row.dialog = None;
    }
    if let Some(methods) = methods_now {
        s.methods = Some(methods);
    }
}

/// "Verbinden" with a password: the API wants it before a provider
/// account is attached, then the flow leaves for the provider. False once
/// the dialog closes.
fn connect_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    dialog: &mut ConnectDialog,
    provider: &AuthProvider,
) -> bool {
    let ctx = ui.ctx().clone();
    let title = texts::security_sign_in_connect_title(&provider.name);
    let lead = texts::security_sign_in_connect_lead(&provider.name);
    let submit_label = texts::security_sign_in_connect_submit(&provider.name);
    let pending = dialog.flow.as_ref().map(Flow::label);
    let output = Modal::new(("connect-provider", &provider.id), &title)
        .dirty(!dialog.current.is_empty())
        .show(
            &ctx,
            |ui| {
                let theme = Theme::of(ui.ctx());
                controls::paragraph(
                    ui,
                    &lead,
                    controls::HINT,
                    theme.c.muted,
                    ui.available_width(),
                );
                ui.add_space(20.0);
                let current = TextField::new("link_current_password", &mut dialog.current)
                    .label(texts::security_current_password())
                    .password()
                    .autofocus(true)
                    .error(dialog.errors.field("current_password"))
                    .show(ui);
                if current.response.changed() {
                    dialog.errors.clear_field("current_password");
                }
                ui.add_space(20.0);
                dialog_error(ui, &dialog.errors);
                let (cancel, clicked) =
                    dialog_actions(ui, Button::new(&submit_label).pending(pending.as_deref()));
                (cancel, clicked || current.submitted)
            },
            None,
        );
    let (cancel, submit) = output.inner;
    if cancel || output.close_requested {
        // Dropping the dialog drops its flow too.
        return false;
    }
    if submit && dialog.flow.is_none() {
        if dialog.current.is_empty() {
            dialog.errors.set_fields(vec![(
                "current_password",
                field_message("current_password", "required"),
            )]);
        } else {
            dialog.errors = FormErrors::default();
            let start = OidcStart {
                current_password: Some(dialog.current.clone()),
                ..OidcStart::new(OidcPurpose::Link)
            };
            dialog.flow = Some(Flow::start(&ctx, app.client, provider, start));
        }
    }
    dialog.errors.focus = None;
    // An answer arrives without input; the window looks again.
    if dialog.flow.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_client::ProviderConnection;

    fn methods(password: bool, connected: bool) -> SignInMethods {
        SignInMethods {
            password,
            providers: vec![SignInProvider {
                id: "google".into(),
                name: "Google".into(),
                configured: true,
                connected: connected.then(|| ProviderConnection {
                    email: "lena@example.test".into(),
                    since: "2026-11-10T10:00:00Z".parse().expect("time"),
                }),
            }],
        }
    }

    #[test]
    fn feedback_reads_only_known_values() {
        assert_eq!(
            Feedback::read(Some("google"), Some("cancelled")),
            Feedback {
                connected: Some("google".into()),
                oidc_error: Some("cancelled"),
            }
        );
        assert_eq!(
            Feedback::read(Some("Google!"), Some("registration_closed")),
            Feedback::default()
        );
    }

    #[test]
    fn the_provider_and_the_error_rows() {
        assert_eq!(sign_in_provider(&methods(true, false)), None);
        assert_eq!(
            sign_in_provider(&methods(false, true)).map(|p| p.name),
            Some("Google".into())
        );
        assert_eq!(oidc_error_rows(&methods(true, false)), vec!["google"]);
        assert_eq!(oidc_error_rows(&methods(true, true)), vec!["google"]);
    }
}
