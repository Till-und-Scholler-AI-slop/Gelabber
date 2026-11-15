//! Friends, friend codes and message requests (#563,
//! web/src/i18n/catalogs/friends.ts): the friend list, the requests page,
//! the friend code, the profile card's button and a request's bar in a
//! conversation.

use crate::{Locale, get};

crate::strings! {
    nav_friends = "Freunde" | "Friends";
    nav_requests = "Anfragen" | "Requests";
    page_title = "Freunde" | "Friends";
    tab_all = "Alle" | "All";
    tab_pending = "Ausstehend" | "Pending";
    tab_add = "Hinzufügen" | "Add";
    list_empty = "Noch keine Freunde. Schick eine Anfrage per Freundescode oder über das Profil von jemandem."
        | "No friends yet. Send a request with a friend code or from someone’s profile.";
    list_load_failed = "Die Freundesliste konnte nicht geladen werden."
        | "Couldn’t load your friends.";
    status_online = "Online" | "Online";
    status_idle = "Abwesend" | "Away";
    pending_empty = "Keine offenen Anfragen." | "No open requests.";
    pending_incoming = "Eingegangen" | "Received";
    pending_outgoing = "Gesendet" | "Sent";
    action_message = "Nachricht" | "Message";
    action_remove = "Entfernen" | "Remove";
    action_remove_hint = "Eure Unterhaltung bleibt erhalten. Ihr könnt euch weiter schreiben."
        | "Your conversation stays. You can keep writing to each other.";
    action_accept = "Annehmen" | "Accept";
    action_decline = "Ablehnen" | "Decline";
    action_withdraw = "Zurückziehen" | "Withdraw";
    action_failed = "Das hat nicht geklappt." | "That didn’t work.";
    add_title = "Freund hinzufügen" | "Add a friend";
    add_hint = "Gib den Freundescode einer Person ein, etwa K7QX-M2PD. Sie sieht deine Anfrage unter „Freunde“."
        | "Enter someone’s friend code, like K7QX-M2PD. They see your request under “Friends”.";
    add_label = "Freundescode" | "Friend code";
    add_submit = "Anfrage senden" | "Send request";
    send_failed = "Die Anfrage wurde nicht gesendet." | "The request wasn’t sent.";
    code_title = "Dein Freundescode" | "Your friend code";
    code_hint = "Wer diesen Code hat, kann dir eine Freundschaftsanfrage schicken."
        | "Anyone with this code can send you a friend request.";
    code_copy = "Kopieren" | "Copy";
    code_copied = "Freundescode kopiert." | "Friend code copied.";
    code_regenerate = "Neuen Code erstellen" | "Create a new code";
    code_regenerate_confirm = "Neuen Freundescode erstellen?" | "Create a new friend code?";
    code_regenerate_hint = "Der alte Code funktioniert danach nicht mehr. Schon gesendete Anfragen bleiben."
        | "The old code stops working. Requests already sent stay.";
    code_regenerate_failed = "Kein neuer Code erstellt." | "No new code created.";
    code_loading = "Code wird geladen …" | "Loading code …";
    card_add = "Freund hinzufügen" | "Add friend";
    card_pending = "Anfrage gesendet" | "Request sent";
    card_incoming = "Anfrage annehmen" | "Accept request";
    card_friends = "Befreundet" | "Friends";
    requests_title = "Nachrichtenanfragen" | "Message requests";
    requests_hint = "Leute von deinen Servern, die dir schreiben möchten. Erst wenn du annimmst, landen sie bei deinen Nachrichten."
        | "People from your servers who want to message you. They join your messages only once you accept.";
    requests_empty = "Keine Nachrichtenanfragen." | "No message requests.";
    requests_load_failed = "Die Anfragen konnten nicht geladen werden."
        | "Couldn’t load the requests.";
    requests_open = "Ansehen" | "View";
    new_message_friend = "Freund" | "Friend";
}

/// "{name} aus deinen Freunden entfernen?"
pub fn action_remove_confirm(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} aus deinen Freunden entfernen?"),
        Locale::En => format!("Remove {name} from your friends?"),
    }
}

/// "Anfrage an {name} gesendet."
pub fn add_sent(name: &str) -> String {
    match get() {
        Locale::De => format!("Anfrage an {name} gesendet."),
        Locale::En => format!("Request sent to {name}."),
    }
}

/// "Du und {name} seid jetzt befreundet."
pub fn add_friends(name: &str) -> String {
    match get() {
        Locale::De => format!("Du und {name} seid jetzt befreundet."),
        Locale::En => format!("You and {name} are friends now."),
    }
}

/// The bar instead of the composer while someone's request waits.
pub fn bar_incoming(name: &str) -> String {
    match get() {
        Locale::De => format!(
            "{name} möchte dir schreiben. Wenn du annimmst, könnt ihr euch frei schreiben. Befreundet seid ihr dadurch nicht."
        ),
        Locale::En => format!(
            "{name} wants to message you. If you accept, you can write freely. It doesn’t make you friends."
        ),
    }
}

/// The note above the composer while the user's own request waits.
pub fn bar_outgoing(name: &str, count: i64) -> String {
    match (get(), count == 1) {
        (Locale::De, true) => format!(
            "Nachrichtenanfrage: Bis {name} annimmt, geht noch {count} Nachricht, nur Text ohne Links."
        ),
        (Locale::De, false) => format!(
            "Nachrichtenanfrage: Bis {name} annimmt, gehen noch {count} Nachrichten, nur Text ohne Links."
        ),
        (Locale::En, true) => format!(
            "Message request: until {name} accepts, {count} more message can go, text only without links."
        ),
        (Locale::En, false) => format!(
            "Message request: until {name} accepts, {count} more messages can go, text only without links."
        ),
    }
}

/// The note once the user's request has used up its messages.
pub fn bar_outgoing_full(name: &str) -> String {
    match get() {
        Locale::De => {
            format!(
                "Nachrichtenanfrage: Warte, bis {name} annimmt. Mehr Nachrichten gehen erst danach."
            )
        }
        Locale::En => {
            format!("Message request: wait until {name} accepts. More messages can go after that.")
        }
    }
}

/// "Nachrichtenanfrage · {servers}" in the new-message dialog.
pub fn new_message_request(servers: &str) -> String {
    match get() {
        Locale::De => format!("Nachrichtenanfrage · {servers}"),
        Locale::En => format!("Message request · {servers}"),
    }
}
