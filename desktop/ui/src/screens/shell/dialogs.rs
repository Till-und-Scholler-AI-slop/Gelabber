//! The shell's dialogs (components/ServerDialogs.tsx, InviteDialog.tsx,
//! NewMessageDialog.tsx, ConfirmHost.tsx): add, create and join a server,
//! the invite link, channel and category forms, delete confirmations and
//! "Neue Nachricht". One is open at a time; [`show`] draws it.

use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{ApiError, Category, Channel, ChannelKind, ChannelPatch, Invite, OpState};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::overlays::picker::{PickerState, Row, RowIcon};
use super::state::{self, AddStep, Dialog, InviteState, Terms};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::rules;
use crate::task::Pending;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, FormError, InlineStatus, Modal, StatusTone, TextField};

const CREATE_HINT: &str = "Mit #allgemein und einem Sprachkanal.";
const EXPIRY: [(&str, Option<u32>); 4] = [
    ("1 Stunde", Some(1)),
    ("24 Stunden", Some(24)),
    ("7 Tage", Some(168)),
    ("Nie", None),
];
const USES: [(&str, Option<u32>); 4] = [
    ("1", Some(1)),
    ("5", Some(5)),
    ("25", Some(25)),
    ("Unbegrenzt", None),
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
            format!("Kanal „{}“ löschen?", channel.name),
            Some("Alle Nachrichten darin werden gelöscht."),
            "Kanal löschen",
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
            format!("Kategorie „{}“ löschen?", category.name),
            Some("Die Kanäle darin bleiben erhalten."),
            "Kategorie löschen",
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
        } => channel_dialog(ui, app, *server, *editing, voice, name, category, error),
        Dialog::Category {
            server,
            editing,
            name,
            error,
        } => category_dialog(ui, app, *server, *editing, name, error),
        Dialog::NewMessage { picker, waiting } => new_message(ui, app, picker, waiting),
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
        AddStep::Choose => "Server hinzufügen",
        AddStep::Create => "Server erstellen",
        AddStep::Join => "Server beitreten",
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
                        let create = choice(ui, Icon::Plus, "Server erstellen", CREATE_HINT);
                        // The dialog opens on its first control, as showModal does.
                        if crate::widgets::field::mounted(ui.ctx(), Id::unique("gelabber-add-open"))
                        {
                            create.request_focus();
                        }
                        if create.clicked() {
                            *step = AddStep::Create;
                        }
                        if choice(
                            ui,
                            Icon::Link,
                            "Server beitreten",
                            "Mit einem Einladungslink oder Code.",
                        )
                        .clicked()
                        {
                            *step = AddStep::Join;
                        }
                    }
                    AddStep::Create => {
                        let field = TextField::new("server-name", name)
                            .label("Name")
                            .autofocus(true)
                            .char_limit(100)
                            .hint(Some(CREATE_HINT))
                            .error(error.as_deref())
                            .show(ui);
                        if field.response.changed() {
                            *error = None;
                        }
                        let mut submit = field.submitted;
                        buttons(ui, |ui| {
                            submit |= ui
                                .add(
                                    Button::new("Erstellen")
                                        .primary()
                                        .pending(pending.then_some("Erstellen…")),
                                )
                                .clicked();
                            if ui.add(Button::new("Zurück").ghost()).clicked() {
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
                            .label("Einladungslink oder Code")
                            .placeholder("Link oder Code einfügen")
                            .autofocus(true)
                            .error(error.as_deref())
                            .show(ui);
                        if field.response.changed() {
                            *error = None;
                        }
                        let mut submit = field.submitted;
                        buttons(ui, |ui| {
                            submit |= ui.add(Button::new("Weiter").primary()).clicked();
                            if ui.add(Button::new("Zurück").ghost()).clicked() {
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
        return Err("Bitte einen Einladungslink oder Code eingeben.");
    }
    let invalid = "Das sieht nicht wie ein Einladungslink aus.";
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
                return Err("Dieser Link gehört zu einem anderen Gelabber-Server.");
            }
            from_path(url.path())
        }
        None => bare(value),
    }
}

/// `termsSummary`: "gültig 7 Tage · unbegrenzt nutzbar".
fn summary(terms: Terms) -> String {
    let expiry = match EXPIRY[terms.expiry].1 {
        None => "unbegrenzt gültig".to_owned(),
        Some(_) => format!("gültig {}", EXPIRY[terms.expiry].0),
    };
    let uses = match USES[terms.uses].1 {
        None => "unbegrenzt nutzbar".to_owned(),
        Some(n) => format!("{n}-mal nutzbar"),
    };
    format!("{expiry} · {uses}")
}

fn create_invite(app: &AppCtx, server: Uuid, terms: Terms) -> Pending<Result<Invite, ApiError>> {
    let http = app.client.http().clone();
    let (hours, uses) = (EXPIRY[terms.expiry].1, USES[terms.uses].1);
    Pending::spawn(app.client, async move {
        http.create_invite(server, uses, hours).await
    })
}

fn invite_dialog(ui: &mut Ui, app: &mut AppCtx, s: &mut InviteState) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    if s.invite.is_none() && s.creating.is_none() && s.error.is_none() {
        s.creating = Some(create_invite(app, s.server, s.terms));
    }
    if let Some(result) = crate::task::poll(&mut s.creating) {
        match result {
            Ok(invite) => {
                if let Some((old, _)) = s.invite.take() {
                    if !s.handed_out {
                        app.client.revoke_invite(&old.code);
                    }
                    s.status = Some((
                        format!("Neuer Link · {}", summary(s.terms)),
                        StatusTone::Neutral,
                    ));
                }
                s.invite = Some((invite, s.terms));
                s.handed_out = false;
                s.error = None;
            }
            Err(e) => s.error = Some(rules::error_message(e.code()).to_owned()),
        }
    }
    let creating = s.creating.is_some();
    let link = s
        .invite
        .as_ref()
        .map(|(i, _)| app.client.http().invite_link(&i.code))
        .unwrap_or_default();
    let title = format!("Zu {} einladen", s.name);
    let output = Modal::new(("invite", s.server), &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            if s.invite.is_none()
                && let Some(error) = &s.error
            {
                let message = format!("Der Einladungslink konnte nicht erstellt werden. {error}");
                ui.add(FormError::new(&message));
                if ui.add(Button::new("Erneut versuchen").small()).clicked() {
                    s.error = None;
                }
                return;
            }
            // The link and "Link kopieren".
            let row_w = ui.available_width();
            let copy_label = "Link kopieren";
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
                "Link wird erstellt…"
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
                    s.status = Some(("Link wird erstellt…".into(), StatusTone::Neutral));
                } else if !link.is_empty() {
                    let (message, tone, copied) = crate::screens::chat::clipboard::copy_link(
                        ui.ctx(),
                        &link,
                        &summary(s.terms),
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
                    let block = Block::single(ui, &summary(s.terms), text::META, c.muted);
                    paint_mid(ui.painter(), &block, r.left(), r.center().y);
                }
            }
            if s.error.is_some() && s.invite.is_some() {
                text::label(
                    ui,
                    "Der neue Link konnte nicht erstellt werden. Der bisherige gilt weiter.",
                    text::META,
                    c.danger,
                );
            }
            // "Ablauf & Limit ändern".
            ui.add_space(4.0);
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(row_w, 1.0 + 12.0 + 32.0), egui::Sense::hover());
            crate::widgets::hairline_h(ui.painter(), &theme, r.left()..=r.right(), r.top());
            let label = Block::single(ui, "Ablauf & Limit ändern", text::UI, c.muted);
            let summary_rect = Rect::from_min_size(
                pos2(r.left(), r.top() + 13.0),
                Vec2::new(label.width() + 4.0 + 14.0, 32.0),
            );
            let response = common::click(ui, summary_rect, Id::unique("invite-terms"));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, "Ablauf & Limit ändern")
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
                    ("Läuft ab nach", EXPIRY.map(|o| o.0), s.terms.expiry, 0),
                    ("Wie oft nutzbar", USES.map(|o| o.0), s.terms.uses, 1),
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
                    s.creating = Some(create_invite(app, s.server, s.terms));
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
) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let title = if editing.is_some() {
        "Kanal bearbeiten"
    } else {
        "Kanal erstellen"
    };
    let categories = app.client.with_state(|s| {
        s.detail(server)
            .map(|d| d.categories.clone())
            .unwrap_or_default()
    });
    let mut done = false;
    // An open category list takes Esc first, like the web's native select.
    let select_id = Id::unique(("channel-category", server));
    if ctx.data(|d| d.get_temp::<bool>(select_id)).unwrap_or(false)
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        ctx.data_mut(|d| d.insert_temp(select_id, false));
    }
    let output = Modal::new(("channel-dialog", server), title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            if editing.is_none() {
                ui.vertical(|ui| {
                    // The legend sits 2px higher in its fieldset than a label.
                    ui.spacing_mut().item_spacing.y = 6.0;
                    text::label(ui, "Kanaltyp", text::UI, c.text);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let w = (ui.available_width() - 8.0) / 2.0;
                        for (is_voice, icon, label, detail) in [
                            (false, Icon::Hash, "Text", "Nachrichten, Dateien"),
                            (true, Icon::Speaker, "Voice", "Sprache, Live"),
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
                .map(|s| format!("Wird gespeichert als #{s}"));
            let field = TextField::new("channel-name", name)
                .label("Name")
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
                text::label(ui, "Kategorie", text::UI, c.text);
                let mut options = vec![(None, "Ohne Kategorie".to_owned())];
                options.extend(categories.iter().map(|x| (Some(x.id), x.name.clone())));
                select(ui, select_id, &options, category);
            });
            let mut submit = field.submitted;
            buttons(ui, |ui| {
                submit |= ui
                    .add(
                        Button::new(if editing.is_some() {
                            "Speichern"
                        } else {
                            "Erstellen"
                        })
                        .primary(),
                    )
                    .clicked();
                if ui.add(Button::new("Abbrechen").ghost()).clicked() {
                    done = true;
                }
            });
            if submit {
                match rules::validate_name(name) {
                    Some(code) => *error = Some(rules::field_message("name", code)),
                    None => {
                        let name = name.trim();
                        match editing {
                            Some(channel) => app.client.update_channel(
                                server,
                                channel,
                                ChannelPatch {
                                    name: Some(name.to_owned()),
                                    category_id: Some(
                                        category.map(|c| c.to_string()).unwrap_or_default(),
                                    ),
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
        "Kategorie umbenennen"
    } else {
        "Kategorie erstellen"
    };
    let mut done = false;
    let output = Modal::new(("category-dialog", server), title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let field = TextField::new("category-name", name)
                .label("Name")
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
                            "Speichern"
                        } else {
                            "Erstellen"
                        })
                        .primary(),
                    )
                    .clicked();
                if ui.add(Button::new("Abbrechen").ghost()).clicked() {
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
                p.3 = "Unterhaltung fortsetzen".into();
                p.4 = true;
                recent.push(p);
            }
        }
        people.sort_by_cached_key(|p| gelabber_client::store::collation_key(&p.1));
        recent.extend(people);
        (recent, missing)
    });
    let opened = Id::unique("new-message-loaded");
    if !ctx.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
        for id in &missing {
            app.client.refresh_server(*id);
        }
        ctx.data_mut(|d| d.insert_temp(opened, true));
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
        "Wird geladen…".to_owned()
    } else if people.is_empty() {
        "Keine Personen. Direktnachrichten gehen an Mitglieder deiner Server.".to_owned()
    } else {
        format!("Niemand gefunden für „{}“.", picker.query.trim())
    };
    let mut picked = None;
    let busy = waiting.is_some();
    let output = Modal::new("new-message", "Neue Nachricht").show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            picked = super::overlays::picker::show(
                ui,
                app.images,
                "new-message-query",
                picker,
                "Person",
                "Name suchen",
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
                ctx.data_mut(|d| d.remove::<bool>(opened));
                return false;
            }
            None => {
                app.client.start_dm(user);
                *waiting = Some(user);
            }
        }
    }
    let keep = !output.close_requested;
    if !keep {
        ctx.data_mut(|d| d.remove::<bool>(opened));
    }
    keep
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
}
