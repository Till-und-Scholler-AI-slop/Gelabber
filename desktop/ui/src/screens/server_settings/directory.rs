//! Server settings "Verzeichnis" (web pages/ServerDirectory.tsx,
//! serverDirectory.css, #520): the owner's listing form with its preview
//! as Entdecken shows the row, then the join requests, which the owner and
//! `moderate_members` answer. The pending count follows `jq`
//! (`State::join_requests`).

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{
    ApiError, DirectoryLanguage, JoinMode, JoinRequest, Listing, ListingSettings,
    ListingSettingsBody, ServerDetail,
};
use gelabber_i18n::{common, server_settings as texts};
use uuid::Uuid;

use super::Load;
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, failure_message};
use crate::screens::auth::form_error;
use crate::screens::settings::controls::{self, Status};
use crate::task::{Pending, poll};
use crate::text::{self, Block};
use crate::theme::{Theme, corner, mix, space};
use crate::widgets::field::layout_unit;
use crate::widgets::{Avatar, Button, InlineStatus, Switch, snap};

/// `DESCRIPTION_MAX` of web/src/discover/rules.ts.
pub(super) const DESCRIPTION_MAX: usize = 300;
/// `.settings-form`: max-width 560.
const FORM_WIDTH: f32 = 560.0;
/// The form's fields in focus order, for a refused save.
const FIELDS: [&str; 4] = ["listed", "description", "language", "join_mode"];

/// An answer to a request (accept or decline) on its way.
type Answer = Pending<Result<(), ApiError>>;

/// The listing form's draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Form {
    pub listed: bool,
    pub description: String,
    /// Empty until a language is picked.
    pub language: String,
    pub join_mode: JoinMode,
}

impl Form {
    /// `formOf`.
    pub fn of(settings: &ListingSettings) -> Self {
        Self {
            listed: settings.listed,
            description: settings.description.clone(),
            language: settings.language.clone().unwrap_or_default(),
            join_mode: settings.join_mode,
        }
    }

    /// Whether it says something else than what the server holds; the
    /// description counts trimmed.
    pub fn differs(&self, saved: &Self) -> bool {
        self.listed != saved.listed
            || self.description.trim() != saved.description
            || self.language != saved.language
            || self.join_mode != saved.join_mode
    }
}

/// `validateDescription`: required once listed, one line of at most
/// [`DESCRIPTION_MAX`] characters.
pub(super) fn validate_description(raw: &str, listed: bool) -> Option<&'static str> {
    let value = raw.trim();
    if listed && value.is_empty() {
        return Some("required");
    }
    if value.chars().count() > DESCRIPTION_MAX {
        return Some("too_long");
    }
    if value.chars().any(char::is_control) {
        return Some("invalid");
    }
    None
}

/// `validateLanguage`: one of the languages `GET /api/directory` names.
pub(super) fn validate_language(
    code: &str,
    languages: &[DirectoryLanguage],
) -> Option<&'static str> {
    if code.is_empty() {
        Some("required")
    } else if languages.iter().any(|l| l.code == code) {
        None
    } else {
        Some("invalid")
    }
}

/// `oneLine`: pasted line breaks become spaces.
fn one_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut breaking = false;
    for ch in value.chars() {
        if ch == '\r' || ch == '\n' {
            if !breaking {
                out.push(' ');
            }
            breaking = true;
        } else {
            out.push(ch);
            breaking = false;
        }
    }
    out
}

/// The section's state, afresh on every visit.
#[derive(Default)]
pub(super) struct Directory {
    settings: Option<Result<ListingSettings, ApiError>>,
    settings_load: Load<ListingSettings>,
    /// The draft; `None` follows what the server holds.
    form: Option<Form>,
    errors: FormErrors,
    save: Load<ListingSettings>,
    status: Status,
    requests: Option<Result<Vec<JoinRequest>, ApiError>>,
    requests_load: Load<Vec<JoinRequest>>,
    /// The `jq` changes last seen for this server.
    seen: Option<u64>,
    /// The pending requests: from the queue just read, or the last `jq`.
    count: Option<i64>,
    /// An answer in flight: whose request, and whether it accepts.
    answer: Option<(Uuid, bool, Answer)>,
    queue_status: Status,
    /// When the section opened: "vor 2 Stunden" needs no live clock.
    opened_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Directory {
    /// The open requests for the anchor's badge, when there are any.
    pub(super) fn pending(&self) -> Option<u64> {
        self.count.filter(|n| *n > 0).map(|n| n as u64)
    }
}

fn reload_settings(app: &AppCtx, d: &mut Directory, server: Uuid) {
    let http = app.client.http().clone();
    d.settings_load = Some(Pending::spawn(app.client, async move {
        http.listing_settings(server).await
    }));
}

fn reload_requests(app: &AppCtx, d: &mut Directory, server: Uuid) {
    let http = app.client.http().clone();
    d.requests_load = Some(Pending::spawn(app.client, async move {
        http.join_requests(server).await
    }));
}

/// Loads what the section shows and takes the answers; `shown` while the
/// caller sees the section, `owner` for the listing form.
pub(super) fn poll_loads(
    ui: &Ui,
    app: &mut AppCtx,
    d: &mut Directory,
    detail: &ServerDetail,
    shown: bool,
    owner: bool,
) {
    let server = detail.server.id;
    if !shown {
        return;
    }
    if d.opened_at.is_none() {
        d.opened_at = Some(chrono::Utc::now());
    }
    if owner && d.settings.is_none() && d.settings_load.is_none() {
        reload_settings(app, d, server);
    }
    if d.requests.is_none() && d.requests_load.is_none() {
        reload_requests(app, d, server);
    }
    // `jq`: the count changed, so the queue reads itself again.
    let frame = app
        .client
        .with_state(|s| s.join_requests.get(&server).copied());
    if let Some(frame) = frame
        && d.seen != Some(frame.changes)
    {
        let first = d.seen.is_none();
        d.seen = Some(frame.changes);
        d.count = Some(frame.pending);
        if !first || d.requests.is_some() {
            reload_requests(app, d, server);
        }
    }
    if let Some(result) = poll(&mut d.settings_load) {
        d.settings = Some(result);
    }
    if let Some(result) = poll(&mut d.requests_load) {
        // The queue just read is the newest count, too.
        if let Ok(list) = &result {
            d.count = Some(list.len() as i64);
        }
        d.requests = Some(result);
    }
    if let Some(result) = poll(&mut d.save) {
        match result {
            Ok(settings) => {
                d.settings = Some(Ok(settings));
                d.form = None;
                d.status.success(ui, common::saved());
                crate::screens::discover::listing_saved(ui.ctx());
            }
            Err(e) => d.errors.from_failure(&Failure::from_api(&e), &FIELDS),
        }
    }
    let answered = d
        .answer
        .as_mut()
        .and_then(|(user, accept, pending)| pending.take().map(|result| (*user, *accept, result)));
    if let Some((user, accept, result)) = answered {
        d.answer = None;
        match result {
            Ok(()) => {
                // Off the list before the buttons come back.
                if let Some(Ok(list)) = &mut d.requests {
                    list.retain(|r| r.user_id != user);
                    d.count = Some(list.len() as i64);
                }
                if accept {
                    app.client.refresh_server(server);
                }
            }
            Err(e) => d.queue_status.error(
                ui,
                failure_message(&Failure::from_api(&e), chrono::Utc::now()),
            ),
        }
        reload_requests(app, d, server);
    }
    if d.settings_load.is_some()
        || d.requests_load.is_some()
        || d.save.is_some()
        || d.answer.is_some()
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// The section's body under its heading: the owner's form, then the queue.
pub(super) fn section(
    ui: &mut Ui,
    app: &mut AppCtx,
    d: &mut Directory,
    detail: &ServerDetail,
    languages: &[DirectoryLanguage],
    owner: bool,
) {
    if owner {
        let width = ui.available_width().min(FORM_WIDTH);
        controls::column(ui, width, |ui| listing(ui, app, d, detail, languages));
        // `.server-directory-requests`: margin-top 12 inside the section's
        // 14px gap.
        ui.add_space(14.0 + 12.0);
    }
    queue(ui, app, d, detail);
}

/// `ListingSettingsPanel`.
fn listing(
    ui: &mut Ui,
    app: &mut AppCtx,
    d: &mut Directory,
    detail: &ServerDetail,
    languages: &[DirectoryLanguage],
) {
    let server = detail.server.id;
    let settings = match &d.settings {
        Some(Ok(settings)) => settings.clone(),
        Some(Err(_)) => {
            let pending = d.settings_load.is_some();
            if crate::widgets::load_error(ui, texts::directory_load_failed(), pending) && !pending {
                reload_settings(app, d, server);
            }
            return;
        }
        None => {
            controls::hint(ui, texts::directory_loading());
            return;
        }
    };
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let saved = Form::of(&settings);
    let mut form = d.form.clone().unwrap_or_else(|| saved.clone());
    let saving = d.save.is_some();
    let mut edited = false;

    if settings.delisted {
        // `.server-directory-delisted`: padding 10 12, the warning tint.
        let block = Block::wrapped(
            ui,
            texts::directory_delisted(),
            text::BODY,
            c.text,
            width - 26.0,
        );
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, layout_unit(block.height()) + 22.0),
            Sense::hover(),
        );
        let shown = snap(ui.ctx(), rect);
        ui.painter().rect(
            shown,
            corner(theme.radius.md),
            mix(c.bg, c.warning, 0.10),
            Stroke::new(1.0, mix(c.bg, c.warning, 0.40)),
            StrokeKind::Inside,
        );
        block.paint(ui.painter(), rect.min + vec2(13.0, 11.0));
        crate::widgets::announce(&response, egui::Role::Label, texts::directory_delisted());
        ui.add_space(space::S5);
    }

    if listed_switch(ui, &mut form.listed) {
        edited = true;
        d.errors.clear_field("listed");
        d.errors.clear_field("description");
    }
    ui.add_space(space::S5);

    // Beschreibung: label, the 88px field, the count, the error.
    label(ui, texts::directory_description());
    let length = form.description.trim().chars().count();
    let description_error = d.errors.field("description").map(str::to_owned);
    let invalid = description_error.is_some() || length > DESCRIPTION_MAX;
    let field = controls::textarea(
        ui,
        "directory-description",
        texts::directory_description(),
        &mut form.description,
        88.0,
        invalid,
    );
    if field.changed() {
        form.description = one_line(&form.description);
        edited = true;
        d.errors.clear_field("description");
    }
    let count = texts::directory_description_count(length, DESCRIPTION_MAX);
    ui.ctx().accesskit_node_builder(field.id, |node| {
        node.set_description(match &description_error {
            Some(error) => format!("{error} {count}"),
            None => count.clone(),
        });
    });
    ui.add_space(space::S1_5);
    let over = length > DESCRIPTION_MAX;
    controls::paragraph(
        ui,
        &count,
        text::META,
        if over { c.danger } else { c.muted },
        width,
    );
    if let Some(error) = &description_error {
        ui.add_space(space::S1_5);
        controls::paragraph(ui, error, text::META, c.danger, width);
    }
    ui.add_space(space::S5);

    // Sprache: the languages the directory names, "Sprache wählen" until
    // one is picked.
    label(ui, texts::directory_language());
    let options: Vec<(String, String)> =
        std::iter::once((String::new(), texts::directory_language_pick().to_owned()))
            .chain(languages.iter().map(|l| (l.code.clone(), l.name.clone())))
            .collect();
    let select_id = ui.next_auto_id();
    if controls::select(ui, "directory-language", &options, &mut form.language, c.bg) {
        edited = true;
        d.errors.clear_field("language");
    }
    let language_error = d.errors.field("language").map(str::to_owned);
    ui.ctx().accesskit_node_builder(select_id, |node| {
        node.set_label(texts::directory_language());
        if let Some(error) = &language_error {
            node.set_description(error.clone());
        }
    });
    if let Some(error) = &language_error {
        ui.add_space(space::S1_5);
        controls::paragraph(ui, error, text::META, c.danger, width);
    }
    ui.add_space(space::S5);

    // Beitritt: the two modes as radio cards.
    label(ui, texts::directory_join_mode());
    ui.add_space(2.0);
    let modes = [
        (
            JoinMode::Open,
            texts::directory_join_open(),
            texts::directory_join_open_hint(),
        ),
        (
            JoinMode::Request,
            texts::directory_join_request(),
            texts::directory_join_request_hint(),
        ),
    ];
    for (i, (mode, title, hint)) in modes.into_iter().enumerate() {
        if i > 0 {
            ui.add_space(space::S2);
        }
        // The notification dialog's card: the same look, with its radio role.
        let checked = form.join_mode == mode;
        if crate::screens::shell::overlays::notify::radio_card(
            ui,
            title,
            Some(hint),
            checked,
            false,
        ) {
            form.join_mode = mode;
            edited = true;
            d.errors.clear_field("join_mode");
        }
    }
    ui.add_space(space::S5);

    // Vorschau: the row as Entdecken shows it; its buttons do nothing here.
    // The label, then the list 8 + 18px (`.discover-list` margin) under it.
    let line = layout_unit(text::UI.line_px());
    controls::paint(
        ui,
        ui.cursor().min,
        texts::directory_preview(),
        text::UI,
        c.text,
    );
    ui.add_space(line + space::S2 + 18.0);
    let preview = Listing {
        server_id: server,
        name: detail.server.name.clone(),
        icon_url: detail.server.icon_url.clone(),
        description: form.description.trim().to_owned(),
        language: form.language.clone(),
        member_count: detail.members.len() as i64,
        join_mode: form.join_mode,
        member: false,
        request: None,
        retry_at: None,
    };
    let top = ui.cursor().min;
    crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, top.y);
    ui.add_space(1.0);
    crate::screens::discover::preview(ui, app, &preview, languages);
    ui.add_space(space::S5);

    form_error(ui, &d.errors);
    let dirty = form.differs(&saved);
    let mut submit = false;
    controls::row(ui, 12.0, 36.0, |ui| {
        submit = ui
            .add(
                Button::new(common::save())
                    .primary()
                    .disabled(!dirty)
                    .pending(saving.then_some(texts::directory_saving())),
            )
            .clicked();
        let status = d.status.current(ui);
        if !dirty && !saving && status.is_none() {
            controls::paint(
                ui,
                pos2(ui.cursor().min.x, ui.max_rect().center().y - 9.0),
                texts::directory_unchanged(),
                controls::HINT,
                c.muted,
            );
        }
        if let Some((m, t)) = status {
            ui.add(InlineStatus::new(Some(&m), t));
        }
    });
    if edited {
        d.status.clear();
    }
    if submit && dirty && !saving {
        let mut fields: Vec<(&str, String)> = Vec::new();
        match validate_description(&form.description, form.listed) {
            Some("required") => fields.push((
                "description",
                texts::directory_description_required().into(),
            )),
            Some("invalid") => {
                fields.push(("description", texts::directory_description_invalid().into()))
            }
            Some(_) => fields.push((
                "description",
                texts::directory_description_too_long().into(),
            )),
            None => {}
        }
        if validate_language(&form.language, languages).is_some() {
            fields.push(("language", texts::directory_language_required().into()));
        }
        if fields.is_empty() {
            d.status.clear();
            let body = ListingSettingsBody {
                listed: form.listed,
                description: form.description.trim().to_owned(),
                language: form.language.clone(),
                join_mode: form.join_mode,
            };
            let http = app.client.http().clone();
            d.save = Some(Pending::spawn(app.client, async move {
                http.put_listing_settings(server, &body).await
            }));
        } else {
            d.errors.set_fields(fields);
        }
    }
    // The fields keep the focus they have.
    d.errors.focus = None;
    d.form = (form != saved).then_some(form);
}

/// `.ui-label` over a field, and the field's 6px gap under it.
fn label(ui: &mut Ui, s: &str) {
    let theme = Theme::of(ui.ctx());
    controls::paint(ui, ui.cursor().min, s, text::UI, theme.c.text);
    ui.add_space(layout_unit(text::UI.line_px()) + space::S1_5);
}

/// `Switch` with its label (`.ui-choice-row--between`): 36px (the 4px
/// padding inside the min-height), the label left, the switch right; the
/// whole row toggles it.
fn listed_switch(ui: &mut Ui, on: &mut bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let label = texts::directory_listed();
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, *on, label));
    let block = Block::single(ui, label, text::UI, theme.c.text);
    block.paint(
        ui.painter(),
        pos2(rect.left(), rect.center().y - block.height() / 2.0),
    );
    let at = Rect::from_min_size(
        pos2(rect.right() - 38.0, rect.center().y - 11.0),
        vec2(38.0, 22.0),
    );
    let mut value = *on;
    let mut changed = controls::place(
        ui,
        UiBuilder::new().max_rect(at).id_salt("directory-listed"),
        |ui| ui.add(Switch::new(&mut value)).changed(),
    );
    if response.clicked() && !changed {
        value = !value;
        changed = true;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    *on = value;
    changed
}

/// `JoinRequestQueue`: "Beitrittsanfragen" with its status line, then the
/// rows with Annehmen and Ablehnen.
fn queue(ui: &mut Ui, app: &mut AppCtx, d: &mut Directory, detail: &ServerDetail) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let server = detail.server.id;
    let width = ui.available_width();
    controls::row(ui, 12.0, layout_unit(text::UI.line_px()), |ui| {
        let title = texts::join_requests_title();
        let block = Block::single(ui, title, text::UI.weight(600.0), c.text);
        let (rect, response) = ui.allocate_exact_size(block.size(), Sense::hover());
        block.paint(ui.painter(), rect.min);
        crate::widgets::announce(&response, egui::Role::Heading, title);
        if let Some((m, t)) = d.queue_status.current(ui) {
            ui.add(InlineStatus::new(Some(&m), t));
        }
    });
    ui.add_space(10.0);
    let list = match &d.requests {
        None => {
            controls::hint(ui, texts::join_requests_loading());
            return;
        }
        Some(Err(_)) => {
            let pending = d.requests_load.is_some();
            if crate::widgets::load_error(ui, texts::join_requests_load_failed(), pending)
                && !pending
            {
                reload_requests(app, d, server);
            }
            return;
        }
        Some(Ok(list)) if list.is_empty() => {
            controls::hint(ui, texts::join_requests_empty());
            return;
        }
        Some(Ok(list)) => list.clone(),
    };
    let now = d.opened_at.unwrap_or_else(chrono::Utc::now);
    let top = ui.cursor().min;
    crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, top.y);
    ui.add_space(1.0);
    let busy = d.answer.as_ref().map(|(user, accept, _)| (*user, *accept));
    for request in &list {
        if let Some(accept) = request_row(ui, app, request, now, busy, width) {
            d.queue_status.clear();
            let http = app.client.http().clone();
            let user = request.user_id;
            let pending = Pending::spawn(app.client, async move {
                if accept {
                    http.accept_join_request(server, user).await
                } else {
                    http.decline_join_request(server, user).await
                }
            });
            d.answer = Some((user, accept, pending));
        }
    }
}

/// One request: avatar, "Kai · vor 2 Stunden" and the message, then
/// Annehmen and Ablehnen. `Some(accept)` when one was pressed.
fn request_row(
    ui: &mut Ui,
    app: &AppCtx,
    request: &JoinRequest,
    now: chrono::DateTime<chrono::Utc>,
    busy: Option<(Uuid, bool)>,
    width: f32,
) -> Option<bool> {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let accept_label = texts::join_requests_accept();
    let decline_label = texts::join_requests_decline();
    let pending = |accept: bool| busy == Some((request.user_id, accept));
    let accept = Button::new(accept_label)
        .small()
        .primary()
        .disabled(busy.is_some())
        .pending(pending(true).then_some(accept_label));
    let decline = Button::new(decline_label)
        .small()
        .disabled(busy.is_some())
        .pending(pending(false).then_some(decline_label));
    let actions_w = accept.natural_width(ui) + 8.0 + decline.natural_width(ui);
    let text_w = (width - 32.0 - 12.0 - 12.0 - actions_w).max(40.0);
    // A request that came in since the section opened is clamped to it.
    let at = request.created_at.min(now);
    let when = gelabber_i18n::intl::format_when((at - now).num_milliseconds());
    let name = Block::single(ui, &request.name, text::UI, c.text);
    let when = Block::single(ui, &format!(" · {when}"), text::UI.weight(400.0), c.muted);
    let message = (!request.message.is_empty())
        .then(|| Block::wrapped(ui, &request.message, text::BODY, c.muted, text_w));
    let name_h = layout_unit(name.height());
    let text_h = name_h + message.as_ref().map_or(0.0, |m| 2.0 + m.height());
    let inner = text_h.max(32.0);
    let height = (8.0 + inner + 8.0).max(56.0) + 1.0;
    let left = ui.cursor().min.x;
    let top = ui.cursor().min.y;
    let rect = Rect::from_min_size(pos2(left, top), vec2(width, height));
    let mid = top + (height - 1.0) / 2.0;
    let painter = ui.painter().clone();
    crate::widgets::rule_h(&painter, c.border, rect.x_range(), rect.bottom() - 1.0);
    Avatar::new(&request.name)
        .image(request.avatar_url.as_deref(), app.images)
        .paint(
            ui,
            Rect::from_min_size(pos2(left, mid - 16.0), vec2(32.0, 32.0)),
        );
    let x = left + 32.0 + 12.0;
    let mut y = mid - text_h / 2.0;
    let clip = Rect::from_min_size(pos2(x, top), vec2(text_w, height));
    name.paint(&painter.with_clip_rect(clip), pos2(x, y));
    when.paint(&painter.with_clip_rect(clip), pos2(x + name.width(), y));
    y += name_h + 2.0;
    if let Some(message) = &message {
        message.paint(&painter, pos2(x, y));
    }
    let accept_w = accept.natural_width(ui);
    let decline_w = decline.natural_width(ui);
    let bx = left + width - actions_w;
    let accept_rect = Rect::from_min_size(pos2(bx, mid - 16.0), vec2(accept_w, 32.0));
    let decline_rect =
        Rect::from_min_size(pos2(bx + accept_w + 8.0, mid - 16.0), vec2(decline_w, 32.0));
    let accepted = ui.put(accept_rect, accept.width(accept_w));
    let declined = ui.put(decline_rect, decline.width(decline_w));
    // `aria-describedby` the person's name.
    for response in [&accepted, &declined] {
        let name = request.name.clone();
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(name));
    }
    ui.allocate_rect(rect, Sense::hover());
    if accepted.clicked() {
        Some(true)
    } else if declined.clicked() {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn languages() -> Vec<DirectoryLanguage> {
        serde_json::from_value(serde_json::json!([
            {"code": "de", "name": "Deutsch", "listings": 1},
            {"code": "en", "name": "English", "listings": 0}
        ]))
        .unwrap()
    }

    /// The cases of discover/rules.test.ts.
    #[test]
    fn the_listing_rules() {
        assert_eq!(validate_description("  ", true), Some("required"));
        assert_eq!(validate_description("", false), None);
        assert_eq!(validate_description(&"a".repeat(300), true), None);
        assert_eq!(
            validate_description(&"a".repeat(301), false),
            Some("too_long")
        );
        assert_eq!(validate_description("a\u{7}b", true), Some("invalid"));
        assert_eq!(validate_language("", &languages()), Some("required"));
        assert_eq!(validate_language("en", &languages()), None);
        assert_eq!(validate_language("xx", &languages()), Some("invalid"));
        assert_eq!(one_line("a\r\n\nb\nc"), "a b c");
    }

    #[test]
    fn a_draft_differs_from_what_is_saved() {
        let saved = Form {
            listed: true,
            description: "Gemütlich.".into(),
            language: "de".into(),
            join_mode: JoinMode::Request,
        };
        let mut draft = saved.clone();
        draft.description = "  Gemütlich. ".into();
        assert!(!draft.differs(&saved));
        draft.join_mode = JoinMode::Open;
        assert!(draft.differs(&saved));
    }

    // ------------------------------------------------- drawn, kittest

    use std::sync::Arc;

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    use crate::app::testkit::{
        FakeApi, HOME, Reply, launcher, wait_for, wait_for_label, workspace,
    };
    use crate::platform::paths::testing::TempConfig;

    type Harness = egui_kittest::Harness<'static, crate::Launcher>;
    /// Writes with their bodies.
    type Sent = Arc<Mutex<Vec<(String, String)>>>;

    const KAI: &str = "00000000-0000-4000-8000-000000000044";

    struct Stand {
        /// Lena's role on Wohnzimmer and her permissions there.
        role: &'static str,
        permissions: Value,
        directory: bool,
        settings: Value,
    }

    fn settings(listed: bool, description: &str, language: Value, delisted: bool) -> Value {
        json!({
            "listed": listed, "description": description, "language": language,
            "join_mode": "request", "delisted": delisted, "listed_at": null
        })
    }

    fn owner(settings: Value) -> Stand {
        Stand {
            role: "owner",
            permissions: json!(["manage_server", "moderate_members", "manage_channels"]),
            directory: true,
            settings,
        }
    }

    /// Wohnzimmer as `stand` says, with Kai's request in the queue until it
    /// is answered.
    fn api(stand: Stand, sent: Sent) -> FakeApi {
        let answered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        FakeApi::start(true, move |method, path, body| {
            use std::sync::atomic::Ordering;
            let base = format!("/servers/{HOME}");
            let write = || {
                sent.lock()
                    .push((format!("{method} {path}"), body.to_owned()));
            };
            match (method, path) {
                ("GET", "/directory") if stand.directory => Some(Reply::json(json!({
                    "enabled": true,
                    "languages": [
                        {"code": "de", "name": "Deutsch", "listings": 1},
                        {"code": "en", "name": "English", "listings": 0}
                    ]
                }))),
                ("GET", "/directory") => {
                    Some(Reply::json(json!({"enabled": false, "languages": []})))
                }
                ("GET", "/servers") => {
                    let mut list = workspace(method, path)?.body;
                    list[0]["role"] = json!(stand.role);
                    list[0]["permissions"] = stand.permissions.clone();
                    Some(Reply::json(list))
                }
                ("GET", p) if p == base => {
                    let mut server = workspace(method, path)?.body;
                    server["role"] = json!(stand.role);
                    server["permissions"] = stand.permissions.clone();
                    Some(Reply::json(server))
                }
                ("GET", p) if p == format!("{base}/listing") => {
                    Some(Reply::json(stand.settings.clone()))
                }
                ("PUT", p) if p == format!("{base}/listing") => {
                    write();
                    let mut saved: Value = serde_json::from_str(body).ok()?;
                    saved["delisted"] = json!(false);
                    saved["listed_at"] = json!("2026-11-11T10:00:00Z");
                    Some(Reply::json(saved))
                }
                ("GET", p) if p == format!("{base}/join-requests") => {
                    if answered.load(Ordering::SeqCst) {
                        return Some(Reply::json(json!([])));
                    }
                    Some(Reply::json(json!([{
                        "user_id": KAI, "name": "Kai", "avatar_url": null,
                        "message": "Darf ich mitmachen?",
                        "created_at": "2026-11-11T08:00:00Z"
                    }])))
                }
                ("POST", p) if p.starts_with(&format!("{base}/join-requests/")) => {
                    write();
                    answered.store(true, Ordering::SeqCst);
                    Some(Reply::status(204, Value::Null))
                }
                _ => workspace(method, path),
            }
        })
    }

    fn route() -> String {
        format!("/s/{HOME}/settings")
    }

    fn steps(harness: &mut Harness, n: usize) {
        for _ in 0..n {
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// To the section, as the scene does: a row out of view takes no click.
    fn open_section(harness: &mut Harness) {
        wait_for(harness, "the Verzeichnis link", |h| {
            h.query_by_role_and_label(Role::Link, "Verzeichnis")
                .is_some()
        });
        harness
            .get_by_role_and_label(Role::Link, "Verzeichnis")
            .click();
        steps(harness, 30);
    }

    /// Verzeichnis' Speichern: the last one, after Allgemein's.
    fn save<'a>(harness: &'a egui_kittest::Harness<'_, crate::Launcher>) -> egui_kittest::Node<'a> {
        harness
            .query_all_by_role_and_label(Role::Button, "Speichern")
            .last()
            .expect("Speichern")
    }

    fn sent_to(sent: &Sent, line: &str) -> Option<String> {
        sent.lock()
            .iter()
            .find(|(l, _)| l == line)
            .map(|(_, b)| b.clone())
    }

    /// The owner lists the server: the switch, then Speichern sends all
    /// four fields; the preview shows the row others see.
    #[test]
    fn the_owner_lists_the_server() {
        let _config = TempConfig::new("directory-owner");
        let sent = Sent::default();
        let stand = owner(settings(false, "Gemütliche Abende.", json!("de"), false));
        let api = api(stand, Arc::clone(&sent));
        let mut harness = launcher(&api, &route());
        harness.set_size(egui::vec2(1280.0, 1400.0));
        open_section(&mut harness);
        wait_for_label(&mut harness, "Im Verzeichnis zeigen");
        // The preview's button is drawn but takes no click, and screen
        // readers skip it (`inert`).
        let preview = harness.get_by_role_and_label(Role::Button, "Beitritt anfragen");
        assert!(preview.accesskit_node().is_disabled());
        assert!(preview.accesskit_node().is_hidden());
        assert!(save(&harness).accesskit_node().is_disabled());
        harness
            .get_by_role_and_label(Role::Switch, "Im Verzeichnis zeigen")
            .click();
        steps(&mut harness, 4);
        harness
            .get_by_role_and_label(Role::RadioButton, "Offen")
            .click();
        steps(&mut harness, 4);
        // An open server offers Beitreten in the preview.
        harness.get_by_role_and_label(Role::Button, "Beitreten");
        save(&harness).click();
        let put = format!("PUT /servers/{HOME}/listing");
        wait_for(&mut harness, "the save", |_| api.saw(&put));
        let body: Value = serde_json::from_str(&sent_to(&sent, &put).unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"listed": true, "description": "Gemütliche Abende.",
                   "language": "de", "join_mode": "open"})
        );
        // Saved: the form follows the answer, Speichern waits again.
        wait_for(&mut harness, "the saved form", |h| {
            save(h).accesskit_node().is_disabled()
        });
    }

    /// Listed without a description or a language, nothing is sent.
    #[test]
    fn a_listing_needs_a_description_and_a_language() {
        let _config = TempConfig::new("directory-required");
        let sent = Sent::default();
        let api = api(owner(settings(false, "", Value::Null, true)), sent);
        let mut harness = launcher(&api, &route());
        harness.set_size(egui::vec2(1280.0, 1400.0));
        open_section(&mut harness);
        wait_for_label(&mut harness, "Im Verzeichnis zeigen");
        // The operator took it out: the note says so.
        harness.get_by_label("Der Betreiber hat diesen Server aus dem Verzeichnis genommen.");
        harness
            .get_by_role_and_label(Role::Switch, "Im Verzeichnis zeigen")
            .click();
        steps(&mut harness, 4);
        save(&harness).click();
        steps(&mut harness, 10);
        assert!(!api.saw(&format!("PUT /servers/{HOME}/listing")));
        let described = |label: &str, text: &str| {
            harness.query_all_by_label(label).any(|n| {
                n.accesskit_node()
                    .description()
                    .is_some_and(|d| d.contains(text))
            })
        };
        assert!(described(
            "Beschreibung",
            "Im Verzeichnis braucht der Server eine Beschreibung."
        ));
        assert!(described("Sprache", "Bitte eine Sprache wählen."));
    }

    /// Someone who may moderate members answers the requests but has no
    /// listing form; the anchor counts the open requests.
    #[test]
    fn a_moderator_accepts_a_request() {
        let _config = TempConfig::new("directory-moderator");
        let sent = Sent::default();
        let stand = Stand {
            role: "member",
            permissions: json!(["moderate_members"]),
            directory: true,
            settings: settings(true, "Gemütlich.", json!("de"), false),
        };
        let api = api(stand, Arc::clone(&sent));
        let mut harness = launcher(&api, &route());
        open_section(&mut harness);
        wait_for_label(&mut harness, "Annehmen");
        let link = harness.get_by_role_and_label(Role::Link, "Verzeichnis");
        assert_eq!(
            link.accesskit_node().description().as_deref(),
            Some("1 offene Anfrage")
        );
        assert!(harness.query_by_label("Im Verzeichnis zeigen").is_none());
        assert!(!api.saw(&format!("GET /servers/{HOME}/listing")));
        harness
            .get_by_role_and_label(Role::Button, "Annehmen")
            .click();
        let accept = format!("POST /servers/{HOME}/join-requests/{KAI}/accept");
        wait_for(&mut harness, "the answer", |_| api.saw(&accept));
        wait_for(&mut harness, "the row to go", |h| {
            h.query_by_label("Annehmen").is_none()
        });
        assert!(sent_to(&sent, &accept).is_some());
    }

    /// Without the right, or with the directory off, there is no section.
    #[test]
    fn the_section_is_for_the_owner_and_moderators_while_on() {
        let _config = TempConfig::new("directory-gated");
        for stand in [
            Stand {
                role: "member",
                permissions: json!([]),
                directory: true,
                settings: settings(false, "", Value::Null, false),
            },
            Stand {
                directory: false,
                ..owner(settings(false, "", Value::Null, false))
            },
        ] {
            let directory = stand.directory;
            let api = api(stand, Sent::default());
            let mut harness = launcher(&api, &route());
            wait_for(&mut harness, "the members", |h| {
                h.query_by_role_and_label(Role::Link, "Mitglieder")
                    .is_some()
            });
            wait_for(&mut harness, "the status", |_| api.saw("GET /directory"));
            steps(&mut harness, 10);
            assert!(
                harness
                    .query_by_role_and_label(Role::Link, "Verzeichnis")
                    .is_none(),
                "directory {directory}"
            );
            assert!(!api.saw(&format!("GET /servers/{HOME}/join-requests")));
        }
    }
}
