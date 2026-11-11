//! Mail about the account's lifecycle (#519), in the recipient's [`Locale`]:
//! the notice after an account was deleted and the one when a data export
//! is ready, plus the export's `README.txt`. Like every account mail, the
//! texts never repeat what someone typed.

use chrono::{DateTime, Utc};

use super::Locale;

/// Subject and body of the mail sent once a deleted account's data is gone.
/// Sent to the address the account had, in the language it had.
pub fn account_deleted(locale: Locale, instance: &str) -> (String, String) {
    match locale {
        Locale::De => (
            "Dein Gelabber-Konto wurde gelöscht".to_owned(),
            format!(
                "Hallo,\n\n\
                 dein Gelabber-Konto auf {instance} ist gelöscht. Deine Nachrichten, \
                 Dateien, Direktnachrichten und Reaktionen sind entfernt, und du bist \
                 aus allen Servern ausgetreten.\n\n\
                 Diese Mail ist nur eine Bestätigung; du musst nichts tun.\n"
            ),
        ),
        Locale::En => (
            "Your Gelabber account has been deleted".to_owned(),
            format!(
                "Hi,\n\n\
                 Your Gelabber account on {instance} has been deleted. Your messages, \
                 files, direct messages and reactions have been removed, and you have \
                 left every server.\n\n\
                 This email is only a confirmation; there is nothing you need to do.\n"
            ),
        ),
    }
}

/// Subject and body of the mail sent when a data export is ready. It links
/// to the settings page, where the archive downloads for the signed-in
/// account; it carries no token. `base` is the public URL without a
/// trailing slash.
pub fn export_ready(
    locale: Locale,
    base: &str,
    instance: &str,
    expires_at: DateTime<Utc>,
) -> (String, String) {
    match locale {
        Locale::De => {
            let until = expires_at.format("%d.%m.%Y, %H:%M UTC");
            (
                "Dein Gelabber-Datenexport ist bereit".to_owned(),
                format!(
                    "Hallo,\n\n\
                     dein Datenexport auf {instance} ist fertig. Melde dich an und lade \
                     das Archiv hier herunter:\n\n\
                     {base}/settings/konto\n\n\
                     Es steht bis {until} bereit und wird danach gelöscht.\n\n\
                     Hast du keinen Export angefordert? Dann melde dich an und ändere \
                     dein Passwort.\n"
                ),
            )
        }
        Locale::En => {
            let until = expires_at.format("%d/%m/%Y, %H:%M UTC");
            (
                "Your Gelabber data export is ready".to_owned(),
                format!(
                    "Hi,\n\n\
                     Your data export on {instance} is ready. Sign in and download the \
                     archive here:\n\n\
                     {base}/settings/konto\n\n\
                     It is available until {until} and is deleted afterwards.\n\n\
                     Didn't ask for an export? Then sign in and change your password.\n"
                ),
            )
        }
    }
}

/// `README.txt` at the top of a data export archive.
pub fn export_readme(locale: Locale) -> &'static str {
    match locale {
        Locale::De => {
            "Gelabber-Datenexport\n\
             \n\
             Dieses Archiv enthält, was du auf Gelabber geschrieben und hochgeladen hast.\n\
             \n\
             account.json            dein Konto und Profil, deine Einstellungen, angemeldeten\n\
             \x20                       Geräte und Server und die Einladungen, die du erstellt hast\n\
             profile/                dein Profilbild und Banner, wenn du welche hochgeladen hast\n\
             messages/servers/       deine Nachrichten in Serverkanälen, je Server und Kanal,\n\
             \x20                       auch in Servern, die du verlassen hast\n\
             messages/dms/           deine Direktnachrichten, je Unterhaltung\n\
             reactions.json          deine Reaktionen\n\
             files/                  die Dateien, die du gesendet hast\n\
             \n\
             Nachrichten anderer Personen sind nicht enthalten. Die JSON-Dateien sind\n\
             UTF-8, alle Zeiten sind UTC (RFC 3339).\n"
        }
        Locale::En => {
            "Gelabber data export\n\
             \n\
             This archive holds what you wrote and uploaded on Gelabber.\n\
             \n\
             account.json            your account and profile, your settings, signed-in\n\
             \x20                       devices and servers, and the invites you created\n\
             profile/                your profile picture and banner, if you uploaded them\n\
             messages/servers/       your messages in server channels, per server and\n\
             \x20                       channel, also in servers you have left\n\
             messages/dms/           your direct messages, per conversation\n\
             reactions.json          your reactions\n\
             files/                  the files you sent\n\
             \n\
             Other people's messages are not included. The JSON files are UTF-8, and\n\
             all times are UTC (RFC 3339).\n"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_deleted_names_the_instance_in_both_languages() {
        let (de_subject, de_body) = account_deleted(Locale::De, "chat.example.org");
        let (en_subject, en_body) = account_deleted(Locale::En, "chat.example.org");
        assert_eq!(de_subject, "Dein Gelabber-Konto wurde gelöscht");
        assert_eq!(en_subject, "Your Gelabber account has been deleted");
        assert!(
            de_body.contains("Konto auf chat.example.org ist gelöscht"),
            "{de_body}"
        );
        assert_eq!(
            en_body,
            "Hi,\n\nYour Gelabber account on chat.example.org has been deleted. Your \
             messages, files, direct messages and reactions have been removed, and you \
             have left every server.\n\n\
             This email is only a confirmation; there is nothing you need to do.\n"
        );
    }

    #[test]
    fn export_ready_links_to_the_settings_with_the_date_in_each_language() {
        let expires = DateTime::parse_from_rfc3339("2026-11-18T14:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let base = "https://chat.example.org";
        let (de_subject, de_body) = export_ready(Locale::De, base, "chat.example.org", expires);
        let (en_subject, en_body) = export_ready(Locale::En, base, "chat.example.org", expires);
        assert_eq!(de_subject, "Dein Gelabber-Datenexport ist bereit");
        assert_eq!(en_subject, "Your Gelabber data export is ready");
        for body in [&de_body, &en_body] {
            assert!(
                body.contains("\n\nhttps://chat.example.org/settings/konto\n\n"),
                "{body}"
            );
            assert!(body.contains("on chat.example.org") || body.contains("auf chat.example.org"));
            assert!(!body.contains("token"), "{body}");
        }
        assert!(
            de_body.contains("bis 18.11.2026, 14:05 UTC bereit"),
            "{de_body}"
        );
        assert!(
            en_body.contains("until 18/11/2026, 14:05 UTC and"),
            "{en_body}"
        );
    }

    #[test]
    fn the_readme_names_every_part_of_the_archive_in_both_languages() {
        for locale in [Locale::De, Locale::En] {
            let readme = export_readme(locale);
            for part in [
                "account.json",
                "profile/",
                "messages/servers/",
                "messages/dms/",
                "reactions.json",
                "files/",
                "RFC 3339",
            ] {
                assert!(readme.contains(part), "{locale:?} {part}");
            }
            assert!(
                readme
                    .lines()
                    .all(|line| !line.starts_with(' ') || line.starts_with("  "))
            );
        }
        assert!(export_readme(Locale::De).starts_with("Gelabber-Datenexport\n\n"));
        assert!(export_readme(Locale::En).starts_with("Gelabber data export\n\n"));
    }
}
