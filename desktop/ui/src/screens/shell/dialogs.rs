//! The shell's dialogs (components/ServerDialogs.tsx, InviteDialog.tsx,
//! NewMessageDialog.tsx, ConfirmHost.tsx): add, create and join a server,
//! the invite link, channel and category forms, delete confirmations and
//! "Neue Nachricht". One is open at a time; [`show`] draws it.

use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{ApiError, Category, Channel, ChannelKind, ChannelPatch, Invite, OpState};
use gelabber_i18n::{Text, dms, servers, shell};
use std::sync::atomic::Ordering;

use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::overlays::picker::{PickerState, Row, RowIcon};
use super::state::{self, AddStep, Created, Dialog, InviteState, Terms};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::rules;
use crate::task::Pending;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, FormError, InlineStatus, Modal, StatusTone, TextField};

const EXPIRY: [(Text, Option<u32>); 4] = [
    (servers::terms_expiry_hour, Some(1)),
    (servers::terms_expiry_day, Some(24)),
    (servers::terms_expiry_week, Some(168)),
    (servers::terms_expiry_never, None),
];
const USES: [(Text, Option<u32>); 4] = [
    (|| "1", Some(1)),
    (|| "5", Some(5)),
    (|| "25", Some(25)),
    (servers::terms_uses_unlimited, None),
];

pub fn open_add(ctx: &egui::Context, step: AddStep) {
    super::overlays::opened(ctx);
    state::with(ctx, |s| {
        s.dialog = Some(Dialog::Add {
            step,
            name: String::new(),
            code: String::new(),
            error: None,
            sent: false,
        });
    });
}

/// "Zu {server} einladen": creates a link as it opens.
pub fn open_invite(ctx: &egui::Context, server: Uuid, name: &str) {
    super::overlays::opened(ctx);
    state::with(ctx, |s| {
        s.dialog = Some(Dialog::Invite(Box::new(InviteState {
            server,
            name: name.to_owned(),
            terms: Terms::default(),
            invite: None,
            creating: None,
            latest: Default::default(),
            error: None,
            handed_out: false,
            status: None,
            terms_open: false,
        })));
    });
}

pub fn open_channel(
    ctx: &egui::Context,
    server: Uuid,
    editing: Option<&Channel>,
    category: Option<Uuid>,
) {
    super::overlays::opened(ctx);
    state::with(ctx, |s| {
        s.dialog = Some(Dialog::Channel {
            server,
            editing: editing.map(|c| c.id),
            voice: editing.is_some_and(|c| c.kind == ChannelKind::Voice),
            name: editing.map(|c| c.name.clone()).unwrap_or_default(),
            category: editing.map_or(category, |c| c.category_id),
            error: None,
            access: editing.and_then(super::access::of),
            initial_name: editing.map(|c| c.name.clone()).unwrap_or_default(),
            initial_access: editing.and_then(super::access::of),
        });
    });
}

pub fn open_category(ctx: &egui::Context, server: Uuid, editing: Option<&Category>) {
    super::overlays::opened(ctx);
    state::with(ctx, |s| {
        s.dialog = Some(Dialog::Category {
            server,
            editing: editing.map(|c| c.id),
            name: editing.map(|c| c.name.clone()).unwrap_or_default(),
            error: None,
        });
    });
}

/// Asked through the confirm host, which starts on "Abbrechen".
pub fn confirm_delete_channel(ctx: &egui::Context, server: Uuid, channel: &Channel) {
    super::overlays::ask(
        ctx,
        super::overlays::Question::danger(
            shell::channels_delete_channel_title(&channel.name),
            Some(shell::channels_delete_channel_message()),
            shell::channels_delete_channel_confirm(),
            super::overlays::ConfirmAction::DeleteChannel {
                server,
                channel: channel.id,
            },
        ),
    );
}

pub fn confirm_delete_category(ctx: &egui::Context, server: Uuid, category: &Category) {
    super::overlays::ask(
        ctx,
        super::overlays::Question::danger(
            shell::channels_delete_category_title(&category.name),
            Some(shell::channels_delete_category_message()),
            shell::channels_delete_category_confirm(),
            super::overlays::ConfirmAction::DeleteCategory {
                server,
                category: category.id,
            },
        ),
    );
}

pub fn open_new_message(ctx: &egui::Context) {
    super::overlays::opened(ctx);
    state::with(ctx, |s| {
        s.dialog = Some(Dialog::NewMessage {
            picker: PickerState::default(),
            waiting: None,
            loaded: false,
        });
    });
}

/// Draws the open dialog, if any.
pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    super::overlays::follow_route(ui.ctx(), app.nav.route());
    let Some(mut dialog) = state::with(ui.ctx(), |s| s.dialog.take()) else {
        return;
    };
    let keep = match &mut dialog {
        Dialog::Add {
            step,
            name,
            code,
            error,
            sent,
        } => add_server(ui, app, step, name, code, error, sent),
        Dialog::Invite(invite) => invite_dialog(ui, app, invite),
        Dialog::Channel {
            server,
            editing,
            voice,
            name,
            category,
            error,
            access,
            initial_name,
            initial_access,
        } => channel_dialog(
            ui,
            app,
            *server,
            *editing,
            voice,
            name,
            category,
            error,
            ChannelAccess {
                value: access,
                initial_name,
                initial: initial_access,
            },
        ),
        Dialog::Category {
            server,
            editing,
            name,
            error,
        } => category_dialog(ui, app, *server, *editing, name, error),
        Dialog::NewMessage {
            picker,
            waiting,
            loaded,
        } => new_message(ui, app, picker, waiting, loaded),
    };
    if keep {
        state::with(ui.ctx(), |s| {
            // A dialog opened from inside this one wins.
            if s.dialog.is_none() {
                s.dialog = Some(dialog);
            }
        });
    }
}

/// A right-aligned row of buttons at the end of a dialog body.
fn buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui);
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn add_server(
    ui: &mut Ui,
    app: &mut AppCtx,
    step: &mut AddStep,
    name: &mut String,
    code: &mut String,
    error: &mut Option<String>,
    sent: &mut bool,
) -> bool {
    let ctx = ui.ctx().clone();
    let title = match step {
        AddStep::Choose => servers::add_title(),
        AddStep::Create => servers::create_title(),
        AddStep::Join => servers::join_title(),
    };
    let op = app.client.with_state(|s| s.op("create_server").cloned());
    let pending = *sent && matches!(op, Some(OpState::Pending));
    if *sent {
        match &op {
            Some(OpState::Done) => {
                // The newest server is the one just made.
                let made = app.client.with_state(|s| {
                    s.servers.last().map(|v| {
                        let first = s
                            .detail(v.id)
                            .and_then(|d| d.channels.iter().find(|c| c.kind == ChannelKind::Text))
                            .map(|c| c.id);
                        (v.id, v.name.clone(), first, s.me().map(|m| m.id))
                    })
                });
                app.client.clear_op("create_server");
                if let Some((id, server_name, first, user)) = made {
                    if let Some(first) = first {
                        state::remember(&ctx, user, |m| {
                            m.channels.insert(id, first);
                        });
                    }
                    state::with(&ctx, |s| s.invite_prompt = Some((id, server_name)));
                    app.nav.navigate(&format!("/s/{id}"));
                }
                return false;
            }
            Some(OpState::Failed(e)) => {
                *error = Some(rules::error_message(e.code()).to_owned());
                *sent = false;
            }
            _ => {}
        }
    }
    let mut close = false;
    let output = Modal::new("add-server", title)
        .dirty(!name.is_empty() || !code.is_empty())
        .show(
            &ctx,
            |ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                match step {
                    AddStep::Choose => {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let create = choice(
                            ui,
                            Icon::Plus,
                            servers::create_title(),
                            servers::create_hint(),
                        );
                        // The dialog opens on its first control, as showModal does.
                        if crate::widgets::field::mounted(ui.ctx(), Id::unique("gelabber-add-open"))
                        {
                            create.request_focus();
                        }
                        if create.clicked() {
                            *step = AddStep::Create;
                        }
                        if choice(ui, Icon::Link, servers::join_title(), servers::join_hint())
                            .clicked()
                        {
                            *step = AddStep::Join;
                        }
                    }
                    AddStep::Create => {
                        let field = TextField::new("server-name", name)
                            .label(servers::form_name_label())
                            .autofocus(true)
                            .char_limit(100)
                            .hint(Some(servers::create_hint()))
                            .error(error.as_deref())
                            .show(ui);
                        if field.response.changed() {
                            *error = None;
                        }
                        let mut submit = field.submitted;
                        buttons(ui, |ui| {
                            submit |= ui
                                .add(
                                    Button::new(servers::form_create())
                                        .primary()
                                        .pending(pending.then_some(servers::form_creating())),
                                )
                                .clicked();
                            if ui
                                .add(Button::new(gelabber_i18n::common::back()).ghost())
                                .clicked()
                            {
                                *step = AddStep::Choose;
                            }
                        });
                        if submit && !pending {
                            match rules::validate_name(name) {
                                Some(code) => *error = Some(rules::field_message("name", code)),
                                None => {
                                    app.client.create_server(name.trim());
                                    *sent = true;
                                }
                            }
                        }
                    }
                    AddStep::Join => {
                        let field = TextField::new("join-invite", code)
                            .label(servers::join_input_label())
                            .placeholder(servers::join_input_placeholder())
                            .autofocus(true)
                            .error(error.as_deref())
                            .show(ui);
                        if field.response.changed() {
                            *error = None;
                        }
                        let mut submit = field.submitted;
                        buttons(ui, |ui| {
                            submit |= ui
                                .add(Button::new(gelabber_i18n::common::next()).primary())
                                .clicked();
                            if ui
                                .add(Button::new(gelabber_i18n::common::back()).ghost())
                                .clicked()
                            {
                                *step = AddStep::Choose;
                            }
                        });
                        if submit {
                            let origin = app.client.http().invite_link("x");
                            match parse_invite(code, &origin) {
                                Ok(code) => {
                                    app.nav.navigate(&format!("/invite/{code}"));
                                    close = true;
                                }
                                Err(message) => *error = Some(message.to_owned()),
                            }
                        }
                    }
                }
            },
            None,
        );
    !(output.close_requested || close)
}

/// `.add-server-choice`: icon, a 600 14px title and a muted line.
fn choice(ui: &mut Ui, icon: Icon, title: &str, detail: &str) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let hovered = response.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(ui.painter(), rect, theme.radius.md, &theme);
    }
    ui.painter().rect(
        rect,
        corner(theme.radius.md),
        if hovered { c.hover } else { c.panel },
        Stroke::new(1.0, if hovered { c.accent } else { c.border_strong }),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 17.0, rect.top() + 13.0 + 2.0),
        20.0,
        c.accent_text,
    );
    let strong = Block::single(ui, title, text::TITLE_SM.size(14.0), c.text);
    let small = Block::single(ui, detail, text::META, c.muted);
    let x = rect.left() + 17.0 + 20.0 + 12.0;
    let h = strong.height() + 2.0 + small.height();
    let top = rect.top() + (64.0 - h) / 2.0;
    strong.paint(ui.painter(), pos2(x, top));
    small.paint(ui.painter(), pos2(x, top + strong.height() + 2.0));
    response
}

/// `parseInvite`: a full link, a link without scheme, `/invite/<code>` or
/// the bare 12-character code.
pub fn parse_invite(input: &str, own_link: &str) -> Result<String, &'static str> {
    let value = input.trim();
    if value.is_empty() {
        return Err(servers::join_error_empty());
    }
    let invalid = servers::join_error_invalid();
    let bare = |v: &str| {
        let code = v.to_lowercase();
        let ok = code.chars().count() == 12
            && code
                .chars()
                .all(|c| "abcdefghjkmnpqrstuvwxyz23456789".contains(c));
        if ok { Ok(code) } else { Err(invalid) }
    };
    let from_path = |path: &str| {
        let rest = path.strip_prefix("/invite/").ok_or(invalid)?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.is_empty() || rest.contains(['/', '?', '#']) {
            return Err(invalid);
        }
        bare(rest)
    };
    if value.starts_with('/') {
        return from_path(value);
    }
    let own = url::Url::parse(own_link).ok();
    let lower = value.to_lowercase();
    let url = if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(url::Url::parse(value).map_err(|_| invalid)?)
    } else if value.contains("/invite/")
        && !value.contains(char::is_whitespace)
        && value.split('/').next().is_some_and(|h| h.contains('.'))
    {
        let scheme = own.as_ref().map_or("https", |u| u.scheme());
        Some(url::Url::parse(&format!("{scheme}://{value}")).map_err(|_| invalid)?)
    } else {
        None
    };
    match url {
        Some(url) => {
            let host = |u: &url::Url| {
                format!(
                    "{}{}",
                    u.host_str().unwrap_or(""),
                    u.port().map(|p| format!(":{p}")).unwrap_or_default()
                )
            };
            if own.as_ref().is_some_and(|o| host(o) != host(&url)) {
                return Err(servers::join_error_foreign());
            }
            from_path(url.path())
        }
        None => bare(value),
    }
}

/// `termsSummary`: "gültig 7 Tage · unbegrenzt nutzbar".
fn summary(terms: Terms) -> String {
    let expiry = match EXPIRY[terms.expiry].1 {
        None => servers::terms_valid_forever().to_owned(),
        Some(_) => servers::terms_valid_for(EXPIRY[terms.expiry].0()),
    };
    let uses = match USES[terms.uses].1 {
        None => servers::terms_uses_unlimited_note().to_owned(),
        Some(n) => servers::terms_uses_limited(u64::from(n)),
    };
    format!("{expiry} · {uses}")
}

/// A link with `terms`; one that newer terms superseded while it was made
/// is revoked at once, as nobody saw it (also after the dialog closed).
fn create_invite(app: &AppCtx, s: &InviteState, terms: Terms) -> Pending<Created> {
    let http = app.client.http().clone();
    let (hours, uses) = (EXPIRY[terms.expiry].1, USES[terms.uses].1);
    let (server, latest) = (s.server, s.latest.clone());
    let request = latest.fetch_add(1, Ordering::SeqCst) + 1;
    Pending::spawn(app.client, async move {
        let result = http.create_invite(server, uses, hours).await;
        if let Ok(invite) = &result
            && latest.load(Ordering::SeqCst) != request
        {
            let _ = http.revoke_invite(&invite.code).await;
        }
        (terms, result)
    })
}

/// Takes a finished link request: the new link with its terms, or on a
/// failure the chips go back to the link still shown. Returns the code of
/// a replaced link to revoke.
fn settle(s: &mut InviteState, terms: Terms, result: Result<Invite, ApiError>) -> Option<String> {
    match result {
        Ok(invite) => {
            let mut revoke = None;
            if let Some((old, _)) = s.invite.take() {
                if !s.handed_out {
                    revoke = Some(old.code);
                }
                s.status = Some((
                    servers::invite_new_link(&summary(terms)),
                    StatusTone::Neutral,
                ));
            }
            s.invite = Some((invite, terms));
            s.terms = terms;
            s.handed_out = false;
            s.error = None;
            revoke
        }
        Err(e) => {
            if let Some((_, shown)) = &s.invite {
                s.terms = *shown;
            }
            s.error = Some(rules::error_message(e.code()).to_owned());
            None
        }
    }
}

/// The terms of the link shown (the chosen ones before there is one).
fn shown_terms(s: &InviteState) -> Terms {
    s.invite.as_ref().map_or(s.terms, |(_, terms)| *terms)
}

fn invite_dialog(ui: &mut Ui, app: &mut AppCtx, s: &mut InviteState) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    if s.invite.is_none() && s.creating.is_none() && s.error.is_none() {
        s.creating = Some(create_invite(app, s, s.terms));
    }
    if let Some((terms, result)) = crate::task::poll(&mut s.creating)
        && let Some(code) = settle(s, terms, result)
    {
        app.client.revoke_invite(&code);
    }
    let creating = s.creating.is_some();
    let link = s
        .invite
        .as_ref()
        .map(|(i, _)| app.client.http().invite_link(&i.code))
        .unwrap_or_default();
    let title = servers::invite_title(&s.name);
    let output = Modal::new(("invite", s.server), &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            if s.invite.is_none()
                && let Some(error) = &s.error
            {
                let message = servers::invite_create_failed(error);
                ui.add(FormError::new(&message));
                if ui
                    .add(Button::new(gelabber_i18n::common::try_again()).small())
                    .clicked()
                {
                    s.error = None;
                }
                return;
            }
            // The link and "Link kopieren".
            let row_w = ui.available_width();
            let copy_label = servers::invite_copy();
            let copy_w = Block::single(ui, copy_label, text::UI.weight(600.0), c.text).width()
                + 16.0
                + 8.0
                + 34.0;
            let (row, _) = ui.allocate_exact_size(Vec2::new(row_w, 36.0), egui::Sense::hover());
            let field = Rect::from_min_size(row.min, Vec2::new(row_w - copy_w - 8.0, 36.0));
            ui.painter().rect(
                field,
                corner(theme.radius.sm),
                // `.ui-field[readonly]`.
                c.surface,
                Stroke::new(1.0, c.border_strong),
                StrokeKind::Inside,
            );
            let shown = if creating {
                servers::invite_making()
            } else {
                link.as_str()
            };
            // `ui-monospace` 13px, cut with an ellipsis.
            let mono = TextStyle::new(13.0, 400.0, 1.5);
            let mut format = mono.format(c.text);
            format.font_id = egui::FontId::new(13.0, crate::fonts::mono_family());
            let mut job = egui::text::LayoutJob::single_section(shown.to_owned(), format);
            job.wrap = egui::text::TextWrapping {
                max_width: field.width() - 22.0,
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let block = Block::new(ui, job, mono);
            paint_mid(ui.painter(), &block, field.left() + 11.0, field.center().y);
            let copy_rect = Rect::from_min_size(
                pos2(field.right() + 8.0, row.top()),
                Vec2::new(copy_w, 36.0),
            );
            let mut copy = ui.new_child(egui::UiBuilder::new().max_rect(copy_rect));
            let copy_button = copy.add(Button::new(copy_label).primary().icon(Icon::Copy));
            // `initialFocus` on "Link kopieren".
            if crate::widgets::field::mounted(ui.ctx(), Id::unique("gelabber-invite-open")) {
                copy_button.request_focus();
            }
            if copy_button.clicked() {
                if creating {
                    s.status = Some((servers::invite_making().into(), StatusTone::Neutral));
                } else if !link.is_empty() {
                    let (message, tone, copied) = crate::screens::chat::clipboard::copy_link(
                        ui.ctx(),
                        &link,
                        &summary(shown_terms(s)),
                    );
                    s.handed_out |= copied;
                    s.status = Some((message, tone));
                }
            }
            match &s.status {
                Some((message, tone)) => {
                    ui.add(InlineStatus::new(Some(message), *tone));
                }
                None => {
                    let (r, _) =
                        ui.allocate_exact_size(Vec2::new(row_w, 20.0), egui::Sense::hover());
                    let block = Block::single(ui, &summary(shown_terms(s)), text::META, c.muted);
                    paint_mid(ui.painter(), &block, r.left(), r.center().y);
                }
            }
            if s.error.is_some() && s.invite.is_some() {
                text::label(ui, servers::invite_replace_failed(), text::META, c.danger);
            }
            // "Ablauf & Limit ändern".
            ui.add_space(4.0);
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(row_w, 1.0 + 12.0 + 32.0), egui::Sense::hover());
            crate::widgets::hairline_h(ui.painter(), &theme, r.left()..=r.right(), r.top());
            let label = Block::single(ui, servers::invite_terms_editor(), text::UI, c.muted);
            let summary_rect = Rect::from_min_size(
                pos2(r.left(), r.top() + 13.0),
                Vec2::new(label.width() + 4.0 + 14.0, 32.0),
            );
            let response = common::click(ui, summary_rect, Id::unique("invite-terms"));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, servers::invite_terms_editor())
            });
            let color = if response.hovered() { c.text } else { c.muted };
            paint_mid(
                ui.painter(),
                &label,
                summary_rect.left(),
                summary_rect.center().y,
            );
            // The chevron points right while closed (`rotate: -90deg`).
            let chevron = pos2(summary_rect.right() - 14.0, summary_rect.center().y - 7.0);
            let turn = if s.terms_open { 0 } else { 3 };
            icons::paint_turned(ui.painter(), Icon::Chevron, chevron, 14.0, color, turn);
            if response.clicked() {
                s.terms_open = !s.terms_open;
            }
            if s.terms_open {
                // Column gap 8, margin-top 8; each caps label 8 more (the
                // panel's 12 after the summary already counts).
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                let mut changed = None;
                for (n, (caps, options, current, which)) in [
                    (
                        servers::invite_expires_after(),
                        EXPIRY.map(|o| o.0()),
                        s.terms.expiry,
                        0,
                    ),
                    (
                        servers::invite_uses_label(),
                        USES.map(|o| o.0()),
                        s.terms.uses,
                        1,
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    if n > 0 {
                        ui.add_space(8.0);
                    }
                    text::label(ui, &caps.to_uppercase(), text::CAPS, c.muted);
                    super::overlays::row(ui, 32.0, 8.0, |ui| {
                        for (i, option) in options.iter().enumerate() {
                            if chip(ui, option, i == current).clicked() && i != current {
                                changed = Some((which, i));
                            }
                        }
                    });
                }
                if let Some((which, i)) = changed {
                    if which == 0 {
                        s.terms.expiry = i;
                    } else {
                        s.terms.uses = i;
                    }
                    s.creating = Some(create_invite(app, s, s.terms));
                }
            }
        },
        None,
    );
    !output.close_requested
}

/// `.ui-chip`: 32 tall, pill, accent border and selected fill when chosen.
fn chip(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let style = if selected {
        text::UI.weight(600.0)
    } else {
        text::UI
    };
    let block = Block::single(ui, label, style, c.text);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(block.width() + 26.0, 32.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let fill = if selected {
        c.selected
    } else if response.hovered() {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        corner(theme.radius.full),
        fill,
        Stroke::new(1.0, if selected { c.accent } else { c.border_strong }),
        StrokeKind::Inside,
    );
    paint_mid(ui.painter(), &block, rect.left() + 13.0, rect.center().y);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// "Wird gespeichert als #slug" (lower case, spaces/underscores → `-`).
fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        let ch = match ch {
            ' ' | '_' => '-',
            '#' => continue,
            c => c.to_lowercase().next().unwrap_or(c),
        };
        if ch == '-' && out.ends_with('-') {
            continue;
        }
        out.push(ch);
    }
    out.trim_matches('-').to_owned()
}

/// The channel dialog's "Zugriff" and what it opened with.
struct ChannelAccess<'a> {
    value: &'a mut Option<Vec<Uuid>>,
    initial_name: &'a str,
    initial: &'a Option<Vec<Uuid>>,
}

#[allow(clippy::too_many_arguments)]
fn channel_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    server: Uuid,
    editing: Option<Uuid>,
    voice: &mut bool,
    name: &mut String,
    category: &mut Option<Uuid>,
    error: &mut Option<String>,
    access: ChannelAccess,
) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let title = if editing.is_some() {
        servers::channel_edit_title()
    } else {
        servers::channel_create_title()
    };
    let (categories, roles) = app.client.with_state(|s| {
        s.detail(server)
            .map(|d| (d.categories.clone(), d.roles.clone()))
            .unwrap_or_default()
    });
    let mut done = false;
    // An open category list takes Esc first, like the web's native select;
    // the access list's popover closes on its own Esc too.
    let select_id = Id::unique(("channel-category", server));
    let access_id = Id::unique(("channel-access", server));
    for list in [select_id, access_id] {
        if ctx.data(|d| d.get_temp::<bool>(list)).unwrap_or(false)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            ctx.data_mut(|d| d.insert_temp(list, false));
        }
    }
    let dirty =
        name.as_str() != access.initial_name || !super::access::same(access.value, access.initial);
    let modal = Modal::new(("channel-dialog", server), title).dirty(dirty);
    let output = modal.show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            if editing.is_none() {
                ui.vertical(|ui| {
                    // The legend sits 2px higher in its fieldset than a label.
                    ui.spacing_mut().item_spacing.y = 6.0;
                    text::label(ui, servers::channel_kind_legend(), text::UI, c.text);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let w = (ui.available_width() - 8.0) / 2.0;
                        for (is_voice, icon, label, detail) in [
                            (
                                false,
                                Icon::Hash,
                                servers::channel_kind_text(),
                                servers::channel_kind_text_hint(),
                            ),
                            (
                                true,
                                Icon::Speaker,
                                servers::channel_kind_voice(),
                                servers::channel_kind_voice_hint(),
                            ),
                        ] {
                            // A `RadioCard`: the radio, then the label and its hint.
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::new(w, 63.0), egui::Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::RadioButton,
                                    true,
                                    *voice == is_voice,
                                    label,
                                )
                            });
                            let on = *voice == is_voice;
                            ui.painter().rect(
                                rect,
                                corner(theme.radius.md),
                                if on {
                                    c.selected
                                } else if response.hovered() {
                                    c.hover
                                } else {
                                    c.panel
                                },
                                Stroke::new(1.0, if on { c.accent } else { c.border_strong }),
                                StrokeKind::Inside,
                            );
                            crate::screens::settings::controls::radio_mark(
                                ui,
                                pos2(rect.left() + 13.0, rect.top() + 12.0),
                                on,
                            );
                            let x = rect.left() + 46.0;
                            icons::paint(
                                ui.painter(),
                                icon,
                                pos2(x, rect.top() + 16.0),
                                14.0,
                                c.text,
                            );
                            let strong = Block::single(ui, label, text::UI.weight(600.0), c.text);
                            strong.paint(ui.painter(), pos2(x + 14.0 + 6.0, rect.top() + 13.0));
                            let small = Block::single(ui, detail, text::META, c.muted);
                            small.paint(
                                ui.painter(),
                                pos2(x, rect.top() + 13.0 + strong.height() + 3.0),
                            );
                            if response.clicked() {
                                *voice = is_voice;
                            }
                        }
                    });
                });
            }
            let hint = (!*voice)
                .then(|| slug(name))
                .filter(|s| !s.is_empty() && *s != name.trim())
                .map(|s| servers::channel_saved_as(&s));
            let field = TextField::new("channel-name", name)
                .label(servers::form_name_label())
                .autofocus(true)
                .char_limit(100)
                .hint(hint.as_deref())
                .error(error.as_deref())
                .show(ui);
            if field.response.changed() {
                *error = None;
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                text::label(ui, servers::channel_category_label(), text::UI, c.text);
                let mut options = vec![(None, servers::channel_no_category().to_owned())];
                options.extend(categories.iter().map(|x| (Some(x.id), x.name.clone())));
                select(ui, select_id, &options, category);
            });
            super::access::field(ui, access_id, &roles, access.value);
            let mut submit = field.submitted;
            buttons(ui, |ui| {
                submit |= ui
                    .add(
                        Button::new(if editing.is_some() {
                            gelabber_i18n::common::save()
                        } else {
                            servers::form_create()
                        })
                        .primary(),
                    )
                    .clicked();
                if ui
                    .add(Button::new(gelabber_i18n::common::cancel()).ghost())
                    .clicked()
                {
                    done = true;
                }
            });
            if submit {
                match rules::validate_name(name) {
                    Some(code) => *error = Some(rules::field_message("name", code)),
                    None => {
                        let name = name.trim();
                        let role_ids =
                            super::access::role_ids(access.initial, access.value, &roles);
                        match editing {
                            Some(channel) => app.client.update_channel(
                                server,
                                channel,
                                ChannelPatch {
                                    name: Some(name.to_owned()),
                                    category_id: Some(
                                        category.map(|c| c.to_string()).unwrap_or_default(),
                                    ),
                                    role_ids,
                                },
                            ),
                            None => app.client.create_channel(
                                server,
                                name,
                                if *voice {
                                    ChannelKind::Voice
                                } else {
                                    ChannelKind::Text
                                },
                                *category,
                                role_ids,
                            ),
                        }
                        done = true;
                    }
                }
            }
        },
        None,
    );
    !(output.close_requested || done)
}

fn category_dialog(
    ui: &mut Ui,
    app: &mut AppCtx,
    server: Uuid,
    editing: Option<Uuid>,
    name: &mut String,
    error: &mut Option<String>,
) -> bool {
    let ctx = ui.ctx().clone();
    let title = if editing.is_some() {
        servers::category_rename_title()
    } else {
        servers::category_create_title()
    };
    let mut done = false;
    let output = Modal::new(("category-dialog", server), title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let field = TextField::new("category-name", name)
                .label(servers::form_name_label())
                .autofocus(true)
                .char_limit(100)
                .error(error.as_deref())
                .show(ui);
            if field.response.changed() {
                *error = None;
            }
            let mut submit = field.submitted;
            buttons(ui, |ui| {
                submit |= ui
                    .add(
                        Button::new(if editing.is_some() {
                            gelabber_i18n::common::save()
                        } else {
                            servers::form_create()
                        })
                        .primary(),
                    )
                    .clicked();
                if ui
                    .add(Button::new(gelabber_i18n::common::cancel()).ghost())
                    .clicked()
                {
                    done = true;
                }
            });
            if submit {
                match rules::validate_name(name) {
                    Some(code) => *error = Some(rules::field_message("name", code)),
                    None => {
                        match editing {
                            Some(id) => app.client.rename_category(server, id, name.trim()),
                            None => app.client.create_category(server, name.trim()),
                        }
                        done = true;
                    }
                }
            }
        },
        None,
    );
    !(output.close_requested || done)
}

/// "Neue Nachricht" (`NewMessageDialog`): anyone from the servers you
/// share, picked with the keyboard or a click, then their DM.
fn new_message(
    ui: &mut Ui,
    app: &mut AppCtx,
    picker: &mut PickerState,
    waiting: &mut Option<Uuid>,
    loaded: &mut bool,
) -> bool {
    let ctx = ui.ctx().clone();
    if let Some(user) = *waiting {
        if let Some(dm) = app
            .client
            .with_state(|s| s.dms.iter().find(|d| d.peer.id == user).map(|d| d.id))
        {
            app.nav.navigate(&format!("/d/{dm}"));
            return false;
        }
        if matches!(
            app.client.with_state(|s| s.op("dm").cloned()),
            Some(OpState::Failed(_))
        ) {
            *waiting = None;
        }
    }
    // dmCandidates: everyone you share a server with (details loaded on
    // demand); people already written to first, in DM order, then by name.
    let (people, missing) = app.client.with_state(|s| {
        let me = s.me().map(|m| m.id);
        let mut people: Vec<(Uuid, String, Option<String>, String, bool)> = Vec::new();
        let missing: Vec<Uuid> = s
            .servers
            .iter()
            .filter(|v| s.detail(v.id).is_none())
            .map(|v| v.id)
            .collect();
        for server in &s.servers {
            let Some(detail) = s.detail(server.id) else {
                continue;
            };
            for m in &detail.members {
                if Some(m.user_id) == me {
                    continue;
                }
                match people.iter_mut().find(|p| p.0 == m.user_id) {
                    Some(p) => {
                        p.3.push_str(", ");
                        p.3.push_str(&server.name);
                    }
                    None => people.push((
                        m.user_id,
                        m.name.clone(),
                        m.avatar_url.clone(),
                        server.name.clone(),
                        false,
                    )),
                }
            }
        }
        let mut recent = Vec::new();
        for dm in &s.dms {
            if let Some(i) = people.iter().position(|p| p.0 == dm.peer.id) {
                let mut p = people.remove(i);
                p.3 = dms::new_message_continue().into();
                p.4 = true;
                recent.push(p);
            }
        }
        people.sort_by_cached_key(|p| gelabber_client::store::collation_key(&p.1));
        recent.extend(people);
        (recent, missing)
    });
    if !*loaded {
        for id in &missing {
            app.client.refresh_server(*id);
        }
        *loaded = true;
    }
    // filterCandidates: best match first, the list order among equals.
    let mut scored: Vec<(u8, usize)> = people
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            super::overlays::picker::match_score(&picker.query, &p.1).map(|s| (s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let shown: Vec<_> = scored.iter().map(|(_, i)| &people[*i]).collect();
    let rows: Vec<Row> = shown
        .iter()
        .map(|(_, name, avatar, detail, _)| Row {
            label: name.clone(),
            detail: Some(detail.clone()),
            icon: RowIcon::Avatar {
                name: name.clone(),
                url: avatar.clone(),
            },
            badge: gelabber_client::Badge::default(),
        })
        .collect();
    let empty = if !missing.is_empty() {
        gelabber_i18n::common::loading().to_owned()
    } else if people.is_empty() {
        dms::new_message_nobody().to_owned()
    } else {
        dms::new_message_not_found(picker.query.trim())
    };
    let mut picked = None;
    let busy = waiting.is_some();
    let output = Modal::new("new-message", dms::new_message()).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            picked = super::overlays::picker::show(
                ui,
                app.images,
                "new-message-query",
                picker,
                dms::new_message_person(),
                dms::new_message_placeholder(),
                &rows,
                &empty,
                busy,
            );
        },
        None,
    );
    if let Some(i) = picked {
        let user = shown[i].0;
        match app
            .client
            .with_state(|s| s.dms.iter().find(|d| d.peer.id == user).map(|d| d.id))
        {
            Some(dm) => {
                app.nav.navigate(&format!("/d/{dm}"));
                return false;
            }
            None => {
                app.client.start_dm(user);
                *waiting = Some(user);
            }
        }
    }
    !output.close_requested
}

/// `Select` (components/ui/Select.tsx): a `.ui-field` box with the chosen
/// name and a 14px chevron 11px from the right; a click lists the options.
fn select(ui: &mut Ui, id: Id, options: &[(Option<Uuid>, String)], value: &mut Option<Uuid>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut open: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 37.0), egui::Sense::click());
    let rect = crate::widgets::snap(ui.ctx(), rect);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        open = !open;
    }
    let painter = ui.painter();
    if open {
        crate::widgets::focus_ring(painter, rect, theme.radius.sm, &theme);
    }
    painter.rect(
        rect,
        corner(theme.radius.sm),
        c.bg,
        Stroke::new(1.0, if open { c.focus } else { c.border_strong }),
        StrokeKind::Inside,
    );
    let label = options
        .iter()
        .find(|(v, _)| v == value)
        .or(options.first())
        .map_or("", |(_, l)| l.as_str());
    let block = line(ui, label, text::BODY, c.text, rect.width() - 12.0 - 34.0);
    paint_mid(painter, &block, rect.left() + 12.0, rect.center().y);
    icons::paint(
        painter,
        Icon::Chevron,
        pos2(rect.right() - 11.0 - 14.0, rect.center().y - 7.0),
        14.0,
        c.muted,
    );
    if open {
        // The popover tops out at 360 like `.ui-popover`.
        let picked = crate::widgets::Popover::new(id, rect)
            .min_width(rect.width().min(360.0))
            .select_list()
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut picked = None;
                for (v, l) in options {
                    if crate::widgets::menu_item(ui, None, l, false).clicked() {
                        picked = Some(*v);
                    }
                }
                picked
            });
        match picked {
            // A press outside; the box itself toggles above.
            None => open = false,
            Some(Some(v)) => {
                *value = v;
                open = false;
            }
            Some(None) => {}
        }
    }
    ui.data_mut(|d| d.insert_temp(id, open));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invites_parse_like_the_web() {
        let own = "http://127.0.0.1:5180/invite/x";
        assert_eq!(parse_invite("abcdefghjkmn", own), Ok("abcdefghjkmn".into()));
        assert_eq!(parse_invite("ABCDEFGHJKMN", own), Ok("abcdefghjkmn".into()));
        assert_eq!(
            parse_invite("/invite/abcdefghjkmn", own),
            Ok("abcdefghjkmn".into())
        );
        assert_eq!(
            parse_invite("http://127.0.0.1:5180/invite/abcdefghjkmn", own),
            Ok("abcdefghjkmn".into())
        );
        assert_eq!(
            parse_invite("https://example.org/invite/abcdefghjkmn", own),
            Err("Dieser Link gehört zu einem anderen Gelabber-Server.")
        );
        assert_eq!(
            parse_invite("  ", own),
            Err("Bitte einen Einladungslink oder Code eingeben.")
        );
        assert_eq!(
            parse_invite("hallo", own),
            Err("Das sieht nicht wie ein Einladungslink aus.")
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Mein  Kanal_neu"), "mein-kanal-neu");
        assert_eq!(slug("#spiele"), "spiele");
    }

    fn invite(code: &str) -> Invite {
        serde_json::from_value(serde_json::json!({
            "code": code, "server_id": Uuid::nil(), "created_by": Uuid::nil(),
            "created_at": "2026-01-01T00:00:00Z", "expires_at": null,
            "max_uses": null, "uses": 0
        }))
        .expect("invite")
    }

    /// InviteDialog: a failed change puts the chips back to the link shown,
    /// whose terms the summary and "Link kopieren" name.
    #[test]
    fn a_failed_terms_change_keeps_the_terms_of_the_link_shown() {
        let mut s = InviteState {
            server: Uuid::nil(),
            name: "Wohnzimmer".into(),
            terms: Terms::default(),
            invite: Some((invite("aaaaaaaaaaaa"), Terms::default())),
            creating: None,
            latest: Default::default(),
            error: None,
            handed_out: false,
            status: None,
            terms_open: true,
        };
        let hour = Terms { expiry: 0, uses: 0 };
        s.terms = hour;
        let failed = ApiError::Transport("offline".into());
        assert_eq!(settle(&mut s, hour, Err(failed)), None);
        assert_eq!(s.terms, Terms::default());
        assert_eq!(shown_terms(&s), Terms::default());
        assert!(s.error.is_some());
        // The retry of the same chip works and replaces the unseen link.
        s.terms = hour;
        assert_eq!(
            settle(&mut s, hour, Ok(invite("bbbbbbbbbbbb"))),
            Some("aaaaaaaaaaaa".into())
        );
        assert_eq!(shown_terms(&s), hour);
        assert!(s.error.is_none());
        // A copied link stays valid.
        s.handed_out = true;
        assert_eq!(
            settle(&mut s, Terms::default(), Ok(invite("cccccccccccc"))),
            None
        );
    }

    /// A link made for terms that changed again meanwhile is revoked: the
    /// dialog never showed it (InviteDialog's `latest`).
    #[test]
    fn a_superseded_link_is_revoked() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        use egui_kittest::kittest::Queryable;

        use crate::app::testkit::{FakeApi, HOME, Reply, launcher, wait_for, workspace};
        use crate::platform::paths::testing::TempConfig;

        let _config = TempConfig::new("invite-superseded");
        let release = Arc::new(AtomicBool::new(false));
        let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (hold, count) = (release.clone(), served.clone());
        let api = FakeApi::start(true, move |method, path, body| {
            if method == "POST" && path == format!("/servers/{HOME}/invites") {
                let code = if body.contains("\"expires_in_hours\":1}") {
                    while !hold.load(Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    "einestunde22"
                } else if body.contains("\"expires_in_hours\":24}") {
                    "vierundzwanz"
                } else {
                    "siebentage22"
                };
                count.fetch_add(1, Ordering::SeqCst);
                return Some(Reply::json(serde_json::json!({
                    "code": code, "server_id": HOME, "created_by": HOME,
                    "created_at": "2026-01-01T00:00:00Z", "expires_at": null,
                    "max_uses": null, "uses": 0
                })));
            }
            if method == "DELETE" && path.starts_with("/invites/") {
                return Some(Reply::json(serde_json::Value::Null));
            }
            workspace(method, path)
        });
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        crate::app::testkit::wait_for_label(&mut harness, "allgemein");
        open_invite(
            &harness.ctx.clone(),
            HOME.parse().expect("id"),
            "Wohnzimmer",
        );
        let posts = || {
            let post = format!("POST /servers/{HOME}/invites");
            api.seen.lock().iter().filter(|r| **r == post).count()
        };
        let served_now = || served.load(Ordering::SeqCst);
        wait_for(&mut harness, "the first link", |_| served_now() == 1);
        harness.run_steps(3);
        harness.get_by_label("Ablauf & Limit ändern").click();
        crate::app::testkit::wait_for_label(&mut harness, "1 Stunde");
        harness.get_by_label("1 Stunde").click();
        wait_for(&mut harness, "the held request", |_| posts() == 2);
        harness.get_by_label("24 Stunden").click();
        wait_for(&mut harness, "the newest link", |_| served_now() == 2);
        harness.run_steps(5);
        release.store(true, Ordering::SeqCst);
        wait_for(&mut harness, "the unseen link's revoke", |_| {
            api.saw("DELETE /invites/einestunde22")
        });
        // The link replaced by the shown one goes too; the shown one stays.
        assert!(api.saw("DELETE /invites/siebentage22"));
        assert!(!api.saw("DELETE /invites/vierundzwanz"));
    }

    /// modalClose.ts: Esc or "Schließen" on a dialog with input asks
    /// "Änderungen verwerfen?" first; an empty one closes at once.
    #[test]
    fn a_dialog_with_input_asks_before_it_closes() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;

        use crate::app::testkit::{FakeApi, launcher, wait_for_label};
        use crate::platform::paths::testing::TempConfig;

        let _config = TempConfig::new("dialog-discard");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        wait_for_label(&mut harness, "Benutzermenü");
        let question = "Änderungen verwerfen?";
        // Empty: Esc closes it.
        open_add(&harness.ctx.clone(), AddStep::Join);
        wait_for_label(&mut harness, "Einladungslink oder Code");
        harness.key_press(egui::Key::Escape);
        harness.run_steps(4);
        assert!(harness.query_by_label("Einladungslink oder Code").is_none());
        assert!(harness.query_by_label(question).is_none());
        // With a code typed: Esc asks, and Esc again keeps the dialog.
        open_add(&harness.ctx.clone(), AddStep::Join);
        wait_for_label(&mut harness, "Einladungslink oder Code");
        fn field<'a>(h: &'a egui_kittest::Harness<'_, crate::Launcher>) -> egui_kittest::Node<'a> {
            use egui_kittest::kittest::NodeT;
            h.query_all_by_label("Einladungslink oder Code")
                .find(|n| n.accesskit_node().role() == Role::TextInput)
                .expect("the field")
        }
        field(&harness).focus();
        harness.step();
        field(&harness).type_text("abc");
        harness.run_steps(2);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(4);
        harness.get_by_label(question);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(4);
        assert!(harness.query_by_label(question).is_none());
        assert!(harness.query_by_label("Einladungslink oder Code").is_some());
        // "Schließen" asks too; "Verwerfen" closes it.
        harness
            .query_all_by_label("Schließen")
            .find(|n| {
                use egui_kittest::kittest::NodeT;
                n.accesskit_node().role() == Role::Button
            })
            .expect("the close button")
            .click();
        harness.run_steps(4);
        harness.get_by_label(question);
        harness
            .get_by_role_and_label(Role::Button, "Verwerfen")
            .click();
        harness.run_steps(4);
        assert!(harness.query_by_label(question).is_none());
        assert!(harness.query_by_label("Einladungslink oder Code").is_none());
        assert!(!crate::screens::shell::dialog_open(&harness.ctx));
    }

    #[test]
    fn invite_summaries() {
        assert_eq!(
            summary(Terms::default()),
            "gültig 7 Tage · unbegrenzt nutzbar"
        );
        assert_eq!(
            summary(Terms { expiry: 3, uses: 0 }),
            "unbegrenzt gültig · 1-mal nutzbar"
        );
    }

    /// "Zugriff" (ChannelAccessField, channelRequest): the field reads the
    /// roles, its checklist follows `toggleAccess`, Esc closes only the
    /// list, and the submit sends `role_ids` only when the access changed.
    #[test]
    fn the_channel_dialog_sends_the_access() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{NodeT, Queryable};
        use parking_lot::Mutex;
        use std::sync::Arc;

        use crate::app::testkit::{
            FakeApi, HOME, MODERATION, SPIELE, STAMMGAESTE, launcher, wait_for, wait_for_label,
            workspace,
        };
        use crate::platform::paths::testing::TempConfig;

        let _config = TempConfig::new("channel-access");
        let sent: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let log = sent.clone();
        let api = FakeApi::start(true, move |method, path, body| {
            if method == "PATCH" || method == "POST" {
                log.lock()
                    .push((format!("{method} {path}"), body.to_owned()));
            }
            workspace(method, path)
        });
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{SPIELE}"));
        wait_for_label(&mut harness, "musik");
        let spiele: Channel = serde_json::from_value(serde_json::json!({
            "id": SPIELE, "server_id": HOME, "category_id": null, "name": "spiele",
            "kind": "text", "created_at": "2026-01-01T00:00:00Z",
            "limited": true, "role_ids": [MODERATION, STAMMGAESTE],
        }))
        .expect("channel");
        let home: Uuid = HOME.parse().expect("id");
        open_channel(&harness.ctx.clone(), home, Some(&spiele), None);
        wait_for_label(&mut harness, "Zugriff");
        let trigger = harness.get_by_role_and_label(Role::Button, "Zugriff");
        assert!(
            trigger
                .accesskit_node()
                .description()
                .is_some_and(|d| d.starts_with("Moderation, Stammgäste "))
        );
        trigger.click();
        harness.run_steps(4);
        let checked = |h: &egui_kittest::Harness<'_, crate::Launcher>, name: &str| {
            h.get_by_role_and_label(Role::CheckBox, name)
                .accesskit_node()
                .toggled()
                .is_some_and(|t| t == egui::accesskit::Toggled::True)
        };
        assert!(!checked(&harness, "Alle Mitglieder"));
        assert!(checked(&harness, "Moderation"));
        assert!(checked(&harness, "Stammgäste"));
        harness
            .get_by_role_and_label(Role::CheckBox, "Stammgäste")
            .click();
        harness.run_steps(4);
        assert!(checked(&harness, "Moderation"));
        assert!(!checked(&harness, "Stammgäste"));
        // Esc closes the list, not the dialog.
        harness.key_press(egui::Key::Escape);
        harness.run_steps(4);
        assert!(harness.query_by_label("Stammgäste").is_none());
        let trigger = harness.get_by_role_and_label(Role::Button, "Zugriff");
        assert!(
            trigger
                .accesskit_node()
                .description()
                .is_some_and(|d| d.starts_with("Moderation "))
        );
        harness
            .get_by_role_and_label(Role::Button, "Speichern")
            .click();
        let patch = format!("PATCH /channels/{SPIELE}");
        wait_for(&mut harness, "the patch", |_| {
            sent.lock().iter().any(|(r, _)| *r == patch)
        });
        let body: serde_json::Value = {
            let sent = sent.lock();
            let (_, body) = sent.iter().find(|(r, _)| *r == patch).expect("patch");
            serde_json::from_str(body).expect("json")
        };
        assert_eq!(body["role_ids"], serde_json::json!([MODERATION]));

        // A new channel: "Alle Mitglieder" sends no `role_ids`; choosing
        // a role clears "Alle Mitglieder" and limits it.
        for limit in [false, true] {
            open_channel(&harness.ctx.clone(), home, None, None);
            wait_for_label(&mut harness, "Zugriff");
            harness.run_steps(4);
            harness
                .get_by_role_and_label(Role::TextInput, "Name")
                .focus();
            harness.step();
            harness
                .get_by_role_and_label(Role::TextInput, "Name")
                .type_text(if limit { "geheim" } else { "offen" });
            harness.run_steps(2);
            if limit {
                harness
                    .get_by_role_and_label(Role::Button, "Zugriff")
                    .click();
                harness.run_steps(4);
                assert!(checked(&harness, "Alle Mitglieder"));
                harness
                    .get_by_role_and_label(Role::CheckBox, "Moderation")
                    .click();
                harness.run_steps(4);
                assert!(!checked(&harness, "Alle Mitglieder"));
                harness.key_press(egui::Key::Escape);
                harness.run_steps(4);
            }
            harness
                .get_by_role_and_label(Role::Button, "Erstellen")
                .click();
            let post = format!("POST /servers/{HOME}/channels");
            let count = if limit { 2 } else { 1 };
            wait_for(&mut harness, "the create", |_| {
                sent.lock().iter().filter(|(r, _)| *r == post).count() == count
            });
            let body: serde_json::Value = {
                let sent = sent.lock();
                let (_, body) = sent.iter().rfind(|(r, _)| *r == post).expect("post");
                serde_json::from_str(body).expect("json")
            };
            if limit {
                assert_eq!(body["role_ids"], serde_json::json!([MODERATION]));
            } else {
                assert!(body.get("role_ids").is_none(), "{body}");
            }
            harness.run_steps(4);
        }
    }
}
