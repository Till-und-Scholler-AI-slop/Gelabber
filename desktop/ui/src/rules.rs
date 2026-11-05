//! Client-side checks and the German error copy of web/src/auth/rules.ts
//! and useFormErrors.ts, so forms answer exactly like the web.

use std::collections::BTreeMap;

use gelabber_client::ApiError;

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
        "email" => "E-Mail-Adresse",
        "password" => "Passwort",
        "current_password" => "Aktuelles Passwort",
        "name" => "Name",
        "avatar_url" => "Avatar-URL",
        "bio" => "Über mich",
        "pronouns" => "Pronomen",
        "status_text" => "Status",
        "accent_color" => "Akzentfarbe",
        "banner_color" => "Bannerfarbe",
        "banner_url" => "Banner",
        "avatar_decoration" => "Avatar-Rahmen",
        "kind" => "Kanaltyp",
        "category_id" => "Kategorie",
        "member_permissions" => "Rechte",
        "max_uses" => "Maximale Nutzungen",
        "expires_in_hours" => "Gültigkeit",
        "content" => "Nachricht",
        "before" | "after" => "Cursor",
        "limit" => "Anzahl",
        "user_id" => "Nutzer",
        "filename" => "Dateiname",
        "content_type" => "Dateityp",
        "size" => "Datei",
        "attachment_ids" => "Anhang",
        "gif" => "GIF",
        "q" => "Suche",
        "token" => "Link",
        _ => "Eingabe",
    }
}

/// `fieldMessage(field, code)`.
pub fn field_message(field: &str, code: &str) -> String {
    let label = field_label(field);
    match code {
        "required" => format!("{label} fehlt."),
        "too_short" if field == "password" => format!("Mindestens {PASSWORD_MIN} Zeichen."),
        "too_short" => format!("{label} ist zu kurz."),
        "too_long" if field == "size" => "Die Datei ist zu groß (max. 25 MB).".into(),
        "too_long" if field == "attachment_ids" => "Höchstens 10 Dateien pro Nachricht.".into(),
        "too_long" => format!("{label} ist zu lang."),
        "too_large" => "Alle Dateien zusammen sind zu groß (max. 100 MB).".into(),
        "mismatch" => "Der Inhalt der Datei passt nicht zu ihrem Typ.".into(),
        "taken" => "Diese E-Mail-Adresse ist schon registriert.".into(),
        "wrong" => "Das Passwort stimmt nicht.".into(),
        "stale" => "Dein Bild wurde inzwischen woanders geändert. Lade die Seite neu.".into(),
        "exclusive" => "Ein GIF wird ohne Text und Anhang gesendet.".into(),
        "unavailable" => "GIFs sind auf diesem Server nicht eingerichtet.".into(),
        "not_editable" => "Ein GIF lässt sich nicht bearbeiten.".into(),
        _ => match field {
            "email" => "Das sieht nicht wie eine E-Mail-Adresse aus.".into(),
            "avatar_url" => "Bitte eine vollständige https-URL angeben.".into(),
            "accent_color" | "banner_color" => "Bitte eine Farbe im Format #rrggbb wählen.".into(),
            "bio" => "Nur Text und Zeilenumbrüche, keine Steuerzeichen.".into(),
            "pronouns" | "status_text" => "Nur eine Zeile Text, keine Steuerzeichen.".into(),
            "content_type" => "Dieser Dateityp ist nicht erlaubt.".into(),
            _ => format!("{label} ist ungültig."),
        },
    }
}

/// `errorMessage(code)`: the form-level line for an API error code.
pub fn error_message(code: &str) -> &'static str {
    match code {
        "invalid_credentials" => "E-Mail oder Passwort ist falsch.",
        "theme_conflict" => "Auf einem anderen Gerät geändert. Bitte erneut versuchen.",
        "email_taken" => "Diese E-Mail-Adresse ist schon registriert.",
        "email_unverified" => {
            "Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt."
        }
        "token_invalid" => "Dieser Link ist abgelaufen oder wurde schon benutzt.",
        "unauthenticated" => "Bitte melde dich erneut an.",
        "csrf_invalid" => "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen.",
        "insecure_transport" => {
            "Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost)."
        }
        "forbidden" => "Dafür fehlt dir die Berechtigung.",
        "not_found" => "Das gibt es nicht (mehr).",
        "invite_invalid" => "Diese Einladung ist abgelaufen oder aufgebraucht.",
        "banned" => "Du bist auf diesem Server gesperrt.",
        "rate_limited" => "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.",
        "quota_exceeded" => "Tageslimit für Uploads erreicht.",
        "upstream_unavailable" => {
            "Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen."
        }
        "upstream_quota" => {
            "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen."
        }
        "validation_failed" => "Bitte die markierten Felder prüfen.",
        "timeout" => "Der Server antwortet nicht. Bitte noch einmal versuchen.",
        "network" => "Keine Verbindung zum Server.",
        _ => "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.",
    }
}

/// An API failure reduced to what forms need: the error code (with the
/// client-only `network`), and the per-field codes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Failure {
    pub code: String,
    pub fields: BTreeMap<String, String>,
}

impl Failure {
    pub fn from_api(error: &ApiError) -> Self {
        match error {
            ApiError::Api { code, fields, .. } => Self {
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
                fields: BTreeMap::new(),
            },
            ApiError::Decode(_) => Self {
                code: "internal".into(),
                fields: BTreeMap::new(),
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
}

impl FormErrors {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// `setFields`: client-side check results, in form order.
    pub fn set_fields(&mut self, fields: Vec<(&str, String)>) {
        self.focus = fields.first().map(|(f, _)| (*f).to_owned());
        self.form = (fields.len() > 1).then(|| "Bitte die markierten Felder prüfen.".to_owned());
        self.fields = fields.into_iter().map(|(f, m)| (f.to_owned(), m)).collect();
    }

    /// `fromError`; `order` lists the form's fields top to bottom, for focus.
    pub fn from_failure(&mut self, failure: &Failure, order: &[&str]) {
        let fields: BTreeMap<String, String> = failure
            .fields
            .iter()
            .map(|(f, code)| (f.clone(), field_message(f, code)))
            .collect();
        self.form = if fields.is_empty() {
            Some(error_message(&failure.code).to_owned())
        } else if fields.len() > 1 {
            Some("Bitte die markierten Felder prüfen.".to_owned())
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
                fields: BTreeMap::new(),
            },
            &["email"],
        );
        assert_eq!(
            errors.form.as_deref(),
            Some("E-Mail oder Passwort ist falsch.")
        );
    }

    #[test]
    fn tokens() {
        let token = "ab".repeat(32);
        assert_eq!(link_token(&format!("#token={token}")), Some(token.clone()));
        assert_eq!(link_token("#token=xyz"), None);
    }
}
