//! The server directory "Entdecken" (web/src/i18n/catalogs/discovery.ts):
//! the page, its sidebar, the rows and their actions, the request and
//! report dialogs and the toast of an accepted request. Language names are
//! the API's endonyms, not catalog strings.

use crate::{Locale, get, plural};

crate::strings! {
    page_title = "Entdecken" | "Discover";
    page_lead = "Server, die ihre Inhaber ins Verzeichnis gestellt haben. Alle anderen bleiben unsichtbar."
        | "Servers their owners have put in the directory. All others stay invisible.";
    page_search = "Nach Name oder Thema suchen" | "Search by name or topic";
    page_empty = "Keine Server gefunden." | "No servers found.";
    page_more = "Mehr laden" | "Load more";
    page_load_failed = "Das Verzeichnis konnte nicht geladen werden."
        | "The directory could not be loaded.";
    page_list_label = "Server im Verzeichnis" | "Servers in the directory";
    sidebar_label = "Entdecken" | "Discover";
    sidebar_language = "Sprache" | "Language";
    sidebar_all = "Alle" | "All";
    actions_join = "Beitreten" | "Join";
    actions_joining = "Tritt bei…" | "Joining…";
    actions_request = "Beitritt anfragen" | "Request to join";
    actions_requested = "Angefragt" | "Requested";
    actions_declined = "Nicht angenommen" | "Not accepted";
    actions_open = "Öffnen" | "Open";
    request_title = "Beitritt anfragen" | "Request to join";
    request_message = "Nachricht (optional)" | "Message (optional)";
    request_message_hint = "Bis zu 300 Zeichen." | "Up to 300 characters.";
    request_submit = "Anfrage senden" | "Send request";
    request_sending = "Wird gesendet…" | "Sending…";
    actions_report = "Melden" | "Report";
    actions_withdraw = "Anfrage zurückziehen" | "Withdraw request";
    report_title = "Server melden" | "Report server";
    report_reason = "Grund" | "Reason";
    report_reason_required = "Bitte einen Grund wählen." | "Please choose a reason.";
    report_reasons_spam = "Spam" | "Spam";
    report_reasons_harassment = "Belästigung" | "Harassment";
    report_reasons_sexual = "Sexuelle Inhalte" | "Sexual content";
    report_reasons_illegal = "Rechtswidrige Inhalte" | "Illegal content";
    report_reasons_other = "Sonstiges" | "Other";
    report_details = "Details" | "Details";
    report_details_hint = "Bei „Sonstiges“ nötig. Bis zu 1000 Zeichen."
        | "Required for “Other”. Up to 1,000 characters.";
    report_details_required = "Bei „Sonstiges“ bitte kurz beschreiben, worum es geht."
        | "For “Other”, please describe briefly what it is about.";
    report_submit = "Melden" | "Report";
    report_sending = "Wird gesendet…" | "Sending…";
    report_sent = "Danke, die Meldung ist beim Betreiber."
        | "Thank you, the report has reached the operator.";
}

/// `row.meta`: "Deutsch · 48 Mitglieder"; `members` is [`row_members`].
pub fn row_meta(language: &str, members: &str) -> String {
    format!("{language} · {members}")
}

/// `row.members`.
pub fn row_members(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} {}", plural(count, "Mitglied", "Mitglieder")),
        Locale::En => format!("{n} {}", plural(count, "member", "members")),
    }
}

/// `actions.declinedUntil`: the title of "Nicht angenommen"; `date` is
/// formatted already.
pub fn actions_declined_until(date: &str) -> String {
    match get() {
        Locale::De => format!("Du kannst ab dem {date} wieder anfragen."),
        Locale::En => format!("You can ask again from {date}."),
    }
}

/// `request.lead`.
pub fn request_lead(name: &str) -> String {
    match get() {
        Locale::De => format!("Anfrage an {name}"),
        Locale::En => format!("Request to {name}"),
    }
}

/// `actions.more`: the row menu's label.
pub fn actions_more(name: &str) -> String {
    match get() {
        Locale::De => format!("Mehr zu {name}"),
        Locale::En => format!("More about {name}"),
    }
}

/// `report.lead`.
pub fn report_lead(name: &str) -> String {
    match get() {
        Locale::De => format!("Meldung zu {name}. Sie geht an den Betreiber."),
        Locale::En => format!("Report about {name}. It goes to the operator."),
    }
}

/// `toasts.joined`: a request to join was accepted.
pub fn toasts_joined(name: &str) -> String {
    match get() {
        Locale::De => format!("Du bist jetzt Mitglied von {name}."),
        Locale::En => format!("You are now a member of {name}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::on_this_thread;

    #[test]
    fn texts_with_names_and_counts_speak_both_languages() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(
            toasts_joined("Filmclub"),
            "Du bist jetzt Mitglied von Filmclub."
        );
        assert_eq!(
            row_meta("Deutsch", &row_members(48)),
            "Deutsch · 48 Mitglieder"
        );
        assert_eq!(row_members(1), "1 Mitglied");
        drop(_de);
        let _en = on_this_thread(Locale::En);
        assert_eq!(
            toasts_joined("Filmclub"),
            "You are now a member of Filmclub."
        );
        assert_eq!(row_members(1_234), "1,234 members");
        assert_eq!(actions_more("Filmclub"), "More about Filmclub");
    }
}
