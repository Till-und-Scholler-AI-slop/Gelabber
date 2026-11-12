//! Mail of the feedback package (#542), in the recipient's [`Locale`]: the
//! visitor's confirmation link, the operators' notice of a new entry or
//! reply, the sender's "there is an answer" and an operator's reply to a
//! visitor. No subject carries anything a person typed. The confirmation
//! never quotes the visitor's text, because anyone can make the form mail
//! any address; only the notices (to operators) and the reply to a visitor
//! (an operator's own words) carry typed text, quoted line by line.

use chrono::{DateTime, Utc};

use super::Locale;
use crate::feedback::Kind;

/// The kind as the mails name it.
pub fn kind_name(locale: Locale, kind: Kind) -> &'static str {
    match (locale, kind) {
        (Locale::De, Kind::Idea) => "Idee",
        (Locale::De, Kind::Problem) => "Problem",
        (Locale::De, Kind::Praise) => "Lob",
        (Locale::En, Kind::Idea) => "Idea",
        (Locale::En, Kind::Problem) => "Problem",
        (Locale::En, Kind::Praise) => "Praise",
    }
}

/// `text` with every line prefixed `> `.
pub fn quote(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The confirmation mail of the public form. `link` confirms the entry.
pub fn confirm(locale: Locale, instance: &str, link: &str) -> (String, String) {
    match locale {
        Locale::De => (
            "Bestätige dein Feedback an Gelabber".to_owned(),
            format!(
                "Hallo,\n\n\
                 über das Feedback-Formular auf {instance} wurde mit dieser Adresse \
                 Feedback an die Betreiber geschickt. Öffne diesen Link, damit es \
                 ankommt:\n\n\
                 {link}\n\n\
                 Der Link gilt 24 Stunden. Ohne Bestätigung wird das Feedback danach \
                 gelöscht.\n\n\
                 Warst du das nicht? Dann ignoriere diese Mail.\n"
            ),
        ),
        Locale::En => (
            "Confirm your feedback to Gelabber".to_owned(),
            format!(
                "Hi,\n\n\
                 Someone sent feedback to the operators of {instance} through its \
                 feedback form, giving this address. Open this link so that it \
                 arrives:\n\n\
                 {link}\n\n\
                 The link is valid for 24 hours. Without confirmation the feedback is \
                 deleted after that.\n\n\
                 Wasn't you? Then ignore this email.\n"
            ),
        ),
    }
}

/// An operator's notice of a new entry, or with `reply` of a new reply
/// from its sender. `sender` and `excerpt` are prepared by the caller;
/// `link` opens the entry in Betrieb.
pub fn notice(
    locale: Locale,
    instance: &str,
    kind: Kind,
    reply: bool,
    sender: &str,
    excerpt: &str,
    link: &str,
) -> (String, String) {
    let name = kind_name(locale, kind);
    let excerpt = quote(excerpt);
    match locale {
        Locale::De => {
            let (subject, intro) = if reply {
                (
                    format!("Neue Antwort auf Feedback: {name}"),
                    format!("auf {instance} gibt es eine neue Antwort auf ein Feedback."),
                )
            } else {
                (
                    format!("Neues Feedback: {name}"),
                    format!("auf {instance} ist neues Feedback angekommen."),
                )
            };
            (
                subject,
                format!(
                    "Hallo,\n\n\
                     {intro}\n\n\
                     Art: {name}\n\
                     Von: {sender}\n\n\
                     {excerpt}\n\n\
                     Ansehen und antworten:\n\
                     {link}\n"
                ),
            )
        }
        Locale::En => {
            let (subject, intro) = if reply {
                (
                    format!("New reply to feedback: {name}"),
                    format!("There is a new reply to feedback on {instance}."),
                )
            } else {
                (
                    format!("New feedback: {name}"),
                    format!("New feedback has arrived on {instance}."),
                )
            };
            (
                subject,
                format!(
                    "Hi,\n\n\
                     {intro}\n\n\
                     Kind: {name}\n\
                     From: {sender}\n\n\
                     {excerpt}\n\n\
                     View and reply:\n\
                     {link}\n"
                ),
            )
        }
    }
}

/// The sender's mail that the operators replied, without the reply.
pub fn answered(locale: Locale, base: &str, instance: &str) -> (String, String) {
    match locale {
        Locale::De => (
            "Antwort auf dein Feedback".to_owned(),
            format!(
                "Hallo,\n\n\
                 die Betreiber von {instance} haben auf dein Feedback geantwortet. Du \
                 liest die Antwort in Gelabber unter Einstellungen → Feedback:\n\n\
                 {base}/settings/feedback\n\n\
                 Dort kannst du auch antworten. Bis du sie gelesen hast, schicken wir \
                 zu weiteren Antworten keine Mail.\n"
            ),
        ),
        Locale::En => (
            "Reply to your feedback".to_owned(),
            format!(
                "Hi,\n\n\
                 The operators of {instance} have replied to your feedback. You can \
                 read the reply in Gelabber under Settings → Feedback:\n\n\
                 {base}/settings/feedback\n\n\
                 You can reply there too. Until you have read it, we send no email \
                 about further replies.\n"
            ),
        ),
    }
}

/// An operator's reply to a visitor, quoting it. `sent_on` is when the
/// visitor sent the entry.
pub fn reply(
    locale: Locale,
    base: &str,
    instance: &str,
    kind: Kind,
    sent_on: DateTime<Utc>,
    text: &str,
) -> (String, String) {
    let name = kind_name(locale, kind);
    let text = quote(text);
    match locale {
        Locale::De => {
            let date = sent_on.format("%d.%m.%Y");
            (
                "Antwort auf dein Feedback an Gelabber".to_owned(),
                format!(
                    "Hallo,\n\n\
                     die Betreiber von {instance} haben auf dein Feedback vom {date} \
                     ({name}) geantwortet:\n\n\
                     {text}\n\n\
                     Auf diese Mail kannst du nicht antworten: Mails an diese Adresse \
                     kommen nicht an. Wenn du noch etwas sagen möchtest, schick neues \
                     Feedback über das Formular:\n\n\
                     {base}/de/feedback\n"
                ),
            )
        }
        Locale::En => {
            let date = sent_on.format("%d/%m/%Y");
            (
                "Reply to your feedback to Gelabber".to_owned(),
                format!(
                    "Hi,\n\n\
                     The operators of {instance} have replied to your feedback of \
                     {date} ({name}):\n\n\
                     {text}\n\n\
                     You cannot reply to this email: mail sent to this address does \
                     not arrive. If you want to add something, send new feedback \
                     through the form:\n\n\
                     {base}/en/feedback\n"
                ),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://chat.example.org";
    const INSTANCE: &str = "chat.example.org";

    fn links(body: &str) -> Vec<&str> {
        body.split_whitespace()
            .filter(|word| word.starts_with(BASE))
            .collect()
    }

    #[test]
    fn quoting_prefixes_every_line() {
        assert_eq!(quote("eins\nzwei\r\n\ndrei"), "> eins\n> zwei\n> \n> drei");
        assert_eq!(quote("one"), "> one");
    }

    #[test]
    fn the_confirmation_carries_the_link_and_no_typed_text() {
        let link = format!("{BASE}/api/public/feedback/confirm?token=abc");
        let (de_subject, de_body) = confirm(Locale::De, INSTANCE, &link);
        let (en_subject, en_body) = confirm(Locale::En, INSTANCE, &link);
        assert_eq!(de_subject, "Bestätige dein Feedback an Gelabber");
        assert_eq!(en_subject, "Confirm your feedback to Gelabber");
        assert_eq!(links(&de_body), [link.as_str()]);
        assert_eq!(links(&de_body), links(&en_body));
        assert!(!de_body.contains("> ") && !en_body.contains("> "));
        assert_eq!(
            en_body,
            "Hi,\n\nSomeone sent feedback to the operators of chat.example.org through \
             its feedback form, giving this address. Open this link so that it arrives:\n\n\
             https://chat.example.org/api/public/feedback/confirm?token=abc\n\n\
             The link is valid for 24 hours. Without confirmation the feedback is deleted \
             after that.\n\n\
             Wasn't you? Then ignore this email.\n"
        );
    }

    #[test]
    fn notices_name_kind_sender_and_quote_the_excerpt() {
        let link = format!("{BASE}/betrieb/feedback?entry=1");
        let (subject, body) = notice(
            Locale::De,
            INSTANCE,
            Kind::Praise,
            false,
            "Tom (tom@example.org)",
            "Toll\nwirklich",
            &link,
        );
        assert_eq!(subject, "Neues Feedback: Lob");
        assert_eq!(
            body,
            "Hallo,\n\nauf chat.example.org ist neues Feedback angekommen.\n\n\
             Art: Lob\nVon: Tom (tom@example.org)\n\n\
             > Toll\n> wirklich\n\n\
             Ansehen und antworten:\nhttps://chat.example.org/betrieb/feedback?entry=1\n"
        );
        let (subject, body) = notice(
            Locale::En,
            INSTANCE,
            Kind::Idea,
            true,
            "ada@example.org",
            "More",
            &link,
        );
        assert_eq!(subject, "New reply to feedback: Idea");
        assert_eq!(
            body,
            "Hi,\n\nThere is a new reply to feedback on chat.example.org.\n\n\
             Kind: Idea\nFrom: ada@example.org\n\n\
             > More\n\n\
             View and reply:\nhttps://chat.example.org/betrieb/feedback?entry=1\n"
        );
        let (subject, _) = notice(Locale::De, INSTANCE, Kind::Problem, true, "", "x", &link);
        assert_eq!(subject, "Neue Antwort auf Feedback: Problem");
        let (subject, _) = notice(Locale::En, INSTANCE, Kind::Praise, false, "", "x", &link);
        assert_eq!(subject, "New feedback: Praise");
    }

    #[test]
    fn the_answer_mail_links_to_the_settings_without_the_reply() {
        for locale in [Locale::De, Locale::En] {
            let (_, body) = answered(locale, BASE, INSTANCE);
            assert_eq!(links(&body), ["https://chat.example.org/settings/feedback"]);
            assert!(body.contains(INSTANCE));
        }
        assert_eq!(
            answered(Locale::De, BASE, INSTANCE).0,
            "Antwort auf dein Feedback"
        );
        assert_eq!(
            answered(Locale::En, BASE, INSTANCE).0,
            "Reply to your feedback"
        );
    }

    #[test]
    fn the_reply_to_a_visitor_quotes_it_with_the_date_and_its_form() {
        let sent_on = DateTime::parse_from_rfc3339("2026-11-08T22:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (subject, body) = reply(
            Locale::De,
            BASE,
            INSTANCE,
            Kind::Problem,
            sent_on,
            "Danke!\nIst behoben.",
        );
        assert_eq!(subject, "Antwort auf dein Feedback an Gelabber");
        assert_eq!(
            body,
            "Hallo,\n\ndie Betreiber von chat.example.org haben auf dein Feedback vom \
             08.11.2026 (Problem) geantwortet:\n\n\
             > Danke!\n> Ist behoben.\n\n\
             Auf diese Mail kannst du nicht antworten: Mails an diese Adresse kommen nicht \
             an. Wenn du noch etwas sagen möchtest, schick neues Feedback über das \
             Formular:\n\n\
             https://chat.example.org/de/feedback\n"
        );
        let (subject, body) = reply(Locale::En, BASE, INSTANCE, Kind::Idea, sent_on, "Thanks");
        assert_eq!(subject, "Reply to your feedback to Gelabber");
        assert!(
            body.contains("replied to your feedback of 08/11/2026 (Idea):\n\n> Thanks\n\n"),
            "{body}"
        );
        assert_eq!(links(&body), ["https://chat.example.org/en/feedback"]);
    }
}
