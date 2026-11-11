//! Signing in and the screens around it (screens/auth, the invite page,
//! "not found"), plus the copy for API error and field codes that every
//! form shows: `error_<code>` and `field_*` (web/src/i18n/catalogs/auth.ts).

use crate::intl::{Unit, format_relative};
use crate::{Locale, get, plural};

crate::strings! {
    // Form-level API error codes (rules.rs `error_message`).
    error_invalid_credentials = "E-Mail oder Passwort ist falsch." | "Wrong email or password.";
    error_theme_conflict = "Auf einem anderen Gerät geändert. Bitte erneut versuchen."
        | "Changed on another device. Please try again.";
    error_email_taken = "Diese E-Mail-Adresse ist schon registriert."
        | "This email address is already registered.";
    error_email_unverified = "Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt."
        | "Please confirm your email address first. We’ve just sent you a new link.";
    error_token_invalid = "Dieser Link ist abgelaufen oder wurde schon benutzt."
        | "This link has expired or has already been used.";
    error_unauthenticated = "Bitte melde dich erneut an." | "Please sign in again.";
    error_csrf_invalid = "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen."
        | "Security token expired. Please try again.";
    error_insecure_transport = "Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost)."
        | "Signing in only works over an encrypted connection. Open Gelabber with https:// (or on this computer via http://localhost).";
    error_forbidden = "Dafür fehlt dir die Berechtigung." | "You don’t have permission for that.";
    error_not_found = "Das gibt es nicht (mehr)." | "That doesn’t exist (any more).";
    error_invite_invalid = "Diese Einladung ist abgelaufen oder aufgebraucht."
        | "This invite has expired or been used up.";
    error_banned = "Du bist auf diesem Server gesperrt." | "You are banned from this server.";
    error_rate_limited = "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen."
        | "Too many requests. Please wait a moment and try again.";
    error_quota_exceeded = "Tageslimit für Uploads erreicht." | "Daily upload limit reached.";
    error_upstream_unavailable = "Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen."
        | "The GIF service isn’t responding right now. Please try again in a moment.";
    error_upstream_quota = "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen."
        | "This server has used up its GIF quota for now. Please try again later.";
    error_validation_failed = "Bitte die markierten Felder prüfen."
        | "Please check the highlighted fields.";
    error_timeout = "Der Server antwortet nicht. Bitte noch einmal versuchen."
        | "The server isn’t responding. Please try again.";
    error_network = "Keine Verbindung zum Server." | "No connection to the server.";
    error_internal = "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen."
        | "Something went wrong. Please try again.";

    // Field names inside field messages (rules.rs `field_label`).
    field_label_email = "E-Mail-Adresse" | "Email address";
    field_label_password = "Passwort" | "Password";
    field_label_current_password = "Aktuelles Passwort" | "Current password";
    field_label_name = "Name" | "Name";
    field_label_avatar_url = "Avatar-URL" | "Avatar URL";
    field_label_bio = "Über mich" | "About me";
    field_label_pronouns = "Pronomen" | "Pronouns";
    field_label_status_text = "Status" | "Status";
    field_label_accent_color = "Akzentfarbe" | "Accent colour";
    field_label_banner_color = "Bannerfarbe" | "Banner colour";
    field_label_banner_url = "Banner" | "Banner";
    field_label_avatar_decoration = "Avatar-Rahmen" | "Avatar frame";
    field_label_kind = "Kanaltyp" | "Channel type";
    field_label_category_id = "Kategorie" | "Category";
    field_label_member_permissions = "Rechte" | "Permissions";
    field_label_max_uses = "Maximale Nutzungen" | "Maximum uses";
    field_label_expires_in_hours = "Gültigkeit" | "Validity";
    field_label_content = "Nachricht" | "Message";
    field_label_cursor = "Cursor" | "Cursor";
    field_label_limit = "Anzahl" | "Count";
    field_label_user_id = "Nutzer" | "User";
    field_label_filename = "Dateiname" | "File name";
    field_label_content_type = "Dateityp" | "File type";
    field_label_size = "Datei" | "File";
    field_label_attachment_ids = "Anhang" | "Attachment";
    field_label_gif = "GIF" | "GIF";
    field_label_q = "Suche" | "Search";
    field_label_token = "Link" | "Link";
    field_label_other = "Eingabe" | "Input";

    // Field codes without a parameter (rules.rs `field_message`).
    field_error_file_too_large = "Die Datei ist zu groß (max. 25 MB)."
        | "The file is too large (max. 25 MB).";
    field_error_too_many_files = "Höchstens 10 Dateien pro Nachricht."
        | "At most 10 files per message.";
    field_error_too_large = "Alle Dateien zusammen sind zu groß (max. 100 MB)."
        | "All files together are too large (max. 100 MB).";
    field_error_mismatch = "Der Inhalt der Datei passt nicht zu ihrem Typ."
        | "The file’s content doesn’t match its type.";
    field_error_taken = "Diese E-Mail-Adresse ist schon registriert."
        | "This email address is already registered.";
    field_error_wrong = "Das Passwort stimmt nicht." | "The password is wrong.";
    field_error_stale = "Dein Bild wurde inzwischen woanders geändert. Lade die Seite neu."
        | "Your image was changed somewhere else in the meantime. Reload the page.";
    field_error_exclusive = "Ein GIF wird ohne Text und Anhang gesendet."
        | "A GIF is sent without text or attachments.";
    field_error_unavailable = "GIFs sind auf diesem Server nicht eingerichtet."
        | "GIFs aren’t set up on this server.";
    field_error_not_editable = "Ein GIF lässt sich nicht bearbeiten." | "A GIF can’t be edited.";
    field_error_email_invalid = "Das sieht nicht wie eine E-Mail-Adresse aus."
        | "That doesn’t look like an email address.";
    field_error_avatar_url_invalid = "Bitte eine vollständige https-URL angeben."
        | "Please enter a full https URL.";
    field_error_color_invalid = "Bitte eine Farbe im Format #rrggbb wählen."
        | "Please pick a colour in the format #rrggbb.";
    field_error_bio_invalid = "Nur Text und Zeilenumbrüche, keine Steuerzeichen."
        | "Only text and line breaks, no control characters.";
    field_error_line_invalid = "Nur eine Zeile Text, keine Steuerzeichen."
        | "Only one line of text, no control characters.";
    field_error_content_type_invalid = "Dieser Dateityp ist nicht erlaubt."
        | "This file type isn’t allowed.";

    // Problems the sign-out strip shows (auth/session.ts).
    session_logout_unconfirmed = "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut."
        | "The server couldn’t confirm the sign-out. Please try again.";
    /// "Überall abmelden" left before the server confirmed it.
    session_logout_everywhere_unconfirmed = "Die Abmeldung auf allen Geräten konnte nicht bestätigt werden. Bitte versuche es erneut."
        | "The server couldn’t confirm the sign-out on all devices. Please try again.";

    // The auth frame.
    layout_invite_banner = "Einladung erhalten. Melde dich an oder registriere dich, um beizutreten."
        | "You’ve been invited. Sign in or register to join.";
    /// The placeholder while the session is checked.
    layout_connecting = "Verbinde…" | "Connecting…";

    // Shared by the auth pages.
    page_sending = "Wird gesendet…" | "Sending…";
    page_new_link_sent = "Neuer Link gesendet." | "New link sent.";
    page_check_inbox = "Postfach prüfen" | "Check your inbox";
    page_no_mail = "Keine Mail erhalten? Prüfe den Spam-Ordner oder sende den Link erneut."
        | "No email? Check your spam folder or send the link again.";
    page_other_address = "Andere Adresse verwenden" | "Use a different address";
    page_to_sign_in = "Zur Anmeldung" | "Go to sign-in";
    page_back_to_sign_in = "Zurück zur Anmeldung" | "Back to sign-in";
    page_forgot_password = "Passwort vergessen?" | "Forgot your password?";
    page_link_expired = "Link abgelaufen" | "Link expired";
    page_link_used = "Dieser Link ist abgelaufen oder wurde schon benutzt."
        | "This link has expired or has already been used.";

    // /login
    login_session_expired = "Sitzung abgelaufen. Melde dich erneut an."
        | "Session expired. Please sign in again.";
    login_title = "Anmelden" | "Sign in";
    login_new_here = "Neu bei Gelabber?" | "New to Gelabber?";
    login_create_account = "Konto anlegen" | "Create account";
    login_no_account = "Noch kein Konto?" | "No account yet?";
    login_register = "Registrieren" | "Register";
    login_submit = "Anmelden" | "Sign in";
    login_submitting = "Anmelden…" | "Signing in…";
    login_forgot_password_no_mail = "Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt."
        | "Forgot your password? Ask the person who runs this Gelabber server.";

    // /register and the mail it sends
    register_title = "Konto anlegen" | "Create account";
    register_have_account = "Schon registriert?" | "Already registered?";
    register_sign_in = "Anmelden" | "Sign in";
    register_name_hint = "Für andere sichtbar." | "Visible to others.";
    register_submit = "Registrieren" | "Register";
    register_submitting = "Konto wird angelegt…" | "Creating account…";
    register_mail_sent = "Bestätigungslink an <b>{email}</b> gesendet. Er gilt 24 Stunden."
        | "Confirmation link sent to <b>{email}</b>. It is valid for 24 hours.";
    register_then_invite = "Danach geht es weiter zur Einladung."
        | "After that, you’ll continue to the invite.";
    register_resend = "Link erneut senden" | "Send link again";

    // /forgot-password
    forgot_title = "Passwort vergessen" | "Forgot password";
    forgot_lead = "Du erhältst einen Link, um ein neues Passwort festzulegen."
        | "You’ll get a link to set a new password.";
    forgot_submit = "Link schicken" | "Send link";
    forgot_mail_sent = "Falls zu <b>{email}</b> ein Konto existiert, wurde ein Link gesendet. Er gilt eine Stunde."
        | "If an account exists for <b>{email}</b>, a link has been sent. It is valid for one hour.";
    forgot_resend = "Erneut senden" | "Send again";

    // /reset-password
    reset_title = "Neues Passwort" | "New password";
    reset_lead = "Danach bist du auf allen anderen Geräten abgemeldet."
        | "Afterwards you’ll be signed out on all other devices.";
    reset_password_label = "Neues Passwort" | "New password";
    reset_submit = "Passwort speichern" | "Save password";
    reset_submitting = "Wird gespeichert…" | "Saving…";
    reset_done = "Passwort geändert. Andere Geräte sind abgemeldet."
        | "Password changed. Other devices are signed out.";
    reset_request_new = "Neuen Link anfordern" | "Request a new link";

    // /verify-email
    verify_title = "E-Mail-Adresse bestätigen" | "Confirm email address";
    verify_lead = "Gib zur Bestätigung dein Passwort ein." | "Enter your password to confirm.";
    verify_submit = "Bestätigen" | "Confirm";
    verify_submitting = "Wird bestätigt…" | "Confirming…";
    verify_done = "E-Mail-Adresse bestätigt." | "Email address confirmed.";
    verify_sign_in_for_new = "Melde dich an, um einen neuen zu erhalten."
        | "Sign in to get a new one.";

    // /invite/{code}
    invite_load_failed = "Einladung konnte nicht geladen werden" | "The invite couldn’t be loaded";
    invite_to_servers = "Zu deinen Servern" | "Go to your servers";
    invite_to_server = "Zum Server" | "Go to server";
    invite_join = "Beitreten" | "Join";
    invite_joining = "Beitreten…" | "Joining…";
    invite_already_member = "Du bist bereits Mitglied von" | "You’re already a member of";
    invite_invited_to = "Einladung zu" | "Invitation to";
    invite_banned_title = "Kein Zutritt" | "No entry";
    invite_dead_title = "Einladung nicht nutzbar" | "Invite can’t be used";
    invite_not_found = "Dieser Einladungslink existiert nicht oder wurde widerrufen."
        | "This invite link doesn’t exist or has been revoked.";
    invite_ask_for_new = "Frag nach einem neuen Link." | "Ask for a new link.";
    invite_expiry_expired = "abgelaufen" | "expired";

    // Unknown paths
    not_found_title = "Seite nicht gefunden" | "Page not found";
    not_found_body = "Diese Adresse gibt es in Gelabber nicht."
        | "This address doesn’t exist in Gelabber.";
    not_found_home = "Zur Startseite" | "Go to the start page";

    // Sign-in through a provider (#518): its API error codes, the
    // `oidc_error` a flow can end with, and the page the system browser
    // shows when it hands the result back to the app.
    error_oidc_unavailable = "Die Anmeldung beim Anbieter ist gerade nicht erreichbar. Bitte später noch einmal versuchen."
        | "Sign-in through the provider isn’t reachable right now. Please try again later.";
    error_oidc_email_unverified = "Dein Konto beim Anbieter hat keine bestätigte E-Mail-Adresse, damit kann Gelabber dich nicht anmelden."
        | "Your account with the provider has no confirmed email address, so Gelabber can’t sign you in.";
    error_oidc_account_unverified = "Zu dieser E-Mail-Adresse gibt es schon ein Konto, dessen Adresse nie bestätigt wurde. Melde dich mit deinem Passwort an und verbinde den Anbieter dann unter Einstellungen → Sicherheit."
        | "An account with this email address exists, but its address was never confirmed. Sign in with your password, then connect the provider under Settings → Security.";
    error_identity_taken = "Dieses Konto beim Anbieter ist schon mit einem anderen Gelabber-Konto verbunden."
        | "This provider account is already connected to another Gelabber account.";
    error_password_required = "Lege zuerst ein Passwort fest, sonst kannst du dich nicht mehr anmelden."
        | "Set a password first, or you won’t be able to sign in any more.";
    error_reauth_required = "Bitte bestätige zuerst, dass du es bist, und melde dich noch einmal an."
        | "Please confirm it’s you first by signing in again.";
    error_oidc_wrong_origin = "Die Anmeldung über einen Anbieter geht nur unter der eingerichteten Adresse dieses Gelabber-Servers. Öffne Gelabber dort und versuche es noch einmal."
        | "Signing in through a provider only works at this Gelabber server’s configured address. Open Gelabber there and try again.";
    error_session_changed = "Inzwischen ist hier ein anderes Konto angemeldet. Bitte noch einmal versuchen."
        | "Another account has signed in here in the meantime. Please try again.";
    oidc_error_cancelled = "Anmeldung abgebrochen." | "Sign-in cancelled.";
    oidc_error_expired = "Die Anmeldung ist abgelaufen oder wurde in einem anderen Browser gestartet. Bitte noch einmal versuchen."
        | "The sign-in expired or was started in another browser. Please try again.";
    oidc_error_failed = "Die Anmeldung beim Anbieter hat nicht geklappt. Bitte noch einmal versuchen."
        | "Signing in with the provider didn’t work. Please try again.";
    // Native only: the system browser's page after the loopback callback.
    oidc_loopback_done = "Du kannst dieses Fenster schließen und zu Gelabber zurückkehren."
        | "You can close this window and return to Gelabber.";
    error_invitation_required = "Zum Registrieren brauchst du eine Einladung."
        | "You need an invitation to register.";
    error_invitation_invalid = "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt."
        | "This invitation is unknown, expired or already used.";
    error_account_disabled = "Dieses Konto ist gesperrt." | "This account has been disabled.";
    error_server_blocked = "Dieser Server wurde vom Betreiber gesperrt."
        | "This server has been blocked by the operator.";
    error_join_request_required = "Diesem Server trittst du nur auf Anfrage bei."
        | "This server can only be joined by request.";
    error_join_request_declined = "Deine Anfrage wurde nicht angenommen. Du kannst in ein paar Tagen wieder anfragen."
        | "Your request was not accepted. You can ask again in a few days.";
    error_join_requests_full = "Gerade sind zu viele Anfragen offen. Bitte später noch einmal versuchen."
        | "Too many requests are open right now. Please try again later.";
    error_already_member = "Du bist schon Mitglied dieses Servers."
        | "You are already a member of this server.";
}

/// `fieldError.required`: "{label} fehlt.".
pub fn field_error_required(label: &str) -> String {
    match get() {
        Locale::De => format!("{label} fehlt."),
        Locale::En => format!("{label} is missing."),
    }
}

/// `fieldError.passwordTooShort`, also the password fields' hint
/// (`page.passwordHint`).
pub fn field_error_password_too_short(min: usize) -> String {
    match get() {
        Locale::De => format!("Mindestens {min} Zeichen."),
        Locale::En => format!("At least {min} characters."),
    }
}

/// `fieldError.tooShort`.
pub fn field_error_too_short(label: &str) -> String {
    match get() {
        Locale::De => format!("{label} ist zu kurz."),
        Locale::En => format!("{label} is too short."),
    }
}

/// `fieldError.tooLong`.
pub fn field_error_too_long(label: &str) -> String {
    match get() {
        Locale::De => format!("{label} ist zu lang."),
        Locale::En => format!("{label} is too long."),
    }
}

/// `fieldError.invalid`.
pub fn field_error_invalid(label: &str) -> String {
    match get() {
        Locale::De => format!("{label} ist ungültig."),
        Locale::En => format!("{label} is invalid."),
    }
}

/// `form.rateLimited`: a rate limit with a known wait.
pub fn form_rate_limited(count: u64) -> String {
    match get() {
        Locale::De => format!("Zu viele Versuche. Bitte {count} Sekunden warten."),
        Locale::En => format!(
            "Too many attempts. Please wait {count} {}.",
            plural(count, "second", "seconds")
        ),
    }
}

/// `page.resendIn`: the resend button while it waits.
pub fn page_resend_in(seconds: u64) -> String {
    match get() {
        Locale::De => format!("Erneut senden ({seconds} s)"),
        Locale::En => format!("Send again ({seconds} s)"),
    }
}

/// `invite.members`.
pub fn invite_members(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} {}", plural(count, "Mitglied", "Mitglieder")),
        Locale::En => format!("{n} {}", plural(count, "member", "members")),
    }
}

/// The invite's expiry (servers `expiry.in`): `when` is relative, "morgen"
/// / "tomorrow".
pub fn invite_expiry_in(when: &str) -> String {
    match get() {
        Locale::De => format!("läuft {when} ab"),
        Locale::En => format!("expires {when}"),
    }
}

/// Days ahead as `Intl.RelativeTimeFormat` with `numeric: "auto"` says
/// them: "morgen", "übermorgen", "in 3 Tagen" / "tomorrow", "in 2 days".
pub fn invite_expiry_days(days: i64) -> String {
    match (get(), days) {
        (Locale::De, 1) => "morgen".to_owned(),
        (Locale::De, 2) => "übermorgen".to_owned(),
        (Locale::En, 1) => "tomorrow".to_owned(),
        _ => format_relative(days, Unit::Day),
    }
}

/// `error.joinRequestDeclinedUntil`: a declined request to join names the
/// day from which the person may ask again; `date` is formatted already.
pub fn error_join_request_declined_until(date: &str) -> String {
    match get() {
        Locale::De => {
            format!(
                "Deine Anfrage wurde nicht angenommen. Du kannst ab dem {date} wieder anfragen."
            )
        }
        Locale::En => format!("Your request was not accepted. You can ask again from {date}."),
    }
}
