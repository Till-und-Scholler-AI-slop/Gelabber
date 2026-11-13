//! Account mail (#195): subject and plain-text body, in the account's
//! [`Locale`]. `base` is the public URL without a trailing slash, `instance`
//! its host name. The texts never contain what a registrant typed (name,
//! password): a confirmation mail can reach someone who never asked for it.

use super::Locale;
use crate::mail::Mail;

pub fn render(locale: Locale, mail: &Mail, base: &str, instance: &str) -> (String, String) {
    match locale {
        Locale::De => german(mail, base, instance),
        Locale::En => english(mail, base, instance),
    }
}

fn german(mail: &Mail, base: &str, instance: &str) -> (String, String) {
    match mail {
        Mail::ConfirmEmail { token } => (
            "Bestätige deine E-Mail-Adresse für Gelabber".to_owned(),
            format!(
                "Hallo,\n\n\
                 mit dieser Adresse wurde auf {instance} ein Gelabber-Konto angelegt. \
                 Öffne diesen Link, um die Adresse zu bestätigen:\n\n\
                 {base}/verify-email#token={token}\n\n\
                 Der Link gilt 24 Stunden und nur einmal.\n\n\
                 Warst du das nicht? Dann ignoriere diese Mail. Ohne Bestätigung \
                 lässt sich das Konto nicht benutzen.\n"
            ),
        ),
        Mail::ResetPassword { token } => (
            "Neues Passwort für Gelabber".to_owned(),
            format!(
                "Hallo,\n\n\
                 für dein Gelabber-Konto auf {instance} wurde ein neues Passwort \
                 angefordert. Hier kannst du es festlegen:\n\n\
                 {base}/reset-password#token={token}\n\n\
                 Der Link gilt eine Stunde und nur einmal. Danach bist du auf \
                 allen Geräten abgemeldet.\n\n\
                 Hast du nichts angefordert? Dann ignoriere diese Mail; dein \
                 Passwort bleibt, wie es ist.\n"
            ),
        ),
        Mail::AlreadyRegistered => (
            "Dein Gelabber-Konto".to_owned(),
            format!(
                "Hallo,\n\n\
                 gerade wollte jemand auf {instance} ein Gelabber-Konto mit dieser \
                 Adresse anlegen. Du hast dort schon eins.\n\n\
                 Anmelden: {base}/login\n\
                 Passwort vergessen: {base}/forgot-password\n\n\
                 Warst du das nicht? Dann ignoriere diese Mail; an deinem Konto \
                 hat sich nichts geändert.\n"
            ),
        ),
        Mail::BetaInvitation { link } => (
            "Deine Einladung zu Gelabber".to_owned(),
            format!(
                "Hallo,\n\n\
                 du bist eingeladen, auf {instance} ein Gelabber-Konto anzulegen. \
                 Mit diesem Link kannst du dich registrieren:\n\n\
                 {link}\n\n\
                 Die Einladung gilt für ein Konto.\n\n\
                 Hast du keine Einladung erwartet? Dann ignoriere diese Mail.\n"
            ),
        ),
        Mail::ProviderConnected { provider_name } => (
            format!("{provider_name} ist mit deinem Gelabber-Konto verbunden"),
            format!(
                "Hallo,\n\n\
                 du kannst dich auf {instance} jetzt auch über {provider_name} bei \
                 deinem Gelabber-Konto anmelden.\n\n\
                 Anmeldung verwalten: {base}/settings/konto\n\n\
                 Warst du das nicht? Dann melde dich an, trenne {provider_name} unter \
                 Einstellungen → Sicherheit und ändere dein Passwort.\n"
            ),
        ),
        Mail::AccountDeleted => super::account::account_deleted(Locale::De, instance),
        Mail::ExportReady { expires_at } => {
            super::account::export_ready(Locale::De, base, instance, *expires_at)
        }
        Mail::FeedbackConfirm { link } => super::feedback::confirm(Locale::De, instance, link),
        Mail::FeedbackNotice {
            kind,
            reply,
            sender,
            excerpt,
            link,
        } => super::feedback::notice(Locale::De, instance, *kind, *reply, sender, excerpt, link),
        Mail::FeedbackAnswered => super::feedback::answered(Locale::De, base, instance),
        Mail::FeedbackReply {
            kind,
            sent_on,
            text,
        } => super::feedback::reply(Locale::De, base, instance, *kind, *sent_on, text),
    }
}

fn english(mail: &Mail, base: &str, instance: &str) -> (String, String) {
    match mail {
        Mail::ConfirmEmail { token } => (
            "Confirm your email address for Gelabber".to_owned(),
            format!(
                "Hi,\n\n\
                 A Gelabber account was created on {instance} with this address. \
                 Open this link to confirm the address:\n\n\
                 {base}/verify-email#token={token}\n\n\
                 The link is valid for 24 hours and works once.\n\n\
                 Wasn't you? Then ignore this email. Without confirmation the \
                 account cannot be used.\n"
            ),
        ),
        Mail::ResetPassword { token } => (
            "New password for Gelabber".to_owned(),
            format!(
                "Hi,\n\n\
                 Someone asked for a new password for your Gelabber account on \
                 {instance}. You can set it here:\n\n\
                 {base}/reset-password#token={token}\n\n\
                 The link is valid for one hour and works once. Afterwards you are \
                 signed out on all devices.\n\n\
                 Didn't ask for this? Then ignore this email; your password stays \
                 as it is.\n"
            ),
        ),
        Mail::AlreadyRegistered => (
            "Your Gelabber account".to_owned(),
            format!(
                "Hi,\n\n\
                 Someone just tried to create a Gelabber account on {instance} with \
                 this address. You already have one there.\n\n\
                 Sign in: {base}/login\n\
                 Forgot password: {base}/forgot-password\n\n\
                 Wasn't you? Then ignore this email; nothing about your account \
                 has changed.\n"
            ),
        ),
        Mail::BetaInvitation { link } => (
            "Your invitation to Gelabber".to_owned(),
            format!(
                "Hi,\n\n\
                 You are invited to create a Gelabber account on {instance}. \
                 Use this link to register:\n\n\
                 {link}\n\n\
                 The invitation is good for one account.\n\n\
                 Weren't expecting an invitation? Then ignore this email.\n"
            ),
        ),
        Mail::ProviderConnected { provider_name } => (
            format!("{provider_name} is now connected to your Gelabber account"),
            format!(
                "Hi,\n\n\
                 You can now also sign in to your Gelabber account on {instance} \
                 with {provider_name}.\n\n\
                 Manage sign-in: {base}/settings/konto\n\n\
                 Wasn't you? Then sign in, disconnect {provider_name} under \
                 Settings → Security and change your password.\n"
            ),
        ),
        Mail::AccountDeleted => super::account::account_deleted(Locale::En, instance),
        Mail::ExportReady { expires_at } => {
            super::account::export_ready(Locale::En, base, instance, *expires_at)
        }
        Mail::FeedbackConfirm { link } => super::feedback::confirm(Locale::En, instance, link),
        Mail::FeedbackNotice {
            kind,
            reply,
            sender,
            excerpt,
            link,
        } => super::feedback::notice(Locale::En, instance, *kind, *reply, sender, excerpt, link),
        Mail::FeedbackAnswered => super::feedback::answered(Locale::En, base, instance),
        Mail::FeedbackReply {
            kind,
            sent_on,
            text,
        } => super::feedback::reply(Locale::En, base, instance, *kind, *sent_on, text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://chat.example.org";
    const INSTANCE: &str = "chat.example.org";

    fn mails() -> [Mail; 9] {
        [
            Mail::ConfirmEmail {
                token: "abc".into(),
            },
            Mail::ResetPassword {
                token: "abc".into(),
            },
            Mail::AlreadyRegistered,
            Mail::BetaInvitation {
                link: format!("{BASE}/register?invitation=abc"),
            },
            Mail::ProviderConnected {
                provider_name: "Google".into(),
            },
            Mail::FeedbackConfirm {
                link: format!("{BASE}/api/public/feedback/confirm?token=abc"),
            },
            Mail::FeedbackNotice {
                kind: crate::feedback::Kind::Problem,
                reply: false,
                sender: "Tom (tom@example.org)".into(),
                excerpt: "Es hakt".into(),
                link: format!("{BASE}/betrieb/feedback?entry=abc"),
            },
            Mail::FeedbackAnswered,
            Mail::FeedbackReply {
                kind: crate::feedback::Kind::Idea,
                sent_on: chrono::Utc::now(),
                text: "Danke".into(),
            },
        ]
    }

    #[test]
    fn both_languages_carry_the_same_links() {
        for mail in mails() {
            let (de_subject, de_body) = render(Locale::De, &mail, BASE, INSTANCE);
            let (en_subject, en_body) = render(Locale::En, &mail, BASE, INSTANCE);
            assert_ne!(de_subject, en_subject);
            // The reply to a visitor links the form in the mail's language.
            let links = |body: &str| -> Vec<String> {
                body.split_whitespace()
                    .filter(|word| word.starts_with(BASE))
                    .map(|word| word.replace(&format!("{BASE}/de/"), &format!("{BASE}/en/")))
                    .collect()
            };
            assert!(!links(&de_body).is_empty(), "{de_body}");
            assert_eq!(links(&de_body), links(&en_body), "{}", mail.kind());
            assert!(de_body.contains(INSTANCE) && en_body.contains(INSTANCE));
        }
    }

    #[test]
    fn german_texts_are_unchanged() {
        let (subject, body) = render(Locale::De, &mails()[0], BASE, INSTANCE);
        assert_eq!(subject, "Bestätige deine E-Mail-Adresse für Gelabber");
        assert!(body.starts_with(
            "Hallo,\n\nmit dieser Adresse wurde auf chat.example.org ein Gelabber-Konto angelegt. "
        ));
        assert!(body.contains("\n\nhttps://chat.example.org/verify-email#token=abc\n\n"));
    }

    #[test]
    fn english_texts() {
        let [confirm, reset, already, invitation, connected, ..] = mails();
        assert_eq!(
            render(Locale::En, &confirm, BASE, INSTANCE),
            (
                "Confirm your email address for Gelabber".to_owned(),
                "Hi,\n\nA Gelabber account was created on chat.example.org with this address. \
                 Open this link to confirm the address:\n\n\
                 https://chat.example.org/verify-email#token=abc\n\n\
                 The link is valid for 24 hours and works once.\n\n\
                 Wasn't you? Then ignore this email. Without confirmation the account \
                 cannot be used.\n"
                    .to_owned()
            )
        );
        assert_eq!(
            render(Locale::En, &reset, BASE, INSTANCE),
            (
                "New password for Gelabber".to_owned(),
                "Hi,\n\nSomeone asked for a new password for your Gelabber account on \
                 chat.example.org. You can set it here:\n\n\
                 https://chat.example.org/reset-password#token=abc\n\n\
                 The link is valid for one hour and works once. Afterwards you are signed \
                 out on all devices.\n\n\
                 Didn't ask for this? Then ignore this email; your password stays as it is.\n"
                    .to_owned()
            )
        );
        assert_eq!(
            render(Locale::En, &already, BASE, INSTANCE),
            (
                "Your Gelabber account".to_owned(),
                "Hi,\n\nSomeone just tried to create a Gelabber account on chat.example.org \
                 with this address. You already have one there.\n\n\
                 Sign in: https://chat.example.org/login\n\
                 Forgot password: https://chat.example.org/forgot-password\n\n\
                 Wasn't you? Then ignore this email; nothing about your account has changed.\n"
                    .to_owned()
            )
        );
        assert_eq!(
            render(Locale::En, &invitation, BASE, INSTANCE),
            (
                "Your invitation to Gelabber".to_owned(),
                "Hi,\n\nYou are invited to create a Gelabber account on chat.example.org. \
                 Use this link to register:\n\n\
                 https://chat.example.org/register?invitation=abc\n\n\
                 The invitation is good for one account.\n\n\
                 Weren't expecting an invitation? Then ignore this email.\n"
                    .to_owned()
            )
        );
        assert_eq!(
            render(Locale::En, &connected, BASE, INSTANCE),
            (
                "Google is now connected to your Gelabber account".to_owned(),
                "Hi,\n\nYou can now also sign in to your Gelabber account on chat.example.org \
                 with Google.\n\n\
                 Manage sign-in: https://chat.example.org/settings/konto\n\n\
                 Wasn't you? Then sign in, disconnect Google under Settings → Security and \
                 change your password.\n"
                    .to_owned()
            )
        );
    }
}
