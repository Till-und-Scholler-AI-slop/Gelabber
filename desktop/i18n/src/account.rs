//! Einstellungen → Sicherheit, from the account package (v0.10, #519)
//! (web/src/i18n/catalogs/account.ts): the signed-in devices, the data
//! export and deleting the account. The section Anmeldung keeps its
//! `settings` entries. System and browser names are the same in every
//! language and are not catalog strings.

use crate::{Locale, get, plural};

crate::strings! {
    devices_title = "Angemeldete Geräte" | "Signed-in devices";
    devices_load_failed = "Deine Geräte konnten nicht geladen werden."
        | "Your devices could not be loaded.";
    devices_current = "Dieses Gerät" | "This device";
    devices_desktop_only = "Gelabber" | "Gelabber";
    devices_browser = "Browser" | "Browser";
    devices_unknown = "Unbekanntes Gerät" | "Unknown device";
    devices_last_active_now = "gerade eben" | "just now";
    devices_last_active_today = "heute" | "today";
    devices_last_active_yesterday = "gestern" | "yesterday";
    devices_sign_out = "Abmelden" | "Sign out";
    devices_signing_out = "Abmelden…" | "Signing out…";

    data_title = "Kontodaten" | "Account data";
    export_title = "Daten exportieren" | "Export data";
    export_load_failed = "Der Stand deines Exports konnte nicht geladen werden."
        | "The state of your export could not be loaded.";
    export_hint_none = "Deine Nachrichten, Dateien und Kontodaten als Archiv."
        | "Your messages, files and account data as an archive.";
    export_hint_mail = "Du bekommst eine E-Mail, sobald es bereitsteht."
        | "You will get an email as soon as it is ready.";
    export_hint_no_mail = "Du kannst es hier herunterladen, sobald es bereitsteht."
        | "You can download it here as soon as it is ready.";
    export_hint_building = "Wird erstellt. Das kann eine Weile dauern."
        | "Being prepared. This can take a while.";
    export_hint_failed = "Der Export ist fehlgeschlagen." | "The export failed.";
    export_request = "Export anfordern" | "Request export";
    export_building = "Wird erstellt…" | "Being prepared…";
    export_download = "Herunterladen" | "Download";
    export_retry = "Erneut anfordern" | "Request again";

    delete_title = "Konto löschen" | "Delete account";
    delete_hint = "Endgültig. Server, die dir gehören, übergibst oder löschst du vorher."
        | "Final. Hand over or delete the servers you own before you do this.";
    delete_owned_lead = "Dir gehören noch diese Server. Übergib sie an ein anderes Mitglied oder lösche sie, bevor du dein Konto löschst."
        | "You still own these servers. Hand them over to another member or delete them before you delete your account.";
    delete_owned_settings = "Servereinstellungen" | "Server settings";
    delete_confirm_title = "Konto endgültig löschen?" | "Delete your account for good?";
    delete_confirm_lead = "Deine Nachrichten, Dateien, Direktnachrichten und Reaktionen werden gelöscht. Du verlässt alle Server. Das lässt sich nicht rückgängig machen."
        | "Your messages, files, direct messages and reactions will be deleted. You will leave every server. This cannot be undone.";
    delete_confirm_export_hint = "Exportiere deine Daten vorher, wenn du sie behalten willst."
        | "Export your data first if you want to keep it.";
    delete_password = "Passwort" | "Password";
    delete_email = "E-Mail-Adresse" | "Email address";
    delete_email_hint = "Dein Konto hat kein Passwort. Gib zur Bestätigung seine E-Mail-Adresse ein."
        | "Your account has no password. Enter its email address to confirm.";
    delete_email_wrong = "Das ist nicht die Adresse deines Kontos."
        | "That is not your account’s address.";
    delete_deleting = "Löschen…" | "Deleting…";
}

/// `devices.desktop`: "Gelabber für Linux".
pub fn devices_desktop(os: &str) -> String {
    match get() {
        Locale::De => format!("Gelabber für {os}"),
        Locale::En => format!("Gelabber for {os}"),
    }
}

/// `devices.browserOn`: "Firefox unter Windows".
pub fn devices_browser_on(browser: &str, os: &str) -> String {
    match get() {
        Locale::De => format!("{browser} unter {os}"),
        Locale::En => format!("{browser} on {os}"),
    }
}

/// `devices.lastActive`: "Zuletzt aktiv: gestern".
pub fn devices_last_active(when: &str) -> String {
    match get() {
        Locale::De => format!("Zuletzt aktiv: {when}"),
        Locale::En => format!("Last active: {when}"),
    }
}

/// `devices.lastActive.daysAgo`: "vor 12 Tagen".
pub fn devices_last_active_days_ago(count: u64) -> String {
    match get() {
        Locale::De => format!("vor {count} {}", plural(count, "Tag", "Tagen")),
        Locale::En => format!("{count} {} ago", plural(count, "day", "days")),
    }
}

/// `export.hint.ready`: "Bereit bis 18. November 2036, 46,0 MB."; both
/// are formatted already.
pub fn export_hint_ready(date: &str, size: &str) -> String {
    match get() {
        Locale::De => format!("Bereit bis {date}, {size}."),
        Locale::En => format!("Ready until {date}, {size}."),
    }
}

/// `export.tooSoon`: `when` is a relative time ("in 5 Stunden").
pub fn export_too_soon(when: &str) -> String {
    match get() {
        Locale::De => format!("Einen neuen Export kannst du {when} anfordern."),
        Locale::En => format!("You can request a new export {when}."),
    }
}

/// `delete.owned.members`: "5 Mitglieder".
pub fn delete_owned_members(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} {}", plural(count, "Mitglied", "Mitglieder")),
        Locale::En => format!("{n} {}", plural(count, "member", "members")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::on_this_thread;

    #[test]
    fn texts_with_names_and_counts_speak_both_languages() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(devices_desktop("Linux"), "Gelabber für Linux");
        assert_eq!(
            devices_browser_on("Firefox", "Windows"),
            "Firefox unter Windows"
        );
        assert_eq!(
            devices_last_active(&devices_last_active_days_ago(12)),
            "Zuletzt aktiv: vor 12 Tagen"
        );
        assert_eq!(devices_last_active_days_ago(1), "vor 1 Tag");
        assert_eq!(delete_owned_members(1), "1 Mitglied");
        assert_eq!(delete_owned_members(5), "5 Mitglieder");
        drop(_de);
        let _en = on_this_thread(Locale::En);
        assert_eq!(devices_desktop("Linux"), "Gelabber for Linux");
        assert_eq!(devices_browser_on("Safari", "iOS"), "Safari on iOS");
        assert_eq!(devices_last_active_days_ago(2), "2 days ago");
        assert_eq!(
            export_hint_ready("18 November 2036", "46.0 MB"),
            "Ready until 18 November 2036, 46.0 MB."
        );
        assert_eq!(delete_owned_members(1), "1 member");
    }
}
