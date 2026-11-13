//! Feedback to the instance's operators (web/src/i18n/catalogs/feedback.ts,
//! #542): the "Feedback geben" dialog and Einstellungen → Feedback. System
//! names (Linux, macOS, …) and the version are the same in every language
//! and come from elsewhere; language names come from `language`.

use crate::{Locale, get, plural};

crate::strings! {
    dialog_title = "Feedback geben" | "Give feedback";
    dialog_description = "Sag den Betreibern dieser Instanz, was dir auffällt."
        | "Tell the operators of this instance what you notice.";
    /// `<link>` goes to Einstellungen → Feedback and closes the dialog.
    dialog_unread = "Es gibt eine neue Antwort auf dein Feedback. <link>Ansehen</link>"
        | "There is a new reply to your feedback. <link>View</link>";
    kind_label = "Worum geht es?" | "What is it about?";
    kind_idea = "Idee" | "Idea";
    kind_problem = "Problem" | "Problem";
    kind_praise = "Lob" | "Praise";
    text_label = "Deine Nachricht" | "Your message";
    text_placeholder_idea = "Was würdest du dir wünschen?" | "What would you like?";
    text_placeholder_problem = "Was ist passiert, und was hättest du erwartet?"
        | "What happened, and what did you expect?";
    text_placeholder_praise = "Was gefällt dir?" | "What do you like?";
    text_required = "Schreib ein paar Worte." | "Write a few words.";
    text_too_long = "Höchstens 4.000 Zeichen." | "At most 4,000 characters.";
    image_attach = "Bild anhängen" | "Attach image";
    image_remove = "Bild entfernen" | "Remove image";
    image_alt = "Angehängtes Bild" | "Attached image";
    /// The web's hint ends on pasting; the app takes no image from the
    /// clipboard but one dropped on the window (the owner's wording).
    image_hint = "PNG, JPEG oder WebP, höchstens 5 MB. Du kannst ein Bild auch hierher ziehen."
        | "PNG, JPEG or WebP, at most 5 MB. You can also drop one here.";
    image_attached = "Bild angehängt." | "Image attached.";
    image_replaced = "Bild ersetzt." | "Image replaced.";
    image_wrong_type = "Nur PNG, JPEG oder WebP." | "Only PNG, JPEG or WebP.";
    image_too_large = "Das Bild ist größer als 5 MB." | "The image is larger than 5 MB.";
    image_invalid = "Das Bild ließ sich nicht verwenden. Versuch ein anderes."
        | "That image could not be used. Try another one.";
    context_title = "Wird mitgeschickt" | "Sent along";
    context_app = "App" | "App";
    context_system = "System" | "System";
    context_version = "Version" | "Version";
    context_language = "Sprache" | "Language";
    context_page = "Seite" | "Page";
    context_web_app = "Web-App" | "Web app";
    context_unknown_system = "Unbekannt" | "Unknown";
    context_nothing_else = "Sonst nichts: keine Protokolle, keine weiteren Daten."
        | "Nothing else: no logs, no other data.";
    submit = "Senden" | "Send";
    sending = "Wird gesendet…" | "Sending…";
    toast_sent = "Danke! Dein Feedback ist angekommen." | "Thanks! Your feedback has arrived.";
    toast_view = "Ansehen" | "View";
    context_desktop_app = "Desktop-App" | "Desktop app";
    area_give = "Feedback geben" | "Give feedback";
    area_load_failed = "Feedback konnte nicht geladen werden." | "Feedback could not be loaded.";
    area_empty = "Du hast noch kein Feedback geschickt." | "You haven't sent any feedback yet.";
    area_empty_hint = "Ideen, Probleme und Lob gehen direkt an die Betreiber dieser Instanz."
        | "Ideas, problems and praise go straight to the operators of this instance.";
    area_more = "Mehr laden" | "Load more";
    status_open = "Offen" | "Open";
    status_done = "Erledigt" | "Done";
    thread_sent_along = "Mitgeschickt" | "Sent along";
    thread_you = "Du" | "You";
    thread_operators = "Betreiber" | "Operators";
    thread_new = "Neu" | "New";
    thread_done_note = "Die Betreiber haben das als erledigt markiert. Wenn du antwortest, ist es wieder offen."
        | "The operators marked this as done. Replying opens it again.";
    thread_reply = "Antworten" | "Reply";
    thread_reply_label = "Deine Antwort" | "Your reply";
    thread_reply_sent = "Antwort gesendet." | "Reply sent.";
    /// A reply refused with `text: too_many` (`thread.full`).
    thread_full = "Dieser Verlauf ist voll (200 Nachrichten)." | "This thread is full (200 messages).";
    /// A reply to an entry the operators deleted meanwhile (the web's
    /// `toasts.feedbackGone` of the operator catalog).
    entry_gone = "Dieses Feedback gibt es nicht mehr." | "That feedback no longer exists.";
}

/// `text.left`: the counter under the text from 3,600 characters on.
pub fn text_left(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("Noch {n} Zeichen"),
        Locale::En => format!("{n} {}", plural(count, "character left", "characters left")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Locale, on_this_thread};

    #[test]
    fn the_counter_counts_like_the_web() {
        let german = on_this_thread(Locale::De);
        assert_eq!(text_left(1), "Noch 1 Zeichen");
        assert_eq!(text_left(400), "Noch 400 Zeichen");
        drop(german);
        let _english = on_this_thread(Locale::En);
        assert_eq!(text_left(1), "1 character left");
        assert_eq!(text_left(1000), "1,000 characters left");
        assert_eq!(dialog_title(), "Give feedback");
    }
}
