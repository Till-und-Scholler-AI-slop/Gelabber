//! "Profil" (name and picture) and "Sicherheit" (password, sign out
//! everywhere) of pages/ProfilePage.tsx.

use egui::{Ui, vec2};
use gelabber_client::{ApiError, ProfilePatch, User};

use super::controls::{self, Confirm, Picked};
use super::{adopt_user, op_pending, settled};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, field_message, validate_name, validate_password};
use crate::screens::auth::form_error;
use crate::text;
use crate::theme::Theme;
use crate::widgets::{Avatar, AvatarSize, Button, ButtonVariant, TextField};

/// `isUploadedAvatar`: an image the API stores itself.
pub fn is_uploaded_avatar(url: &str) -> bool {
    uploaded(url.trim(), "/api/avatars/")
}

pub(crate) fn uploaded(url: &str, prefix: &str) -> bool {
    url.strip_prefix(prefix).is_some_and(|rest| {
        let mut parts = rest.split('/');
        matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(a), Some(b), None) if uuid::Uuid::parse_str(a).is_ok() && uuid::Uuid::parse_str(b).is_ok()
        )
    })
}

/// `validateAvatarUrl`.
fn validate_avatar_url(raw: &str) -> Option<&'static str> {
    let v = raw.trim();
    if v.is_empty() || is_uploaded_avatar(v) {
        return None;
    }
    if v.chars().count() > 2048 {
        return Some("too_long");
    }
    // `get`: byte 8 may fall inside a multibyte character.
    let ok = v
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        && v[8..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace() && !"/.?#".contains(c))
        && !v.chars().any(char::is_whitespace);
    (!ok).then_some("invalid")
}

#[derive(Default)]
pub struct ProfileState {
    loaded: bool,
    name: String,
    avatar_url: String,
    saved: bool,
    errors: FormErrors,
    /// Which write is running: `save`, `upload` or `remove`.
    running: Option<&'static str>,
    picking: Option<Picked>,
    /// The picked or dropped image, read and cropped off the UI thread.
    preparing: Option<controls::Prepared>,
    /// The URL field as typed 400 ms ago, for the preview.
    settled_url: String,
    typed_at: f64,
}

const ORDER: [&str; 2] = ["name", "avatar_url"];

fn apply_failure(errors: &mut FormErrors, error: &ApiError) {
    errors.from_failure(&Failure::from_api(error), &ORDER);
}

/// Settings area "Profil".
pub fn profile(ui: &mut Ui, app: &mut AppCtx, user: &User, s: &mut ProfileState) {
    let theme = Theme::of(ui.ctx());
    if !s.loaded {
        s.loaded = true;
        s.name = user.name.clone();
        s.avatar_url = user.avatar_url.clone().unwrap_or_default();
        s.settled_url = s.avatar_url.clone();
    }
    // Answers of the running write.
    let key = match s.running {
        Some("upload") => Some("avatar"),
        Some(_) => Some("profile"),
        None => None,
    };
    if let Some(key) = key
        && let Some(result) = settled(app, key)
    {
        let which = s.running.take();
        match result {
            Ok(()) => {
                if let Some(updated) = app.client.with_state(|st| st.me().cloned()) {
                    adopt_user(app, &updated);
                    match which {
                        Some("save") => {
                            s.name = updated.name.clone();
                            s.avatar_url = updated.avatar_url.clone().unwrap_or_default();
                        }
                        Some("upload") => {
                            s.avatar_url = updated.avatar_url.clone().unwrap_or_default();
                            s.errors.clear_field("avatar_url");
                        }
                        _ => {
                            if is_uploaded_avatar(&s.avatar_url) {
                                s.avatar_url.clear();
                            }
                            s.errors.clear_field("avatar_url");
                        }
                    }
                    s.settled_url = s.avatar_url.clone();
                }
                s.saved = true;
            }
            Err(e) => apply_failure(&mut s.errors, &e),
        }
    }
    // A picked file uploads at once.
    if let Some(slot) = &s.picking {
        let done = slot.lock().take();
        if let Some(file) = done {
            s.picking = None;
            if let Some((bytes, _)) = file {
                prepare(ui.ctx(), s, controls::ImageFile::Bytes(bytes));
            }
        }
    }
    if s.running.is_none()
        && s.preparing.is_none()
        && let Some(file) = controls::dropped_image(ui.ctx())
    {
        prepare(ui.ctx(), s, file);
    }
    if let Some(prepared) = crate::task::poll(&mut s.preparing) {
        match prepared {
            Ok((data, kind)) => {
                s.running = Some("upload");
                app.client.upload_avatar(kind, data);
            }
            Err(message) => s.errors.set_fields(vec![("avatar_url", message)]),
        }
    }

    let busy = s.running.is_some()
        || s.preparing.is_some()
        || op_pending(app, "profile")
        || op_pending(app, "avatar");
    let uploading = s.running == Some("upload") || s.preparing.is_some();
    let dirty = s.name.trim() != user.name
        || s.avatar_url.trim() != user.avatar_url.clone().unwrap_or_default();
    let unchanged = !dirty && !s.saved && !busy;
    let now = ui.input(|i| i.time);
    if s.avatar_url.trim() != s.settled_url && now - s.typed_at > 0.4 {
        s.settled_url = s.avatar_url.trim().to_owned();
    } else if s.avatar_url.trim() != s.settled_url {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    let preview = (validate_avatar_url(&s.settled_url).is_none() && !s.settled_url.is_empty())
        .then(|| s.settled_url.clone());

    controls::column(ui, 560.0, |ui| {
        // Avatar row: lg avatar + actions, gap 16.
        controls::row(ui, 16.0, 64.0, |ui| {
            let name = if s.name.is_empty() {
                &user.name
            } else {
                &s.name
            };
            ui.add(
                Avatar::new(name)
                    .image(preview.as_deref(), app.images)
                    .size(AvatarSize::Lg),
            );
            ui.spacing_mut().item_spacing.x = 8.0;
            let upload_clicked = ui
                .add(
                    Button::new("Bild hochladen")
                        .small()
                        .disabled(busy && !uploading)
                        .pending(uploading.then_some("Lädt hoch…")),
                )
                .clicked();
            if upload_clicked && !busy && s.picking.is_none() {
                s.picking = Some(controls::pick_file(
                    ui.ctx(),
                    "Avatar-Bild auswählen",
                    &controls::IMAGE_FILTER,
                ));
            }
            if !s.avatar_url.trim().is_empty() {
                let removing = s.running == Some("remove");
                let remove = ui
                    .add(
                        Button::new("Entfernen")
                            .small()
                            .ghost()
                            .disabled(busy && !removing)
                            .pending(removing.then_some("Entfernt…")),
                    )
                    .clicked();
                if remove && !busy {
                    s.saved = false;
                    s.errors.clear_field("avatar_url");
                    let saved = user.avatar_url.clone().unwrap_or_default();
                    if is_uploaded_avatar(&saved) && s.avatar_url.trim() == saved {
                        s.running = Some("remove");
                        app.client.update_profile(ProfilePatch {
                            avatar_url: Some(String::new()),
                            ..ProfilePatch::default()
                        });
                    } else {
                        s.avatar_url.clear();
                    }
                }
            }
        });
        ui.add_space(20.0);
        let name = TextField::new("name", &mut s.name)
            .label("Name")
            .char_limit(64)
            .error(s.errors.field("name"))
            .show(ui);
        if name.response.changed() {
            s.saved = false;
            s.errors.clear_field("name");
        }
        ui.add_space(20.0);
        let uploaded = is_uploaded_avatar(&s.avatar_url);
        let removal_unsaved = s.avatar_url.trim().is_empty() && user.avatar_url.is_some();
        let hint = if uploaded {
            "Eine URL hier ersetzt das hochgeladene Bild."
        } else if removal_unsaved {
            "Wird beim Speichern entfernt."
        } else {
            "Wird bei jedem Aufruf von der fremden Seite geladen."
        };
        // An uploaded picture shows as an empty field with its own placeholder.
        let mut shown = if uploaded {
            String::new()
        } else {
            s.avatar_url.clone()
        };
        let url = TextField::new("avatar_url", &mut shown)
            .label("Avatar-URL")
            .placeholder(if uploaded {
                "Hochgeladenes Bild"
            } else {
                "https://…"
            })
            .error(s.errors.field("avatar_url"))
            .hint(Some(hint))
            .show(ui);
        if url.response.changed() {
            s.avatar_url = shown;
            s.saved = false;
            s.typed_at = now;
            s.errors.clear_field("avatar_url");
        }
        ui.add_space(20.0);
        form_error(ui, &s.errors);
        let mut submit = name.submitted || url.submitted;
        controls::row(ui, 12.0, 36.0, |ui| {
            let saving = s.running == Some("save");
            submit |= ui
                .add(
                    Button::new("Speichern")
                        .primary()
                        .disabled(!dirty || (busy && !saving))
                        .pending(saving.then_some("Speichern…")),
                )
                .clicked();
            if unchanged {
                controls::paint(
                    ui,
                    egui::pos2(ui.cursor().min.x, ui.max_rect().center().y - 9.0),
                    "Keine Änderungen.",
                    controls::HINT,
                    theme.c.muted,
                );
            }
            if s.saved && !dirty {
                ui.add(crate::widgets::InlineStatus::new(
                    Some("Gespeichert."),
                    crate::widgets::StatusTone::Success,
                ));
            }
        });
        if submit && dirty && !busy {
            s.saved = false;
            let mut fields = Vec::new();
            if let Some(code) = validate_name(&s.name) {
                fields.push(("name", field_message("name", code)));
            }
            if let Some(code) = validate_avatar_url(&s.avatar_url) {
                fields.push(("avatar_url", field_message("avatar_url", code)));
            }
            if fields.is_empty() {
                let avatar = s.avatar_url.trim().to_owned();
                let saved = user.avatar_url.clone().unwrap_or_default();
                s.running = Some("save");
                app.client.update_profile(ProfilePatch {
                    name: Some(s.name.trim().to_owned()),
                    avatar_url: (avatar != saved).then_some(avatar),
                    ..ProfilePatch::default()
                });
            } else {
                s.errors.set_fields(fields);
            }
        }
        // ProfilePage's forms call useFormErrors() without a form ref: focus stays put.
        s.errors.focus = None;
    });
}

fn prepare(ctx: &egui::Context, s: &mut ProfileState, file: controls::ImageFile) {
    s.saved = false;
    s.preparing = Some(controls::prepare(ctx, file, 256, 256));
}

// ------------------------------------------------------------ security

#[derive(Default)]
pub struct SecurityState {
    current: String,
    password: String,
    changed: bool,
    running: bool,
    errors: FormErrors,
}

const SECURITY_ORDER: [&str; 2] = ["current_password", "password"];

/// Settings area "Sicherheit".
pub fn security(ui: &mut Ui, app: &mut AppCtx, user: &User, s: &mut SecurityState) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    if s.running
        && let Some(result) = settled(app, "password")
    {
        s.running = false;
        match result {
            Ok(()) => {
                s.current.clear();
                s.password.clear();
                s.changed = true;
                if let Some(updated) = app.client.with_state(|st| st.me().cloned()) {
                    adopt_user(app, &updated);
                }
            }
            Err(e) => s
                .errors
                .from_failure(&Failure::from_api(&e), &SECURITY_ORDER),
        }
    }
    if let Some(true) = controls::answer(ui.ctx(), "logout-all") {
        app.client.voice_leave();
        app.client.logout_all();
        app.session.adopt(&serde_json::json!({ "user": null }));
        app.nav.navigate("/login");
        return;
    }
    controls::column(ui, 560.0, |ui| {
        let block = text::Rich::new(controls::HINT)
            .push("Angemeldet als ", controls::HINT, c.muted)
            .push(&user.email, controls::HINT.weight(600.0), c.text)
            .layout(ui);
        crate::text::show(ui, &block);
        ui.add_space(20.0);
        let current = TextField::new("current_password", &mut s.current)
            .label("Aktuelles Passwort")
            .password()
            .error(s.errors.field("current_password"))
            .show(ui);
        if current.response.changed() {
            s.changed = false;
            s.errors.clear_field("current_password");
        }
        ui.add_space(20.0);
        let new = TextField::new("new_password", &mut s.password)
            .label("Neues Passwort")
            .password()
            .error(s.errors.field("password"))
            .hint(Some("Andere Geräte werden dabei abgemeldet."))
            .show(ui);
        if new.response.changed() {
            s.changed = false;
            s.errors.clear_field("password");
        }
        ui.add_space(20.0);
        form_error(ui, &s.errors);
        let mut submit = current.submitted || new.submitted;
        controls::row(ui, 12.0, 36.0, |ui| {
            submit |= ui
                .add(
                    Button::new("Passwort ändern")
                        .primary()
                        .pending(s.running.then_some("Ändern…")),
                )
                .clicked();
            if s.changed {
                ui.add(crate::widgets::InlineStatus::new(
                    Some("Geändert. Andere Geräte sind abgemeldet."),
                    crate::widgets::StatusTone::Success,
                ));
            }
        });
        if submit && !s.running {
            s.changed = false;
            let mut fields = Vec::new();
            if s.current.is_empty() {
                fields.push((
                    "current_password",
                    field_message("current_password", "required"),
                ));
            }
            if let Some(code) = validate_password(&s.password) {
                fields.push(("password", field_message("password", code)));
            }
            if fields.is_empty() {
                s.errors = FormErrors::default();
                s.running = true;
                app.client.change_password(&s.current, &s.password);
            } else {
                s.errors.set_fields(fields);
            }
        }
        // ProfilePage's forms call useFormErrors() without a form ref: focus stays put.
        s.errors.focus = None;

        // `.settings-danger`: margin-top 40, border-top, padding-top 24.
        ui.add_space(32.0);
        let top = ui.cursor().min.y;
        crate::widgets::rule_h(ui.painter(), c.border, ui.max_rect().x_range(), top);
        ui.add_space(25.0);
        controls::paint(
            ui,
            ui.cursor().min,
            "Überall abmelden",
            text::TITLE_SM,
            c.text,
        );
        ui.allocate_exact_size(vec2(560.0, 20.8), egui::Sense::hover());
        // `.settings-danger .settings-hint`: margin 4 0 12.
        ui.add_space(4.0);
        controls::paragraph(
            ui,
            "Beendet jede Anmeldung, auch auf diesem Gerät.",
            controls::HINT,
            c.muted,
            560.0,
        );
        ui.add_space(12.0);
        if ui
            .add(Button::new("Auf allen Geräten abmelden").variant(ButtonVariant::DangerOutline))
            .clicked()
        {
            controls::ask(
                ui.ctx(),
                Confirm::danger(
                    "logout-all",
                    "Auf allen Geräten abmelden?",
                    Some("Auch dieses Gerät. Danach musst du dich überall neu anmelden."),
                    "Überall abmelden",
                ),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_rules() {
        assert!(is_uploaded_avatar(
            "/api/avatars/0b7c5f0e-8a0e-4c11-9f43-3f1d1b2a4c5d/1b7c5f0e-8a0e-4c11-9f43-3f1d1b2a4c5d"
        ));
        assert!(!is_uploaded_avatar("/api/avatars/x/y"));
        assert_eq!(validate_avatar_url(""), None);
        assert_eq!(validate_avatar_url("https://a.example/x.png"), None);
        assert_eq!(validate_avatar_url("http://a.example"), Some("invalid"));
        assert_eq!(validate_avatar_url("https:///x"), Some("invalid"));
    }

    /// A multibyte character across byte 8 once panicked the slice.
    #[test]
    fn avatar_urls_with_umlauts_are_checked_not_sliced() {
        assert_eq!(validate_avatar_url("http://ärzte.example"), Some("invalid"));
        assert_eq!(validate_avatar_url("a😀😀"), Some("invalid"));
        assert_eq!(validate_avatar_url("Avatar ö"), Some("invalid"));
        assert_eq!(validate_avatar_url("https://ärzte.example/a.png"), None);
    }
}
