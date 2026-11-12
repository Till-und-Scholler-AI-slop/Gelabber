//! "Profil" (name and picture) of pages/ProfilePage.tsx. "Sicherheit"
//! moved to security.rs, as the web's to SecurityArea.tsx.

use egui::Ui;
use gelabber_client::{ApiError, ProfilePatch, User};
use gelabber_i18n::{common, settings as texts};

use super::controls::{self, Picked};
use super::{adopt_user, op_pending, settled};
use crate::app::AppCtx;
use crate::rules::{Failure, FormErrors, field_message, validate_name};
use crate::screens::auth::form_error;
use crate::theme::Theme;
use crate::widgets::{Avatar, AvatarSize, Button, TextField};

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
                    Button::new(texts::profile_upload())
                        .small()
                        .disabled(busy && !uploading)
                        .pending(uploading.then_some(texts::profile_uploading())),
                )
                .clicked();
            if upload_clicked && !busy && s.picking.is_none() {
                s.picking = Some(controls::pick_file(
                    ui.ctx(),
                    texts::profile_pick_label(),
                    &controls::IMAGE_FILTER,
                ));
            }
            if !s.avatar_url.trim().is_empty() {
                let removing = s.running == Some("remove");
                let remove = ui
                    .add(
                        Button::new(common::remove())
                            .small()
                            .ghost()
                            .disabled(busy && !removing)
                            .pending(removing.then_some(texts::profile_removing())),
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
            .label(texts::profile_name())
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
            texts::profile_avatar_url_replaces_upload()
        } else if removal_unsaved {
            texts::profile_avatar_url_removed_on_save()
        } else {
            texts::profile_avatar_url_hint()
        };
        // An uploaded picture shows as an empty field with its own placeholder.
        let mut shown = if uploaded {
            String::new()
        } else {
            s.avatar_url.clone()
        };
        let url = TextField::new("avatar_url", &mut shown)
            .label(texts::profile_avatar_url())
            .placeholder(if uploaded {
                texts::profile_avatar_url_uploaded()
            } else {
                texts::profile_avatar_url_placeholder()
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
                    Button::new(common::save())
                        .primary()
                        .disabled(!dirty || (busy && !saving))
                        .pending(saving.then_some(texts::profile_saving())),
                )
                .clicked();
            if unchanged {
                controls::paint(
                    ui,
                    egui::pos2(ui.cursor().min.x, ui.max_rect().center().y - 9.0),
                    texts::profile_unchanged(),
                    controls::HINT,
                    theme.c.muted,
                );
            }
            if s.saved && !dirty {
                ui.add(crate::widgets::InlineStatus::new(
                    Some(common::saved()),
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
