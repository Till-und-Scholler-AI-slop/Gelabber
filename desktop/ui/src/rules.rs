//! Client-side checks and the error copy of web/src/auth/rules.ts and
//! useFormErrors.ts, so forms answer exactly like the web.

use std::collections::BTreeMap;

use gelabber_client::ApiError;
use gelabber_i18n::auth;

pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 128;
pub const NAME_MAX: usize = 64;
pub const EMAIL_MAX: usize = 254;

/// `validateEmail`: a field code or `None`.
pub fn validate_email(raw: &str) -> Option<&'static str> {
    let value = raw.trim().to_lowercase();
    if value.is_empty() {
        return Some("required");
    }
    if value.chars().count() > EMAIL_MAX {
        return Some("too_long");
    }
    let (Some(at), Some(last)) = (value.find('@'), value.rfind('@')) else {
        return Some("invalid");
    };
    if at == 0 || at != last {
        return Some("invalid");
    }
    let domain = &value[at + 1..];
    if domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || value.chars().any(char::is_whitespace)
    {
        return Some("invalid");
    }
    None
}

/// `validatePassword`.
pub fn validate_password(raw: &str) -> Option<&'static str> {
    let count = raw.chars().count();
    if count == 0 {
        Some("required")
    } else if count < PASSWORD_MIN {
        Some("too_short")
    } else if count > PASSWORD_MAX {
        Some("too_long")
    } else {
        None
    }
}

/// `validateName`.
pub fn validate_name(raw: &str) -> Option<&'static str> {
    let value = raw.trim();
    if value.is_empty() {
        Some("required")
    } else if value.chars().count() > NAME_MAX {
        Some("too_long")
    } else if value.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f) {
        Some("invalid")
    } else {
        None
    }
}

fn field_label(field: &str) -> &'static str {
    match field {
        "email" => auth::field_label_email(),
        "password" => auth::field_label_password(),
        "current_password" => auth::field_label_current_password(),
        "name" => auth::field_label_name(),
        "avatar_url" => auth::field_label_avatar_url(),
        "bio" => auth::field_label_bio(),
        "pronouns" => auth::field_label_pronouns(),
        "status_text" => auth::field_label_status_text(),
        "accent_color" => auth::field_label_accent_color(),
        "banner_color" => auth::field_label_banner_color(),
        "banner_url" => auth::field_label_banner_url(),
        "avatar_decoration" => auth::field_label_avatar_decoration(),
        "kind" => auth::field_label_kind(),
        "category_id" => auth::field_label_category_id(),
        "member_permissions" => auth::field_label_member_permissions(),
        "max_uses" => auth::field_label_max_uses(),
        "expires_in_hours" => auth::field_label_expires_in_hours(),
        "content" => auth::field_label_content(),
        "before" | "after" => auth::field_label_cursor(),
        "limit" => auth::field_label_limit(),
        "user_id" => auth::field_label_user_id(),
        "filename" => auth::field_label_filename(),
        "content_type" => auth::field_label_content_type(),
        "size" => auth::field_label_size(),
        "attachment_ids" => auth::field_label_attachment_ids(),
        "gif" => auth::field_label_gif(),
        "q" => auth::field_label_q(),
        "token" => auth::field_label_token(),
        "invitation" => auth::field_label_invitation(),
        _ => auth::field_label_other(),
    }
}

/// `fieldMessage(field, code)`.
pub fn field_message(field: &str, code: &str) -> String {
    let label = field_label(field);
    match code {
        // The API's `{"invitation":"required"}` (invitation_required).
        "required" if field == "invitation" => auth::error_invitation_required().into(),
        "required" => auth::field_error_required(label),
        "too_short" if field == "password" => auth::field_error_password_too_short(PASSWORD_MIN),
        "too_short" => auth::field_error_too_short(label),
        "too_long" if field == "size" => auth::field_error_file_too_large().into(),
        "too_long" if field == "attachment_ids" => auth::field_error_too_many_files().into(),
        "too_long" => auth::field_error_too_long(label),
        "too_large" => auth::field_error_too_large().into(),
        "mismatch" => auth::field_error_mismatch().into(),
        "taken" => auth::field_error_taken().into(),
        "wrong" => auth::field_error_wrong().into(),
        "stale" => auth::field_error_stale().into(),
        "exclusive" => auth::field_error_exclusive().into(),
        "unavailable" => auth::field_error_unavailable().into(),
        "not_editable" => auth::field_error_not_editable().into(),
        _ => match field {
            "email" => auth::field_error_email_invalid().into(),
            "avatar_url" => auth::field_error_avatar_url_invalid().into(),
            "accent_color" | "banner_color" => auth::field_error_color_invalid().into(),
            "bio" => auth::field_error_bio_invalid().into(),
            "pronouns" | "status_text" => auth::field_error_line_invalid().into(),
            "content_type" => auth::field_error_content_type_invalid().into(),
            "invitation" => auth::error_invitation_invalid().into(),
            _ => auth::field_error_invalid(label),
        },
    }
}

/// `errorMessage(code)`: the form-level line for an API error code.
pub fn error_message(code: &str) -> &'static str {
    match code {
        "invalid_credentials" => auth::error_invalid_credentials(),
        "theme_conflict" => auth::error_theme_conflict(),
        "email_taken" => auth::error_email_taken(),
        "email_unverified" => auth::error_email_unverified(),
        "token_invalid" => auth::error_token_invalid(),
        "unauthenticated" => auth::error_unauthenticated(),
        "csrf_invalid" => auth::error_csrf_invalid(),
        "insecure_transport" => auth::error_insecure_transport(),
        "forbidden" => auth::error_forbidden(),
        "not_found" => auth::error_not_found(),
        "invite_invalid" => auth::error_invite_invalid(),
        "banned" => auth::error_banned(),
        "rate_limited" => auth::error_rate_limited(),
        "quota_exceeded" => auth::error_quota_exceeded(),
        "upstream_unavailable" => auth::error_upstream_unavailable(),
        "upstream_quota" => auth::error_upstream_quota(),
        "validation_failed" => auth::error_validation_failed(),
        "timeout" => auth::error_timeout(),
        "network" => auth::error_network(),
        "oidc_unavailable" => auth::error_oidc_unavailable(),
        "oidc_email_unverified" => auth::error_oidc_email_unverified(),
        "oidc_account_unverified" => auth::error_oidc_account_unverified(),
        "identity_taken" => auth::error_identity_taken(),
        "password_required" => auth::error_password_required(),
        "reauth_required" => auth::error_reauth_required(),
        "oidc_wrong_origin" => auth::error_oidc_wrong_origin(),
        "session_changed" => auth::error_session_changed(),
        "invitation_required" => auth::error_invitation_required(),
        "invitation_invalid" => auth::error_invitation_invalid(),
        "account_disabled" => auth::error_account_disabled(),
        "server_blocked" => auth::error_server_blocked(),
        _ => auth::error_internal(),
    }
}

/// `OIDC_ERRORS`: why a provider sign-in came back without signing in
/// (the `?oidc_error=` of `/login`, `/register` and `/settings/konto`).
pub const OIDC_ERRORS: [&str; 11] = [
    "cancelled",
    "expired",
    "failed",
    "email_unverified",
    "account_unverified",
    "identity_taken",
    "insecure_transport",
    "session_changed",
    "invitation_required",
    "invitation_invalid",
    "account_disabled",
];

/// `isOidcError`: the code when it is one of [`OIDC_ERRORS`].
pub fn oidc_error(value: Option<&str>) -> Option<&'static str> {
    let value = value?;
    OIDC_ERRORS.iter().copied().find(|code| *code == value)
}

/// `oidcErrorMessage`: the copy of an `oidc_error`, in the interface's
/// language. The operator's codes take the client's copy.
pub fn oidc_error_message(code: &str) -> &'static str {
    match code {
        "cancelled" => auth::oidc_error_cancelled(),
        "expired" => auth::oidc_error_expired(),
        "failed" => auth::oidc_error_failed(),
        "email_unverified" => error_message("oidc_email_unverified"),
        "account_unverified" => error_message("oidc_account_unverified"),
        "invitation_required" | "invitation_invalid" | "account_disabled" => {
            gelabber_client::error_message(code)
        }
        _ => error_message(code),
    }
}

/// An API failure reduced to what forms need: the error code (with the
/// client-only `network`), and the per-field codes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Failure {
    pub code: String,
    pub fields: BTreeMap<String, String>,
    /// Seconds a rate limit asks to wait.
    pub retry_after: Option<f64>,
}

impl Failure {
    pub fn from_api(error: &ApiError) -> Self {
        match error {
            ApiError::Api {
                code,
                fields,
                retry_after,
                ..
            } => Self {
                retry_after: *retry_after,
                code: if code.is_empty() {
                    "internal".into()
                } else {
                    code.clone()
                },
                fields: fields
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            ApiError::Transport(_) => Self {
                code: "network".into(),
                ..Self::default()
            },
            ApiError::Decode(_) => Self {
                code: "internal".into(),
                ..Self::default()
            },
        }
    }
}

/// A form's inline errors (`useFormErrors`): one form-level line plus a
/// message per field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormErrors {
    pub form: Option<String>,
    pub fields: BTreeMap<String, String>,
    /// The field to move focus to once the errors render.
    pub focus: Option<String>,
    /// A rate limit's end (`useCooldown`): the submit button waits till then.
    pub cooldown_until: Option<std::time::Instant>,
}

impl FormErrors {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// `setFields`: client-side check results, in form order.
    pub fn set_fields(&mut self, fields: Vec<(&str, String)>) {
        self.focus = fields.first().map(|(f, _)| (*f).to_owned());
        self.form = (fields.len() > 1).then(|| auth::error_validation_failed().to_owned());
        self.fields = fields.into_iter().map(|(f, m)| (f.to_owned(), m)).collect();
    }

    /// Whole seconds left of a rate limit; 0 once it is over.
    pub fn cooldown(&self) -> u64 {
        self.cooldown_until.map_or(0, |until| {
            let left = until.saturating_duration_since(std::time::Instant::now());
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        })
    }

    /// `fromError`; `order` lists the form's fields top to bottom, for focus.
    pub fn from_failure(&mut self, failure: &Failure, order: &[&str]) {
        if failure.code == "rate_limited"
            && let Some(wait) = failure.retry_after.filter(|w| *w > 0.0)
        {
            let seconds = wait.ceil() as u64;
            self.cooldown_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(seconds));
            self.form = Some(auth::form_rate_limited(seconds));
            self.fields.clear();
            self.focus = None;
            return;
        }
        let fields: BTreeMap<String, String> = failure
            .fields
            .iter()
            .map(|(f, code)| (f.clone(), field_message(f, code)))
            .collect();
        self.form = if fields.is_empty() {
            Some(error_message(&failure.code).to_owned())
        } else if fields.len() > 1 {
            Some(auth::error_validation_failed().to_owned())
        } else {
            None
        };
        self.focus = order
            .iter()
            .find(|f| fields.contains_key(**f))
            .map(|f| (*f).to_owned());
        self.fields = fields;
    }

    /// `clearField`: typing in a field drops its error and the form line.
    pub fn clear_field(&mut self, name: &str) {
        if self.fields.remove(name).is_some() || self.form.is_some() {
            self.form = None;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.form.is_none() && self.fields.is_empty()
    }
}

/// `readLinkToken`: the 64-hex `token` of a mailed link's fragment.
pub fn link_token(fragment: &str) -> Option<String> {
    fragment
        .trim_start_matches('#')
        .split('&')
        .find_map(|part| part.strip_prefix("token="))
        .filter(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_rules() {
        assert_eq!(validate_email(""), Some("required"));
        assert_eq!(validate_email("a@b"), Some("invalid"));
        assert_eq!(validate_email("@b.de"), Some("invalid"));
        assert_eq!(validate_email("a@@b.de"), Some("invalid"));
        assert_eq!(validate_email("a@b.de."), Some("invalid"));
        assert_eq!(validate_email(" Lena@Example.test "), None);
        assert_eq!(
            field_message("email", "invalid"),
            "Das sieht nicht wie eine E-Mail-Adresse aus."
        );
        assert_eq!(field_message("password", "required"), "Passwort fehlt.");
        assert_eq!(
            field_message("password", "too_short"),
            "Mindestens 8 Zeichen."
        );
        assert_eq!(field_message("name", "too_long"), "Name ist zu lang.");
    }

    #[test]
    fn form_lines() {
        let mut errors = FormErrors::default();
        errors.set_fields(vec![("email", "x".into())]);
        assert_eq!(errors.form, None);
        errors.set_fields(vec![("email", "x".into()), ("password", "y".into())]);
        assert_eq!(
            errors.form.as_deref(),
            Some("Bitte die markierten Felder prüfen.")
        );
        errors.clear_field("email");
        assert_eq!(errors.form, None);
        let mut errors = FormErrors::default();
        errors.from_failure(
            &Failure {
                code: "invalid_credentials".into(),
                ..Failure::default()
            },
            &["email"],
        );
        assert_eq!(
            errors.form.as_deref(),
            Some("E-Mail oder Passwort ist falsch.")
        );
        assert_eq!(errors.cooldown(), 0);
        // useFormErrors: a rate limit with a known wait counts it down.
        errors.from_failure(
            &Failure {
                code: "rate_limited".into(),
                retry_after: Some(7.2),
                ..Failure::default()
            },
            &["email"],
        );
        assert_eq!(
            errors.form.as_deref(),
            Some("Zu viele Versuche. Bitte 8 Sekunden warten.")
        );
        assert_eq!(errors.cooldown(), 8);
        // Without one, the generic line.
        let mut plain = FormErrors::default();
        plain.from_failure(
            &Failure {
                code: "rate_limited".into(),
                ..Failure::default()
            },
            &["email"],
        );
        assert_eq!(plain.cooldown(), 0);
        assert_eq!(
            plain.form.as_deref(),
            Some("Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.")
        );
    }

    #[test]
    fn tokens() {
        let token = "ab".repeat(32);
        assert_eq!(link_token(&format!("#token={token}")), Some(token.clone()));
        assert_eq!(link_token("#token=xyz"), None);
    }

    /// Every sign-in code has its own copy, and only the listed
    /// `oidc_error` codes are read from an address.
    #[test]
    fn sign_in_codes_have_copy() {
        for code in [
            "oidc_unavailable",
            "oidc_email_unverified",
            "oidc_account_unverified",
            "identity_taken",
            "password_required",
            "reauth_required",
            "oidc_wrong_origin",
            "session_changed",
        ] {
            assert_ne!(error_message(code), auth::error_internal(), "{code}");
        }
        for code in OIDC_ERRORS {
            assert_eq!(oidc_error(Some(code)), Some(code));
            assert_ne!(oidc_error_message(code), auth::error_internal(), "{code}");
        }
        assert_eq!(oidc_error(Some("registration_closed")), None);
        assert_eq!(oidc_error(None), None);
        assert_eq!(
            oidc_error_message("account_unverified"),
            auth::error_oidc_account_unverified()
        );
        assert_eq!(oidc_error_message("cancelled"), "Anmeldung abgebrochen.");
        assert_eq!(
            oidc_error_message("invitation_required"),
            "Zum Registrieren brauchst du eine Einladung."
        );
    }

    /// The operator's codes (auth/rules.test.ts): the form line, and the
    /// invitation field's own copy for `required` and `invalid`.
    #[test]
    fn operator_codes_have_copy() {
        assert_eq!(
            error_message("account_disabled"),
            "Dieses Konto ist gesperrt."
        );
        assert_eq!(
            error_message("server_blocked"),
            "Dieser Server wurde vom Betreiber gesperrt."
        );
        assert_eq!(
            error_message("invitation_required"),
            "Zum Registrieren brauchst du eine Einladung."
        );
        assert_eq!(
            error_message("invitation_invalid"),
            "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt."
        );
        assert_eq!(
            field_message("invitation", "required"),
            auth::error_invitation_required()
        );
        assert_eq!(
            field_message("invitation", "invalid"),
            auth::error_invitation_invalid()
        );
        assert_eq!(field_message("invitation", "too_long"), "Einladungscode ist zu lang.");
        // A sign-in refused for a disabled account says so at the form.
        let mut errors = FormErrors::default();
        errors.from_failure(
            &Failure {
                code: "account_disabled".into(),
                ..Failure::default()
            },
            &["email", "password"],
        );
        assert_eq!(errors.form.as_deref(), Some("Dieses Konto ist gesperrt."));
    }
}
