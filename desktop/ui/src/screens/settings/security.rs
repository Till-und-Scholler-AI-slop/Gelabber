//! Einstellungen → Sicherheit (pages/SecurityArea.tsx): "Angemeldet als",
//! the section "Anmeldung" (sign_in.rs, #518), "Angemeldete Geräte" with
//! one row per signed-in device and "Überall abmelden", and "Kontodaten"
//! with the data export and deleting the account (#519). Every row is a
//! settings action row (controls.rs `action_row`).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};
use egui::{Sense, Ui, Vec2, vec2};
use gelabber_client::{
    ApiError, AuthProvider, DeleteConfirmation, DeviceClient, DeviceOs, DeviceSession,
    DownloadError, EXPORT_POLL, ExportState, ExportStatus, OidcPurpose, OidcStart, OpState,
    OwnedServer, Role, User,
};
use gelabber_i18n::{account as texts, common, intl, settings as settings_texts};

use super::controls::{self, Confirm, RowButton, Status, action_row, action_row_badged};
use super::sign_in::{self, SECURITY_PATH, SignInState, dialog_error, reauth_note};
use super::{op_pending, settled};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, error_message, field_message};
use crate::screens::auth::oidc::{self, Done, Flow, poll_flow};
use crate::text;
use crate::theme::Theme;
use crate::widgets::{Button, ButtonVariant, Modal, TextField};

/// The area's state, per signed-in account and visit.
#[derive(Default)]
pub struct SecurityState {
    /// "Anmeldung": the password and the providers (#518).
    pub sign_in: SignInState,
    started: bool,
    /// The error line of a device row whose sign-out failed.
    device_status: HashMap<uuid::Uuid, Status>,
    /// Devices being signed out from here.
    revoking: HashSet<uuid::Uuid>,
    export: ExportRow,
    delete: Option<DeleteDialog>,
}

#[derive(Default)]
struct ExportRow {
    status: Status,
    requesting: bool,
    /// When the export was last asked for (egui time), for the polling.
    read_at: f64,
}

/// Settings area "Sicherheit".
pub fn area(ui: &mut Ui, app: &mut AppCtx, user: &User, s: &mut SecurityState) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    if let Some(true) = controls::answer(ui.ctx(), "logout-all") {
        app.client.voice_leave();
        app.client.logout_all();
        app.session.adopt(&serde_json::json!({ "user": null }));
        app.nav.navigate("/login");
        return;
    }
    if !s.started {
        s.started = true;
        app.client.load_devices();
        app.client.load_export();
        s.export.read_at = ui.input(|i| i.time);
    }
    // "Angemeldet als <b>{email}</b>": split first, then the address
    // goes in, so it cannot add tags.
    let mut block = text::Rich::new(controls::HINT);
    for piece in gelabber_i18n::pieces(settings_texts::security_signed_in_as()) {
        let shown = piece.text.replace("{email}", &user.email);
        block = match piece.tag {
            Some(_) => block.push(&shown, controls::HINT.weight(600.0), c.text),
            None => block.push(&shown, controls::HINT, c.muted),
        };
    }
    let block = block.layout(ui);
    crate::text::show(ui, &block);
    sign_in::section(ui, app, &mut s.sign_in);
    devices(ui, app, s);
    account_data(ui, app, s);
}

/// `.settings-action-section`: 24px above, the title, 4px, the list's
/// top hairline.
fn section_heading(ui: &mut Ui, title: &str, id: &str) {
    let theme = Theme::of(ui.ctx());
    ui.add_space(24.0);
    let top = ui.cursor().min;
    controls::paint(ui, top, title, text::TITLE_SM, theme.c.text);
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), text::TITLE_SM.line_px()),
        Sense::hover(),
    );
    let heading = ui.interact(rect, ui.scope_id().with(id), Sense::hover());
    crate::widgets::announce(&heading, egui::Role::Heading, title);
    ui.add_space(4.0);
    let width = ui.available_width();
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        ui.cursor().min.x..=ui.cursor().min.x + width,
        ui.cursor().min.y,
    );
    ui.add_space(1.0);
}

/// `.settings-action-list__state`: a load error or skeleton rows in a
/// row's place, padding 8 and the hairline under it.
fn state_cell(ui: &mut Ui, message: &str, failed: bool, pending: bool, rows: usize) -> bool {
    let theme = Theme::of(ui.ctx());
    ui.add_space(8.0);
    let retry = if failed {
        crate::widgets::load_error(ui, message, pending) && !pending
    } else {
        crate::widgets::skeleton::rows(ui, rows);
        false
    };
    ui.add_space(8.0);
    let width = ui.available_width();
    let y = ui.cursor().min.y;
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        ui.cursor().min.x..=ui.cursor().min.x + width,
        y,
    );
    ui.add_space(1.0);
    retry
}

// -------------------------------------------------------------- devices

/// Product and system names are the same in every language.
fn os_name(os: DeviceOs) -> Option<&'static str> {
    match os {
        DeviceOs::Linux => Some("Linux"),
        DeviceOs::Windows => Some("Windows"),
        DeviceOs::Macos => Some("macOS"),
        DeviceOs::Android => Some("Android"),
        DeviceOs::Ios => Some("iOS"),
        DeviceOs::Other => None,
    }
}

/// `deviceLabel`: "Gelabber für Linux", "Firefox unter Windows", "Browser
/// unter Android"; without a known system only the app or browser, and
/// "Unbekanntes Gerät" for a client the server could not tell.
pub fn device_label(client: DeviceClient, os: DeviceOs) -> String {
    let browser = match client {
        DeviceClient::Unknown => return texts::devices_unknown().to_owned(),
        DeviceClient::Desktop => {
            return match os_name(os) {
                Some(os) => texts::devices_desktop(os),
                None => texts::devices_desktop_only().to_owned(),
            };
        }
        DeviceClient::Firefox => "Firefox",
        DeviceClient::Chrome => "Chrome",
        DeviceClient::Edge => "Edge",
        DeviceClient::Safari => "Safari",
        DeviceClient::Browser => texts::devices_browser(),
    };
    match os_name(os) {
        Some(os) => texts::devices_browser_on(browser, os),
        None => browser.to_owned(),
    }
}

/// `lastActiveLabel`: the API's time is right to an hour at most, so only
/// the local calendar day is named; this device is "gerade eben".
pub fn last_active(session: &DeviceSession, now: DateTime<Utc>) -> String {
    let when = if session.current {
        texts::devices_last_active_now().to_owned()
    } else {
        let day = |at: DateTime<Utc>| at.with_timezone(&Local).date_naive();
        match (day(now) - day(session.last_used_at)).num_days() {
            ..=0 => texts::devices_last_active_today().to_owned(),
            1 => texts::devices_last_active_yesterday().to_owned(),
            n => texts::devices_last_active_days_ago(n as u64),
        }
    };
    texts::devices_last_active(&when)
}

fn revoke_key(id: uuid::Uuid) -> String {
    format!("revoke_device:{id}")
}

/// `DevicesSection`: one row per session in the API's order, then
/// "Überall abmelden", which does not wait for the list.
fn devices(ui: &mut Ui, app: &mut AppCtx, s: &mut SecurityState) {
    section_heading(ui, texts::devices_title(), "devices-heading");
    let (list, op) = app
        .client
        .with_state(|st| (st.devices.clone(), st.op("devices").cloned()));
    // Answers of the sign-outs from here.
    for id in s.revoking.clone() {
        if let Some(result) = settled(app, &revoke_key(id)) {
            s.revoking.remove(&id);
            if let Err(e) = result {
                s.device_status
                    .entry(id)
                    .or_default()
                    .error(ui, error_message(&Failure::from_api(&e).code));
            }
        }
    }
    match list {
        Some(list) => {
            let now = Utc::now();
            for session in &list {
                device_row(ui, app, s, session, now);
            }
        }
        None => {
            let failed = matches!(op, Some(OpState::Failed(_)));
            let pending = matches!(op, Some(OpState::Pending));
            if state_cell(ui, texts::devices_load_failed(), failed, pending, 3) {
                app.client.load_devices();
            }
        }
    }
    // "Überall abmelden" (issue #188), as the last row of the devices.
    if action_row(
        ui,
        settings_texts::security_everywhere_title(),
        settings_texts::security_everywhere_hint(),
        None,
        Some(
            RowButton::new(settings_texts::security_everywhere_button())
                .variant(ButtonVariant::DangerOutline),
        ),
    ) {
        controls::ask(
            ui.ctx(),
            Confirm::danger(
                "logout-all",
                settings_texts::security_everywhere_confirm_title(),
                Some(settings_texts::security_everywhere_confirm_message()),
                settings_texts::security_everywhere_confirm_label(),
            ),
        );
    }
}

fn device_row(
    ui: &mut Ui,
    app: &mut AppCtx,
    s: &mut SecurityState,
    session: &DeviceSession,
    now: DateTime<Utc>,
) {
    let status = s
        .device_status
        .get_mut(&session.id)
        .and_then(|st| st.current(ui));
    let pending = !session.current && op_pending(app, &revoke_key(session.id));
    let clicked = ui
        .push_id(("device", session.id), |ui| {
            action_row_badged(
                ui,
                &device_label(session.client, session.os),
                session.current.then_some(texts::devices_current()),
                &last_active(session, now),
                status.as_ref().map(|(m, t)| (m.as_str(), *t)),
                Some(
                    RowButton::new(texts::devices_sign_out())
                        .pending(pending.then_some(texts::devices_signing_out())),
                ),
            )
        })
        .inner;
    if !clicked || pending {
        return;
    }
    s.device_status.remove(&session.id);
    if session.current {
        // As the user panel does (after asking when that ends a call).
        crate::screens::shell::user_panel::log_out(ui.ctx(), app);
    } else {
        s.revoking.insert(session.id);
        app.client.revoke_device(session.id);
    }
}

// --------------------------------------------------------- account data

/// `AccountDataSection`: the data export and deleting the account.
fn account_data(ui: &mut Ui, app: &mut AppCtx, s: &mut SecurityState) {
    section_heading(ui, texts::data_title(), "account-data-heading");
    export_row(ui, app, &mut s.export);
    delete_row(ui, app, s);
}

/// What the export row's button does in each state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportAction {
    Request,
    Building,
    Download,
    Retry,
}

/// `exportRow`: the row's hint and button for an export state.
pub fn export_row_for(status: &ExportStatus) -> (String, ExportAction) {
    let mail = texts::export_hint_mail();
    match status.export.as_ref() {
        None => {
            let second = if status.mail {
                mail
            } else {
                texts::export_hint_no_mail()
            };
            (
                format!("{} {second}", texts::export_hint_none()),
                ExportAction::Request,
            )
        }
        Some(view) => match view.state {
            ExportState::Pending | ExportState::Running => {
                let building = texts::export_hint_building();
                let hint = if status.mail {
                    format!("{building} {mail}")
                } else {
                    building.to_owned()
                };
                (hint, ExportAction::Building)
            }
            ExportState::Ready => {
                let date = view
                    .expires_at
                    .map(crate::rules::format_day)
                    .unwrap_or_default();
                let size = crate::rules::format_size(view.size_bytes.unwrap_or(0));
                (
                    texts::export_hint_ready(&date, &size),
                    ExportAction::Download,
                )
            }
            ExportState::Failed => (texts::export_hint_failed().to_owned(), ExportAction::Retry),
        },
    }
}

/// `exportRequestError`: a younger ready export (429) says when the next
/// one can be asked for; anything else is the standard error line.
pub fn export_request_error(error: &ApiError) -> String {
    let failure = Failure::from_api(error);
    if failure.code == "rate_limited"
        && let Some(seconds) = failure.retry_after
    {
        let when = if seconds < 60.0 {
            intl::format_relative(seconds.ceil() as i64, intl::Unit::Second)
        } else if seconds < 3600.0 {
            intl::format_relative((seconds / 60.0).ceil() as i64, intl::Unit::Minute)
        } else {
            intl::format_relative((seconds / 3600.0).ceil() as i64, intl::Unit::Hour)
        };
        return texts::export_too_soon(&when);
    }
    error_message(&failure.code).to_owned()
}

/// The outcome of "Herunterladen" as a toast, wherever the person is by
/// then: saved, the server's or the connection's failure, or the file on
/// this device that could not be written (as `files::save` words it).
fn export_saved(ctx: &egui::Context, path: &str, result: Result<(), DownloadError>) {
    match result {
        Ok(()) => {
            crate::platform::files::notice(ctx, gelabber_i18n::app::file_saved_to(path), false)
        }
        Err(DownloadError::File(e)) => {
            tracing::warn!("export download: {e}");
            crate::platform::files::notice(ctx, gelabber_i18n::app::file_not_saved(&e), true);
        }
        Err(DownloadError::Api(e)) => {
            tracing::warn!("export download: {e}");
            if let Some(text) = crate::toasts::error_text(&e) {
                crate::platform::files::notice(
                    ctx,
                    gelabber_i18n::chat::attachment_download_failed(&text),
                    true,
                );
            }
        }
    }
}

/// "Daten exportieren": its hint and button follow the export's state.
fn export_row(ui: &mut Ui, app: &mut AppCtx, row: &mut ExportRow) {
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);
    let (export, op) = app
        .client
        .with_state(|st| (st.export.clone(), st.op("export").cloned()));
    if row.requesting
        && let Some(result) = settled(app, "request_export")
    {
        row.requesting = false;
        if let Err(e) = result {
            // Another device asked already, or the archive is younger than
            // a day: the row is out of date.
            if matches!(e.code(), "export_pending" | "rate_limited") {
                app.client.load_export();
                row.read_at = now;
            }
            row.status.error(ui, export_request_error(&e));
        } else {
            row.read_at = now;
        }
    }
    let Some(export) = export else {
        let failed = matches!(op, Some(OpState::Failed(_)));
        let pending = matches!(op, Some(OpState::Pending));
        if state_cell(ui, texts::export_load_failed(), failed, pending, 1) {
            app.client.load_export();
            row.read_at = now;
        }
        return;
    };
    // While it is being built it is read again every 10 s, while shown.
    if export.building() {
        let due = row.read_at + EXPORT_POLL.as_secs_f64();
        if now >= due {
            if !matches!(op, Some(OpState::Pending)) {
                app.client.load_export();
            }
            row.read_at = now;
            ctx.request_repaint_after(EXPORT_POLL);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
        }
    }
    let (hint, action) = export_row_for(&export);
    let button = match action {
        // One download at a time: its outcome comes as a toast.
        ExportAction::Download => RowButton::new(texts::export_download())
            .variant(ButtonVariant::Primary)
            .disabled(op_pending(app, "export_download")),
        ExportAction::Building => RowButton::new(texts::export_building()).disabled(true),
        ExportAction::Request | ExportAction::Retry => {
            let label = if action == ExportAction::Retry {
                texts::export_retry()
            } else {
                texts::export_request()
            };
            RowButton::new(label).pending(row.requesting.then_some(texts::export_building()))
        }
    };
    let status = row.status.current(ui);
    let clicked = action_row(
        ui,
        texts::export_title(),
        &hint,
        status.as_ref().map(|(m, t)| (m.as_str(), *t)),
        Some(button),
    );
    if !clicked {
        return;
    }
    match action {
        ExportAction::Download => {
            let name = export
                .export
                .as_ref()
                .map(|v| v.file_name())
                .unwrap_or_default();
            let client = app.client.clone();
            let toast = ctx.clone();
            crate::platform::files::save_to(
                &ctx,
                gelabber_i18n::chat::attachment_save_title(),
                &name,
                move |path| {
                    let shown = path.display().to_string();
                    client.download_export(path, move |result| {
                        export_saved(&toast, &shown, result);
                    });
                },
            );
        }
        ExportAction::Request | ExportAction::Retry if !row.requesting => {
            row.status.clear();
            row.requesting = true;
            app.client.request_export();
        }
        _ => {}
    }
}

// --------------------------------------------------------------- delete

#[derive(Default)]
struct DeleteDialog {
    value: String,
    errors: FormErrors,
    /// The servers of a `409 owns_servers`, which outranks the list.
    refused: Option<Vec<OwnedServer>>,
    /// The API wants a fresh sign-in first: "Mit … bestätigen".
    reauth: bool,
    confirm: Option<Flow>,
    /// Servers whose detail was asked for, for their member count.
    asked: HashSet<uuid::Uuid>,
}

fn delete_row(ui: &mut Ui, app: &mut AppCtx, s: &mut SecurityState) {
    if action_row(
        ui,
        texts::delete_title(),
        texts::delete_hint(),
        None,
        Some(RowButton::new(texts::delete_title()).variant(ButtonVariant::DangerOutline)),
    ) {
        s.delete = Some(DeleteDialog::default());
    }
    let Some(dialog) = &mut s.delete else {
        return;
    };
    // Until the methods are known, ask for the password as before sign-in.
    let methods = s.sign_in.methods.as_ref();
    let has_password = methods.is_none_or(|m| m.password);
    let provider = methods.and_then(sign_in::sign_in_provider);
    let listed: Vec<OwnedServer> = app.client.with_state(|st| {
        st.servers
            .iter()
            .filter(|v| v.role == Role::Owner)
            .map(|v| OwnedServer {
                id: v.id,
                name: v.name.clone(),
                member_count: None,
            })
            .collect()
    });
    let owned = dialog
        .refused
        .clone()
        .or_else(|| (!listed.is_empty()).then_some(listed));
    let open = match owned {
        Some(servers) => owned_dialog(ui, app, dialog, &servers),
        None => confirm_dialog(ui, app, dialog, has_password, provider.as_ref()),
    };
    if !open {
        s.delete = None;
    }
}

/// While the account owns servers: them, each with a way to its settings,
/// and "Schließen". False once it closes.
fn owned_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    dialog: &mut DeleteDialog,
    servers: &[OwnedServer],
) -> bool {
    let ctx = ui.ctx().clone();
    let mut go = None;
    let mut close = false;
    let output = Modal::new("delete-account-owned", texts::delete_title()).show(
        &ctx,
        |ui| {
            let theme = Theme::of(ui.ctx());
            let width = ui.available_width();
            controls::paragraph(
                ui,
                texts::delete_owned_lead(),
                controls::HINT,
                theme.c.muted,
                width,
            );
            ui.add_space(20.0);
            crate::widgets::rule_h(
                ui.painter(),
                theme.c.border,
                ui.cursor().min.x..=ui.cursor().min.x + width,
                ui.cursor().min.y,
            );
            ui.add_space(1.0);
            for server in servers {
                // The list has no member count; the server's own detail does.
                let count = server.member_count.map(|n| n.max(0) as u64).or_else(|| {
                    app.client
                        .with_state(|st| st.detail(server.id).map(|d| d.members.len() as u64))
                });
                if count.is_none() && dialog.asked.insert(server.id) {
                    app.client.refresh_server(server.id);
                }
                let hint = count.map(texts::delete_owned_members).unwrap_or_default();
                let clicked = ui
                    .push_id(("owned", server.id), |ui| {
                        action_row(
                            ui,
                            &server.name,
                            &hint,
                            None,
                            Some(RowButton::new(texts::delete_owned_settings())),
                        )
                    })
                    .inner;
                if clicked {
                    go = Some(server.id);
                }
            }
            ui.add_space(20.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 36.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    close = ui.add(Button::new(common::close())).clicked();
                },
            );
        },
        None,
    );
    if let Some(id) = go {
        // The settings page goes to the section itself.
        app.nav
            .navigate(&format!("/s/{id}/settings#server-settings-gefahrenzone"));
        return false;
    }
    !(close || output.close_requested)
}

/// "Konto endgültig löschen?": the password, or the address of an account
/// without one; that path may first want a fresh sign-in. False once it
/// closes.
fn confirm_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    dialog: &mut DeleteDialog,
    has_password: bool,
    provider: Option<&AuthProvider>,
) -> bool {
    let ctx = ui.ctx().clone();
    let field = if has_password { "password" } else { "email" };
    let pending = op_pending(app, "delete_account");
    // Answers.
    if let Some(Err(e)) = settled(app, "delete_account") {
        let failure = Failure::from_api(&e);
        if failure.code == "owns_servers" {
            dialog.errors = FormErrors::default();
            let servers = app.client.with_state(|st| st.owned_servers.clone());
            dialog.refused = Some(servers.unwrap_or_default());
            return true;
        } else if failure.code == "reauth_required" {
            dialog.errors = FormErrors::default();
            dialog.reauth = true;
        } else if failure
            .fields
            .get("email")
            .is_some_and(|code| code != "required")
        {
            // Any refusal of the address but "required": not this one.
            dialog
                .errors
                .set_fields(vec![("email", texts::delete_email_wrong().to_owned())]);
        } else {
            dialog.errors.from_failure(&failure, &[field]);
        }
    }
    if let Some(result) = poll_flow(&mut dialog.confirm) {
        match result {
            // Signed in afresh: the deletion goes through now.
            Ok(Done::SignedIn(session)) => {
                app.session.adopt(&session);
                dialog.reauth = false;
                dialog.errors = FormErrors::default();
            }
            Ok(_) => {}
            Err(error) => oidc::fail(&mut dialog.errors, &error, &[field]),
        }
    }
    let confirm_label = dialog.confirm.as_ref().map(Flow::label);
    let mut confirm = false;
    let output = Modal::new("delete-account", texts::delete_confirm_title())
        .dirty(!dialog.value.is_empty())
        .show(
            &ctx,
            |ui| {
                let theme = Theme::of(ui.ctx());
                let width = ui.available_width();
                controls::paragraph(
                    ui,
                    texts::delete_confirm_lead(),
                    controls::HINT,
                    theme.c.muted,
                    width,
                );
                ui.add_space(20.0);
                controls::paragraph(
                    ui,
                    texts::delete_confirm_export_hint(),
                    controls::HINT,
                    theme.c.muted,
                    width,
                );
                ui.add_space(20.0);
                let input = if has_password {
                    TextField::new("delete_password", &mut dialog.value)
                        .label(texts::delete_password())
                        .password()
                } else {
                    TextField::new("delete_email", &mut dialog.value)
                        .label(texts::delete_email())
                        .hint(Some(texts::delete_email_hint()))
                }
                .autofocus(true)
                .error(dialog.errors.field(field))
                .show(ui);
                if input.response.changed() {
                    dialog.errors.clear_field(field);
                }
                ui.add_space(20.0);
                if dialog.reauth {
                    confirm = reauth_note(ui, provider, confirm_label.as_deref());
                    ui.add_space(20.0);
                }
                dialog_error(ui, &dialog.errors);
                let mut out = (false, false);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 36.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        out.1 = ui
                            .add(
                                Button::new(texts::delete_title())
                                    .danger()
                                    .pending(pending.then_some(texts::delete_deleting())),
                            )
                            .clicked();
                        out.0 = ui.add(Button::new(common::cancel()).ghost()).clicked();
                    },
                );
                (out.0, out.1 || input.submitted)
            },
            None,
        );
    let (cancel, submit) = output.inner;
    if cancel || output.close_requested {
        return false;
    }
    if confirm
        && dialog.confirm.is_none()
        && let Some(provider) = provider
    {
        dialog.errors = FormErrors::default();
        let start = OidcStart {
            redirect: Some(SECURITY_PATH.to_owned()),
            ..OidcStart::new(OidcPurpose::SignIn)
        };
        dialog.confirm = Some(Flow::start(&ctx, app.client, provider, start));
    }
    // `submitDeletion`: an empty field sends nothing, a deletion on its
    // way is not sent twice; a password as typed, an address trimmed.
    if submit && !pending {
        let email = dialog.value.trim();
        let empty = if has_password {
            dialog.value.is_empty()
        } else {
            email.is_empty()
        };
        if empty {
            dialog
                .errors
                .set_fields(vec![(field, field_message(field, "required"))]);
        } else {
            dialog.errors = FormErrors::default();
            let confirmation = if has_password {
                DeleteConfirmation::Password(dialog.value.clone())
            } else {
                DeleteConfirmation::Email(email.to_owned())
            };
            app.client.delete_account(confirmation);
        }
    }
    dialog.errors.focus = None;
    // An answer arrives without input; the window looks again.
    if pending || dialog.confirm.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_client::ExportView;
    use gelabber_i18n::{Locale, on_this_thread};

    fn session(client: DeviceClient, os: DeviceOs, current: bool, days: i64) -> DeviceSession {
        let now = Utc::now();
        DeviceSession {
            id: uuid::Uuid::from_u128(1),
            client,
            os,
            created_at: now,
            last_used_at: now - chrono::Duration::days(days),
            current,
        }
    }

    #[test]
    fn device_labels_in_both_languages() {
        let _de = on_this_thread(Locale::De);
        let label = device_label;
        assert_eq!(
            label(DeviceClient::Desktop, DeviceOs::Linux),
            "Gelabber für Linux"
        );
        assert_eq!(
            label(DeviceClient::Firefox, DeviceOs::Windows),
            "Firefox unter Windows"
        );
        assert_eq!(
            label(DeviceClient::Browser, DeviceOs::Android),
            "Browser unter Android"
        );
        assert_eq!(label(DeviceClient::Safari, DeviceOs::Other), "Safari");
        assert_eq!(label(DeviceClient::Desktop, DeviceOs::Other), "Gelabber");
        assert_eq!(
            label(DeviceClient::Unknown, DeviceOs::Linux),
            "Unbekanntes Gerät"
        );
        drop(_de);
        let _en = on_this_thread(Locale::En);
        assert_eq!(label(DeviceClient::Edge, DeviceOs::Macos), "Edge on macOS");
        assert_eq!(
            label(DeviceClient::Desktop, DeviceOs::Windows),
            "Gelabber for Windows"
        );
        assert_eq!(
            label(DeviceClient::Unknown, DeviceOs::Ios),
            "Unknown device"
        );
    }

    #[test]
    fn last_active_names_the_day() {
        let _de = on_this_thread(Locale::De);
        // Noon keeps a day's distance a calendar day in any zone.
        let now = Local::now()
            .date_naive()
            .and_hms_opt(12, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).single())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let at = |current, days| {
            let mut s = session(DeviceClient::Chrome, DeviceOs::Linux, current, 0);
            s.last_used_at = now - chrono::Duration::days(days);
            last_active(&s, now)
        };
        assert_eq!(at(true, 3), "Zuletzt aktiv: gerade eben");
        assert_eq!(at(false, 0), "Zuletzt aktiv: heute");
        assert_eq!(at(false, 1), "Zuletzt aktiv: gestern");
        assert_eq!(at(false, 12), "Zuletzt aktiv: vor 12 Tagen");
        drop(_de);
        let _en = on_this_thread(Locale::En);
        assert_eq!(at(false, 1), "Last active: yesterday");
        assert_eq!(at(false, 2), "Last active: 2 days ago");
    }

    fn status(state: Option<ExportState>, mail: bool) -> ExportStatus {
        let at = "2036-11-11T12:00:00Z".parse().expect("time");
        ExportStatus {
            export: state.map(|state| ExportView {
                state,
                requested_at: at,
                finished_at: Some(at),
                expires_at: Some("2036-11-18T12:00:00Z".parse().expect("time")),
                size_bytes: Some(48_234_496),
            }),
            mail,
        }
    }

    #[test]
    fn the_export_row_follows_the_state() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(
            export_row_for(&status(None, false)),
            (
                "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du kannst es hier herunterladen, sobald es bereitsteht."
                    .to_owned(),
                ExportAction::Request
            )
        );
        assert_eq!(
            export_row_for(&status(None, true)).0,
            "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du bekommst eine E-Mail, sobald es bereitsteht."
        );
        assert_eq!(
            export_row_for(&status(Some(ExportState::Running), true)),
            (
                "Wird erstellt. Das kann eine Weile dauern. Du bekommst eine E-Mail, sobald es bereitsteht."
                    .to_owned(),
                ExportAction::Building
            )
        );
        assert_eq!(
            export_row_for(&status(Some(ExportState::Pending), false)).0,
            "Wird erstellt. Das kann eine Weile dauern."
        );
        assert_eq!(
            export_row_for(&status(Some(ExportState::Ready), true)),
            (
                "Bereit bis 18. November 2036, 46,0 MB.".to_owned(),
                ExportAction::Download
            )
        );
        assert_eq!(
            export_row_for(&status(Some(ExportState::Failed), true)),
            (
                "Der Export ist fehlgeschlagen.".to_owned(),
                ExportAction::Retry
            )
        );
        drop(_de);
        let _en = on_this_thread(Locale::En);
        assert_eq!(
            export_row_for(&status(Some(ExportState::Ready), true)).0,
            "Ready until 18 November 2036, 46.0 MB."
        );
    }

    #[test]
    fn a_refused_request_says_when_to_ask_again() {
        let _de = on_this_thread(Locale::De);
        let refusal = |code: &str, retry_after| ApiError::Api {
            status: 429,
            code: code.into(),
            message: String::new(),
            fields: serde_json::Value::Null,
            retry_after,
        };
        assert_eq!(
            export_request_error(&refusal("rate_limited", Some(5.0 * 3600.0 - 10.0))),
            "Einen neuen Export kannst du in 5 Stunden anfordern."
        );
        assert_eq!(
            export_request_error(&refusal("rate_limited", Some(90.0))),
            "Einen neuen Export kannst du in 2 Minuten anfordern."
        );
        assert_eq!(
            export_request_error(&refusal("export_pending", None)),
            error_message("export_pending")
        );
    }
}

/// Sicherheit drawn whole against the stand-in API (app/testkit.rs).
#[cfg(test)]
mod drawn {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use egui::accesskit::Role as A11y;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use serde_json::{Value, json};

    use crate::Launcher;
    use crate::app::testkit::{
        FakeApi, HOME, Reply, launcher, wait_for, wait_for_label, workspace,
    };

    const OTHER: &str = "00000000-0000-4000-8000-0000000000d2";

    fn ago(days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
    }

    /// Lena's devices as the parity scene has them.
    fn sessions(revoked: bool) -> Value {
        let mut list = vec![json!({
            "id": "00000000-0000-4000-8000-0000000000d1", "client": "desktop", "os": "linux",
            "created_at": ago(0), "last_used_at": ago(0), "current": true
        })];
        if !revoked {
            list.push(json!({
                "id": OTHER, "client": "firefox", "os": "windows",
                "created_at": ago(3), "last_used_at": ago(1), "current": false
            }));
        }
        list.push(json!({
            "id": "00000000-0000-4000-8000-0000000000d3", "client": "desktop", "os": "windows",
            "created_at": ago(20), "last_used_at": ago(12), "current": false
        }));
        json!({ "sessions": list })
    }

    fn methods() -> Value {
        json!({"password": true, "providers": []})
    }

    fn no_export() -> Value {
        json!({"export": null, "mail": false})
    }

    fn export(state: &str) -> Value {
        let ready = state == "ready";
        json!({"export": {
            "state": state,
            "requested_at": "2036-11-11T10:00:00Z",
            "finished_at": ready.then_some("2036-11-11T10:05:00Z"),
            "expires_at": ready.then_some("2036-11-18T10:05:00Z"),
            "size_bytes": ready.then_some(48_234_496)
        }, "mail": false})
    }

    /// The page's own reads; `None` for the rest.
    fn page(method: &str, path: &str) -> Option<Reply> {
        match (method, path) {
            ("GET", "/auth/sessions") => Some(Reply::json(sessions(false))),
            ("GET", "/auth/methods") => Some(Reply::json(methods())),
            ("GET", "/me/export") => Some(Reply::json(no_export())),
            _ => None,
        }
    }

    fn button<'h>(h: &'h Harness<'_, Launcher>, label: &'h str) -> Vec<egui_kittest::Node<'h>> {
        h.query_all_by_label(label)
            .filter(|n| n.accesskit_node().role() == A11y::Button)
            .collect()
    }

    fn href(h: &mut Harness<'_, Launcher>) -> String {
        h.state_mut().app().expect("app").nav().location().href()
    }

    fn input<'h>(h: &'h Harness<'_, Launcher>, label: &'h str) -> Option<egui_kittest::Node<'h>> {
        h.query_all_by_label(label).find(|n| {
            matches!(
                n.accesskit_node().role(),
                A11y::TextInput | A11y::PasswordInput
            )
        })
    }

    fn type_into(h: &mut Harness<'_, Launcher>, label: &str, text: &str) {
        wait_for(h, label, |h| input(h, label).is_some());
        input(h, label).expect("input").focus();
        h.step();
        input(h, label).expect("input").type_text(text);
        h.step();
    }

    /// The rows of hosted-security.png: the devices with "Dieses Gerät" on
    /// this one, Überall abmelden as their last row, then Kontodaten.
    /// "Abmelden" on another device ends it and reads the list again.
    #[test]
    fn the_devices_and_another_one_signed_out() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-devices");
        let revoked = Arc::new(AtomicBool::new(false));
        let gone = revoked.clone();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/auth/sessions") => Some(Reply::json(sessions(gone.load(Ordering::SeqCst)))),
            ("DELETE", p) if p == format!("/auth/sessions/{OTHER}") => {
                gone.store(true, Ordering::SeqCst);
                Some(Reply::json(json!({})))
            }
            _ => page(method, path),
        });
        let mut h = launcher(&api, "/settings/konto");
        wait_for_label(&mut h, "Gelabber für Linux");
        for label in [
            "Angemeldete Geräte",
            "Dieses Gerät",
            "Zuletzt aktiv: gerade eben",
            "Firefox unter Windows",
            "Zuletzt aktiv: gestern",
            "Gelabber für Windows",
            "Zuletzt aktiv: vor 12 Tagen",
            "Überall abmelden",
            "Kontodaten",
            "Daten exportieren",
            "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du kannst es hier herunterladen, sobald es bereitsteht.",
            "Endgültig. Server, die dir gehören, übergibst oder löschst du vorher.",
        ] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        assert_eq!(button(&h, "Abmelden").len(), 3);
        assert_eq!(button(&h, "Export anfordern").len(), 1);
        assert_eq!(button(&h, "Konto löschen").len(), 1);
        assert_eq!(button(&h, "Auf allen Geräten abmelden").len(), 1);
        button(&h, "Abmelden")[1].click();
        wait_for(&mut h, "the device gone", |h| {
            h.query_by_label("Firefox unter Windows").is_none()
        });
        assert!(api.saw(&format!("DELETE /auth/sessions/{OTHER}")));
        assert!(h.query_by_label("Gelabber für Windows").is_some());
        assert_eq!(href(&mut h), "/settings/konto");
    }

    /// "Abmelden" on this device signs out as the user panel does.
    #[test]
    fn this_device_signs_out_here() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-this-device");
        let api = FakeApi::start(true, |method, path, _| page(method, path));
        let mut h = launcher(&api, "/settings/konto");
        wait_for_label(&mut h, "Gelabber für Linux");
        button(&h, "Abmelden")[0].click();
        wait_for(&mut h, "the sign-in page", |h| {
            h.query_by_label("Anmelden").is_some() && h.query_by_label("Dieses Gerät").is_none()
        });
        assert!(api.saw("POST /auth/logout"));
        assert!(!api.saw("DELETE /auth/sessions/00000000-0000-4000-8000-0000000000d1"));
    }

    /// "Export anfordern" turns the row into "Wird erstellt…"; a request
    /// within a day of the last says when the next one can be.
    #[test]
    fn requesting_the_export() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-export");
        let asked = Arc::new(AtomicBool::new(false));
        let flag = asked.clone();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/me/export") if flag.load(Ordering::SeqCst) => {
                Some(Reply::json(export("pending")))
            }
            ("POST", "/me/export") => {
                flag.store(true, Ordering::SeqCst);
                Some(Reply::status(202, export("pending")))
            }
            _ => page(method, path),
        });
        let mut h = launcher(&api, "/settings/konto");
        wait_for_label(&mut h, "Export anfordern");
        button(&h, "Export anfordern")[0].click();
        wait_for_label(&mut h, "Wird erstellt. Das kann eine Weile dauern.");
        let building = button(&h, "Wird erstellt…");
        assert_eq!(building.len(), 1);
        assert!(building[0].accesskit_node().is_disabled());
        assert!(api.saw("POST /me/export"));

        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("POST", "/me/export") => {
                Some(Reply::error(429, "rate_limited").header("retry-after", "17990"))
            }
            _ => page(method, path),
        });
        let mut h = launcher(&api, "/settings/konto");
        wait_for_label(&mut h, "Export anfordern");
        button(&h, "Export anfordern")[0].click();
        crate::app::testkit::wait_for_text(
            &mut h,
            "Einen neuen Export kannst du in 5 Stunden anfordern.",
        );
    }

    /// A ready export: "Herunterladen" streams it into the file the save
    /// dialog chose (here the Downloads folder), named for its day.
    #[test]
    fn downloading_the_ready_export() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-download");
        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("GET", "/me/export") => Some(Reply::json(export("ready"))),
            ("GET", "/me/export/download") => Some(Reply::json(json!({"zip": true}))),
            _ => page(method, path),
        });
        let dir = std::env::temp_dir().join(format!("gelabber-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut h = launcher(&api, "/settings/konto");
        crate::platform::files::set_choosers(
            &h.ctx,
            crate::platform::files::Choosers {
                bus: None,
                portal: false,
                native: false,
                zenity: String::new(),
                kdialog: String::new(),
                downloads: Some(dir.clone()),
            },
        );
        wait_for_label(&mut h, "Bereit bis 18. November 2036, 46,0 MB.");
        button(&h, "Herunterladen")[0].click();
        let saved = dir.join("gelabber-export-2036-11-11.zip");
        crate::app::testkit::wait_for_text(&mut h, "Gespeichert unter");
        assert_eq!(std::fs::read(&saved).expect("saved"), br#"{"zip":true}"#);
        assert!(api.saw("GET /me/export/download"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file this device cannot write is "not saved", with the reason,
    /// not a lost connection; and the toast comes although the person
    /// went on to another area meanwhile.
    #[test]
    fn an_export_that_cannot_be_written_is_not_saved() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-download-io");
        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("GET", "/me/export") => Some(Reply::json(export("ready"))),
            ("GET", "/me/export/download") => Some(Reply::json(json!({"zip": true}))),
            _ => page(method, path),
        });
        let dir = std::env::temp_dir()
            .join(format!("gelabber-export-io-{}", std::process::id()))
            .join("not-there");
        let mut h = launcher(&api, "/settings/konto");
        crate::platform::files::set_choosers(
            &h.ctx,
            crate::platform::files::Choosers {
                bus: None,
                portal: false,
                native: false,
                zenity: String::new(),
                kdialog: String::new(),
                downloads: Some(dir.clone()),
            },
        );
        wait_for_label(&mut h, "Bereit bis 18. November 2036, 46,0 MB.");
        button(&h, "Herunterladen")[0].click();
        wait_for(&mut h, "the download", |_| {
            api.saw("GET /me/export/download")
        });
        h.state_mut()
            .app()
            .expect("app")
            .navigate("/settings/profil");
        crate::app::testkit::wait_for_text(&mut h, "Datei nicht gespeichert.");
        assert!(h.query_by_label_contains("nicht heruntergeladen").is_none());
        assert!(!dir.exists());
    }

    /// Owning a server (the list says so): the dialog names it with its
    /// members and leads to its Gefahrenzone.
    #[test]
    fn deleting_while_owning_a_server_leads_to_it() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-delete-owned");
        let api = FakeApi::start(true, |method, path, _| {
            page(method, path).or_else(|| workspace(method, path))
        });
        let mut h = launcher(&api, "/settings/konto");
        wait_for(&mut h, "the page", |h| {
            !button(h, "Konto löschen").is_empty()
        });
        // The list of servers is in before the dialog decides.
        wait_for(&mut h, "the servers", |_| api.saw("GET /servers"));
        h.run_steps(3);
        button(&h, "Konto löschen")[0].click();
        wait_for_label(&mut h, "1 Mitglied");
        // Its row, beside the rail's tile.
        assert!(h.query_all_by_label("Wohnzimmer").count() >= 2);
        assert!(h.query_by_label("Konto endgültig löschen?").is_none());
        button(&h, "Servereinstellungen")[0].click();
        wait_for(&mut h, "server settings", |h| {
            h.query_by_label("Servereinstellungen").is_none()
        });
        let location = h.state_mut().app().expect("app").nav().location().clone();
        assert_eq!(location.path, format!("/s/{HOME}/settings"));
        assert_eq!(location.fragment, "server-settings-gefahrenzone");
    }

    /// No server of one's own: the password, then `202` leaves for
    /// /login?reason=deleted, which says so. A `409 owns_servers` on the
    /// way shows the servers instead.
    #[test]
    fn deleting_the_account_with_the_password() {
        let _config = crate::platform::paths::testing::TempConfig::new("security-delete");
        let deleted = Arc::new(AtomicBool::new(false));
        let flag = deleted.clone();
        let api = FakeApi::start(true, move |method, path, body| match (method, path) {
            ("DELETE", "/me") if body.contains(r#""password":"geheim""#) => {
                flag.store(true, Ordering::SeqCst);
                Some(Reply::status(202, json!({"csrf_token": "n"})))
            }
            ("DELETE", "/me") => Some(Reply::status(
                409,
                json!({"error": "owns_servers", "message": "x", "servers": [
                    {"id": HOME, "name": "Wohnzimmer", "member_count": 5}
                ]}),
            )),
            ("GET", "/auth/session") if flag.load(Ordering::SeqCst) => {
                Some(Reply::json(json!({"user": null, "csrf_token": "n"})))
            }
            _ => page(method, path),
        });
        let mut h = launcher(&api, "/settings/konto");
        wait_for(&mut h, "the page", |h| {
            !button(h, "Konto löschen").is_empty()
        });
        wait_for(&mut h, "the servers", |_| api.saw("GET /servers"));
        h.run_steps(3);
        button(&h, "Konto löschen")[0].click();
        wait_for_label(&mut h, "Konto endgültig löschen?");
        // Empty: nothing is sent.
        button(&h, "Konto löschen").last().expect("submit").click();
        h.run_steps(4);
        assert!(!api.saw("DELETE /me"));
        // The API knows of a server the list did not.
        type_into(&mut h, "Passwort", "anders");
        button(&h, "Konto löschen").last().expect("submit").click();
        wait_for_label(&mut h, "5 Mitglieder");
        button(&h, "Schließen").last().expect("close").click();
        h.run_steps(4);
        // Once more, now with nothing in the way.
        button(&h, "Konto löschen")[0].click();
        wait_for_label(&mut h, "Konto endgültig löschen?");
        type_into(&mut h, "Passwort", "geheim");
        button(&h, "Konto löschen").last().expect("submit").click();
        wait_for_label(&mut h, "Dein Konto wurde gelöscht.");
        assert_eq!(href(&mut h), "/login?reason=deleted");
        assert!(deleted.load(Ordering::SeqCst));
    }

    /// `/login?reason=deleted` on its own.
    #[test]
    fn the_sign_in_page_says_the_account_is_gone() {
        let _config = crate::platform::paths::testing::TempConfig::new("login-deleted");
        let api = FakeApi::start(false, |_, _, _| None);
        let mut h = launcher(&api, "/login?reason=deleted");
        wait_for_label(&mut h, "Dein Konto wurde gelöscht.");
        assert!(
            h.query_by_label("Sitzung abgelaufen. Melde dich erneut an.")
                .is_none()
        );
    }
}
