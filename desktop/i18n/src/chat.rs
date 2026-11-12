//! The conversation (web/src/i18n/catalogs/chat.ts): message list, rows,
//! composer, attachments, link previews, reactions, GIFs, search, message
//! toasts and the texts of message notifications. Batch N3 also keeps here
//! what the chat files and the shared widgets need from other web
//! namespaces (the header's bell and member toggle, the password field,
//! the discard question), with the web's wording.

use crate::{Locale, get, plural};

crate::strings! {
    // Message list and its header.
    pane_search_hint = "Wörter oder eine Phrase in Anführungszeichen" | "Words, or a phrase in quotes";
    pane_search_placeholder = "Suchen" | "Search";
    pane_search_label = "In dieser Unterhaltung suchen" | "Search this conversation";
    pane_read_not_saved = "Lesestatus nicht gespeichert." | "Read status not saved.";
    pane_jump = "Springen" | "Jump";
    pane_mark_read = "Als gelesen markieren" | "Mark as read";
    pane_back_to_chat = "Zurück zum Chat" | "Back to chat";
    pane_to_latest = "Zu den neuesten Nachrichten" | "Jump to the latest messages";
    list_label = "Nachrichten" | "Messages";
    list_failed = "Nachrichten konnten nicht geladen werden." | "Couldn’t load messages.";
    list_older_failed = "Ältere Nachrichten konnten nicht geladen werden." | "Couldn’t load older messages.";
    list_reload = "Erneut laden" | "Reload";
    list_loading_older = "Ältere Nachrichten…" | "Older messages…";
    list_new_divider_label = "Neue Nachrichten" | "New messages";
    list_new_divider = "Neu" | "New";
    list_empty = "Noch keine Nachrichten. Schreib die erste." | "No messages yet. Write the first one.";
    list_start_of_dm = "Anfang der Unterhaltung." | "Start of the conversation.";
    list_start_of_channel = "Anfang des Kanals." | "Start of the channel.";
    day_today = "Heute" | "Today";
    day_yesterday = "Gestern" | "Yesterday";
    delete_title_own = "Nachricht löschen?" | "Delete message?";
    delete_message = "Das lässt sich nicht rückgängig machen." | "This can’t be undone.";
    delete_failed = "Nachricht nicht gelöscht." | "Message not deleted.";
    unread_messages = "Ungelesene Nachrichten" | "Unread messages";
    unread_notifications_many = "Mehr als 99 ungelesene Benachrichtigungen" | "More than 99 unread notifications";

    // A message row.
    row_reply = "Antworten" | "Reply";
    row_edit_label = "Nachricht bearbeiten" | "Edit message";
    row_more_label = "Weitere Aktionen" | "More actions";
    row_edited = "(bearbeitet)" | "(edited)";
    row_copied = "Text kopiert." | "Text copied.";
    row_copy_failed = "Der Text konnte nicht kopiert werden." | "Couldn’t copy the text.";
    row_edit_hint = "Esc bricht ab · Enter speichert" | "Esc cancels · Enter saves";
    row_edit_failed = "Änderung nicht gespeichert." | "Change not saved.";
    row_sending = "Wird gesendet…" | "Sending…";
    row_not_sent = "Nicht gesendet" | "Not sent";
    row_unclear = "Status unklar – Verlauf prüfen" | "Status unclear – check the history";
    row_uncertain_hint = "Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest."
        | "The message may already be saved. Check the history before you send it again.";
    row_resend = "Erneut senden" | "Send again";
    row_edit = "Bearbeiten" | "Edit";
    row_discard = "Verwerfen" | "Discard";
    row_reply_deleted = "Die ursprüngliche Nachricht wurde gelöscht." | "The original message was deleted.";
    row_reply_attachment = "Anhang" | "Attachment";

    // The message menu (right click, "Mehr", long press).
    menu_react = "Reagieren" | "React";
    menu_other_reaction = "Andere Reaktion auswählen" | "Choose another reaction";
    menu_copy_text = "Text kopieren" | "Copy text";
    menu_delete_hint = "Löschen (Umschalt: ohne Nachfrage)" | "Delete (Shift: without asking)";

    // Composer.
    composer_read_only = "Du kannst in diesem Kanal nicht schreiben." | "You can’t write in this channel.";
    composer_show_unsent = "Anzeigen" | "Show";
    /// The reply strip: `{name}` goes into the `<b>` piece.
    composer_reply_to = "Antwort an <b>{name}</b>" | "Replying to <b>{name}</b>";
    composer_reply_hint = "· Esc bricht ab" | "· Esc cancels";
    composer_cancel_reply_label = "Antworten abbrechen" | "Cancel reply";
    composer_drop_hint = "Loslassen zum Anhängen" | "Drop to attach";
    composer_attachments_label = "Anhänge" | "Attachments";
    composer_remove_file = "Anhang entfernen" | "Remove attachment";
    composer_attach_file = "Datei anhängen" | "Attach file";
    composer_insert_emoji = "Emoji einfügen" | "Insert emoji";
    composer_send_gif = "GIF senden" | "Send GIF";
    composer_send = "Senden" | "Send";

    // Sending.
    send_running = "Dieser Sendeversuch läuft bereits oder ist nicht mehr vorhanden."
        | "This send is already running or no longer exists.";
    send_uncertain = "Speicherstatus unbekannt. Bitte zuerst den Nachrichtenverlauf prüfen."
        | "Unclear whether it was saved. Check the message history first.";
    send_wrong_channel = "Falscher Kanal für den Sendeversuch." | "Wrong channel for this send.";
    send_presign_failed = "Dateifreigabe fehlgeschlagen." | "Preparing the upload failed.";
    send_upload_failed = "Dateiübertragung fehlgeschlagen." | "File upload failed.";
    send_bind_failed = "Nachricht speichern fehlgeschlagen." | "Saving the message failed.";
    upload_timed_out = "Der Upload hat zu lange gedauert und wurde abgebrochen."
        | "The upload took too long and was cancelled.";
    upload_aborted = "Der Upload wurde abgebrochen." | "The upload was cancelled.";
    upload_failed = "Der Upload ist fehlgeschlagen." | "The upload failed.";

    // Attachments and the picture viewer.
    attachment_download = "Herunterladen" | "Download";
    attachment_download_started = "Download gestartet" | "Download started";
    attachment_uploading = "Wird hochgeladen…" | "Uploading…";
    attachment_waiting = "Wartet" | "Waiting";
    attachment_image_failed = "Bild konnte nicht geladen werden." | "Couldn’t load the image.";
    attachment_reload = "Erneut laden" | "Reload";
    attachment_preview_failed = "Vorschau nicht verfügbar." | "Preview not available.";
    attachment_preview_loading = "Vorschau wird geladen…" | "Loading preview…";
    /// The desktop app's save dialog for a downloaded file.
    attachment_save_title = "Datei speichern" | "Save file";
    /// The desktop app decodes pictures itself and leaves out huge ones.
    attachment_image_too_large = "Bild ist zu groß zum Anzeigen." | "The image is too large to show.";

    // Link previews.
    preview_enlarge = "Bild vergrößern" | "Enlarge image";
    preview_remove = "Vorschauen entfernen" | "Remove previews";
    preview_youtube_title = "YouTube-Video" | "YouTube video";
    preview_play_label = "Video abspielen" | "Play video";

    // Message text.
    content_show_spoiler_label = "Spoiler anzeigen" | "Show spoiler";
    content_copy_failed = "Kopieren nicht möglich" | "Can’t copy";

    // Reactions.
    reactions_label = "Reaktionen" | "Reactions";
    reactions_add = "Reaktion hinzufügen" | "Add reaction";
    reactions_pick = "Reaktion auswählen" | "Choose a reaction";
    reactions_you = "du" | "you";
    reactions_not_saved = "Reaktion nicht gespeichert." | "Reaction not saved.";
    reactions_gone = "Die Nachricht gibt es nicht mehr." | "That message no longer exists.";

    // Mentions and typing.
    mention_list_label = "Erwähnen" | "Mention";
    mention_everyone = "Alle Mitglieder" | "All members";
    mention_here = "Alle, die gerade online sind" | "Everyone online right now";
    mention_here_reached = "Alle, die gerade online waren" | "Everyone who was online then";
    mention_unknown = "Unbekannt" | "Unknown";
    typing_someone = "Jemand" | "Someone";

    // GIFs.
    gif_search = "GIF suchen" | "Search GIFs";
    gif_grid_label = "GIF-Auswahl" | "GIF picker";
    gif_loading_more = "Lädt …" | "Loading…";
    gif_more = "Weitere GIFs anzeigen" | "Show more GIFs";
    gif_loading = "GIFs werden geladen …" | "Loading GIFs…";
    gif_failed = "GIFs konnten nicht geladen werden." | "Couldn’t load GIFs.";
    gif_more_failed = "Weitere GIFs konnten nicht geladen werden." | "Couldn’t load more GIFs.";
    gif_none = "Keine GIFs gefunden." | "No GIFs found.";

    // Search in a conversation, and a found message in its context.
    search_label = "Nachrichten suchen" | "Search messages";
    search_summary = "Treffer in dieser Unterhaltung" | "Results in this conversation";
    search_running = "Suche läuft…" | "Searching…";
    search_failed = "Die Suche konnte nicht geladen werden." | "Couldn’t load the search.";
    search_none = "Keine Nachrichten gefunden." | "No messages found.";
    search_open_in_context = "Nachricht im Kontext öffnen" | "Open message in context";
    search_loading_more = "Lädt…" | "Loading…";
    search_older = "Ältere Treffer" | "Older results";
    context_label = "Nachrichtenkontext" | "Message context";
    context_back_to_results = "Zurück zu den Treffern" | "Back to results";
    context_loading = "Nachrichtenkontext wird geladen…" | "Loading message context…";
    context_missing = "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr."
        | "This message is no longer available, or you no longer have access to it.";
    context_failed = "Der Nachrichtenkontext konnte nicht geladen werden." | "Couldn’t load the message context.";

    // Message toasts and notifications.
    toast_mentioned_you = "hat dich erwähnt" | "mentioned you";
    /// `{voice}` goes after the `<b>` piece.
    toast_live = "<b>Live</b> in {voice}" | "<b>Live</b> in {voice}";
    toast_direct_message = "Direktnachricht" | "Direct message";
    toast_channel = "Kanal" | "Channel";
    notify_new_message = "Neue Nachricht" | "New message";
    notify_file = "Datei" | "File";

    // The conversation header (web notifications `bell.*`, members
    // `toggle.*`, shell `keys.ctrl`).
    header_bell_title = "Benachrichtigungen" | "Notifications";
    header_bell_title_muted = "Stummgeschaltet" | "Muted";
    header_members_show = "Mitgliederliste einblenden" | "Show member list";
    header_members_hide = "Mitgliederliste ausblenden" | "Hide member list";
    /// The search field's shortcut, as the keys are labelled.
    header_search_shortcut = "Strg F" | "Ctrl F";

    // A page that could not load (web servers `load.channel`, dms
    // `conversation.loadFailed`).
    load_channel = "Dieser Kanal konnte nicht geladen werden." | "This channel couldn’t be loaded.";
    load_dm = "Diese Unterhaltung konnte nicht geladen werden." | "Couldn’t load this conversation.";

    // The invite link's copy line (web servers `invite.copyFailed`).
    link_copy_failed = "Kopieren fehlgeschlagen. Kopiere den Link manuell." | "Couldn’t copy. Copy the link by hand.";

    // Shared widgets (web shell `password.*`, `discard.*`, `presence.i`).
    password_show = "Passwort anzeigen" | "Show password";
    password_hide = "Passwort verbergen" | "Hide password";
    password_caps_lock = "Feststelltaste ist an" | "Caps Lock is on";
    discard_title = "Änderungen verwerfen?" | "Discard changes?";
    discard_message = "Deine Eingaben gehen verloren." | "What you entered will be lost.";
    discard_confirm = "Verwerfen" | "Discard";
    presence_away = "Abwesend" | "Away";
}

/// "seit {when}", after the unread bar's count.
pub fn pane_unread_since(when: &str) -> String {
    match get() {
        Locale::De => format!("seit {when}"),
        Locale::En => format!("since {when}"),
    }
}

/// "{n} neue Nachricht(en)": `n` is the count as shown ("99+" past 99),
/// the form follows `count`.
pub fn pane_new_messages(count: u64, n: &str) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{n} neue Nachricht"),
            format!("{n} neue Nachrichten"),
        ),
        Locale::En => plural(
            count,
            format!("{n} new message"),
            format!("{n} new messages"),
        ),
    }
}

/// "Nachricht von {name} löschen?".
pub fn delete_title_other(name: &str) -> String {
    match get() {
        Locale::De => format!("Nachricht von {name} löschen?"),
        Locale::En => format!("Delete message from {name}?"),
    }
}

/// "{count} ungelesene Benachrichtigung(en)".
pub fn unread_notifications(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{count} ungelesene Benachrichtigung"),
            format!("{count} ungelesene Benachrichtigungen"),
        ),
        Locale::En => plural(
            count,
            format!("{count} unread notification"),
            format!("{count} unread notifications"),
        ),
    }
}

/// "Bearbeitet am {when}".
pub fn row_edited_at(when: &str) -> String {
    match get() {
        Locale::De => format!("Bearbeitet am {when}"),
        Locale::En => format!("Edited on {when}"),
    }
}

/// A reply's accessible name: "Antwort auf {name}: {summary}. …".
pub fn row_reply_label(name: &str, summary: &str) -> String {
    match get() {
        Locale::De => format!("Antwort auf {name}: {summary}. Zur Nachricht springen"),
        Locale::En => format!("Reply to {name}: {summary}. Jump to message"),
    }
}

/// The `live_started` system message.
pub fn system_live_started(author: &str, voice: &str) -> String {
    match get() {
        Locale::De => format!("{author} ist live in {voice}."),
        Locale::En => format!("{author} is live in {voice}."),
    }
}

/// A system message of a kind this app does not know.
pub fn system_unknown(author: &str) -> String {
    match get() {
        Locale::De => format!("Systemnachricht von {author}"),
        Locale::En => format!("System message from {author}"),
    }
}

/// The message menu's accessible name.
pub fn menu_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Nachricht von {name}"),
        Locale::En => format!("Message from {name}"),
    }
}

/// "Mit {emoji} reagieren".
pub fn menu_react_with(emoji: &str) -> String {
    match get() {
        Locale::De => format!("Mit {emoji} reagieren"),
        Locale::En => format!("React with {emoji}"),
    }
}

/// "Reaktion {emoji} entfernen".
pub fn menu_remove_reaction(emoji: &str) -> String {
    match get() {
        Locale::De => format!("Reaktion {emoji} entfernen"),
        Locale::En => format!("Remove reaction {emoji}"),
    }
}

/// "{count} Nachricht(en) nicht gesendet".
pub fn composer_unsent(count: u64) -> String {
    match get() {
        Locale::De => plural(
            count,
            format!("{count} Nachricht nicht gesendet"),
            format!("{count} Nachrichten nicht gesendet"),
        ),
        Locale::En => plural(
            count,
            format!("{count} message not sent"),
            format!("{count} messages not sent"),
        ),
    }
}

/// The composer's placeholder; `target` is "#channel" or "@person".
pub fn composer_message_to(target: &str) -> String {
    match get() {
        Locale::De => format!("Nachricht an {target}"),
        Locale::En => format!("Message {target}"),
    }
}

/// "{name} entfernen", a picked file's remove button.
pub fn composer_remove_file_label(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} entfernen"),
        Locale::En => format!("Remove {name}"),
    }
}

/// "{count} Zeichen übrig".
pub fn composer_chars_left(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} Zeichen übrig"),
        Locale::En => plural(
            count,
            format!("{n} character left"),
            format!("{n} characters left"),
        ),
    }
}

/// "{count} Zeichen zu viel".
pub fn composer_chars_over(count: u64) -> String {
    let n = crate::intl::count(count);
    match get() {
        Locale::De => format!("{n} Zeichen zu viel"),
        Locale::En => plural(
            count,
            format!("{n} character too many"),
            format!("{n} characters too many"),
        ),
    }
}

/// Files left out, with the reason.
pub fn composer_not_attached(names: &str, reason: &str) -> String {
    match get() {
        Locale::De => format!("Nicht angehängt: {names}. {reason}"),
        Locale::En => format!("Not attached: {names}. {reason}"),
    }
}

/// A pasted picture's name without the extension: "Bild-{stamp}".
pub fn composer_pasted_image_name(stamp: &str) -> String {
    match get() {
        Locale::De => format!("Bild-{stamp}"),
        Locale::En => format!("Image-{stamp}"),
    }
}

/// "{name} vergrößern", a picture's accessible name.
pub fn attachment_enlarge_label(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} vergrößern"),
        Locale::En => format!("Enlarge {name}"),
    }
}

/// A failed download's toast: "Datei nicht heruntergeladen. {reason}".
pub fn attachment_download_failed(reason: &str) -> String {
    match get() {
        Locale::De => format!("Datei nicht heruntergeladen. {reason}"),
        Locale::En => format!("File not downloaded. {reason}"),
    }
}

/// A GIF with a title: "GIF: {title}".
pub fn attachment_gif_title(title: &str) -> String {
    match get() {
        Locale::De => format!("GIF: {title}"),
        Locale::En => format!("GIF: {title}"),
    }
}

/// "Vorschau: {name}".
pub fn preview_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Vorschau: {name}"),
        Locale::En => format!("Preview: {name}"),
    }
}

/// "Bild vergrößern: {name}".
pub fn preview_enlarge_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Bild vergrößern: {name}"),
        Locale::En => format!("Enlarge image: {name}"),
    }
}

/// "Video abspielen: {title}".
pub fn preview_play_titled_label(title: &str) -> String {
    match get() {
        Locale::De => format!("Video abspielen: {title}"),
        Locale::En => format!("Play video: {title}"),
    }
}

/// A reaction chip's accessible name; `mine`: I reacted too.
pub fn reactions_chip_label(emoji: &str, count: u64, mine: bool) -> String {
    match (get(), mine) {
        (Locale::De, false) => plural(
            count,
            format!("{emoji}: {count} Reaktion"),
            format!("{emoji}: {count} Reaktionen"),
        ),
        (Locale::De, true) => plural(
            count,
            format!("{emoji}: {count} Reaktion, du hast reagiert"),
            format!("{emoji}: {count} Reaktionen, du hast reagiert"),
        ),
        (Locale::En, false) => plural(
            count,
            format!("{emoji}: {count} reaction"),
            format!("{emoji}: {count} reactions"),
        ),
        (Locale::En, true) => plural(
            count,
            format!("{emoji}: {count} reaction, you reacted"),
            format!("{emoji}: {count} reactions, you reacted"),
        ),
    }
}

/// "{names} und {last}".
pub fn reactions_names_and(names: &str, last: &str) -> String {
    match get() {
        Locale::De => format!("{names} und {last}"),
        Locale::En => format!("{names} and {last}"),
    }
}

/// "{names} und {rest} weitere".
pub fn reactions_names_and_more(names: &str, rest: u64) -> String {
    match get() {
        Locale::De => format!("{names} und {rest} weitere"),
        Locale::En => format!("{names} and {rest} more"),
    }
}

/// "{label}, Animation", a GIF's accessible name.
pub fn gif_animation_label(label: &str) -> String {
    match get() {
        Locale::De => format!("{label}, Animation"),
        Locale::En => format!("{label}, animation"),
    }
}

/// Too many GIF requests, with the wait in seconds.
pub fn gif_rate_limited(seconds: u64) -> String {
    match get() {
        Locale::De => {
            format!("Zu viele GIF-Anfragen. Bitte in {seconds} s noch einmal versuchen.")
        }
        Locale::En => format!("Too many GIF requests. Try again in {seconds} s."),
    }
}

/// The server's GIF quota is used up; `wait` is [`gif_wait_seconds`] or
/// [`gif_wait_minutes`].
pub fn gif_quota(wait: &str) -> String {
    match get() {
        Locale::De => format!(
            "Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte in {wait} noch einmal versuchen."
        ),
        Locale::En => {
            format!("This server has used up its GIF quota for now. Try again in {wait}.")
        }
    }
}

/// "{seconds} s".
pub fn gif_wait_seconds(seconds: u64) -> String {
    format!("{seconds} s")
}

/// "{minutes} Min.".
pub fn gif_wait_minutes(minutes: u64) -> String {
    match get() {
        Locale::De => format!("{minutes} Min."),
        Locale::En => format!("{minutes} min"),
    }
}

/// "{count} GIFs zu „{query}“".
pub fn gif_found(count: u64, query: &str) -> String {
    match get() {
        Locale::De => format!("{count} GIFs zu „{query}“"),
        Locale::En => plural(
            count,
            format!("{count} GIF for “{query}”"),
            format!("{count} GIFs for “{query}”"),
        ),
    }
}

/// "{count} angesagte GIFs".
pub fn gif_trending(count: u64) -> String {
    match get() {
        Locale::De => format!("{count} angesagte GIFs"),
        Locale::En => plural(
            count,
            format!("{count} trending GIF"),
            format!("{count} trending GIFs"),
        ),
    }
}

/// A search hit's accessible name.
pub fn search_hit_label(name: &str, text: &str) -> String {
    match get() {
        Locale::De => format!("Zur Nachricht von {name}: {text}"),
        Locale::En => format!("Go to the message from {name}: {text}"),
    }
}

/// "Gefundene Nachricht von {name}".
pub fn context_found_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Gefundene Nachricht von {name}"),
        Locale::En => format!("Found message from {name}"),
    }
}

/// "{where} · {count} neu".
pub fn toast_new_count(place: &str, count: u64) -> String {
    match get() {
        Locale::De => format!("{place} · {count} neu"),
        Locale::En => format!("{place} · {count} new"),
    }
}

/// "{author} hat dich erwähnt · {label}".
pub fn notify_mentioned_title(author: &str, label: &str) -> String {
    match get() {
        Locale::De => format!("{author} hat dich erwähnt · {label}"),
        Locale::En => format!("{author} mentioned you · {label}"),
    }
}

/// "Neue Nachricht von {author}".
pub fn notify_new_message_from(author: &str) -> String {
    match get() {
        Locale::De => format!("Neue Nachricht von {author}"),
        Locale::En => format!("New message from {author}"),
    }
}

/// A timestamp's tooltip: the full date "um" / "at" the time
/// (`dateStyle: "full", timeStyle: "short"`).
pub fn time_full(date: &str, time: &str) -> String {
    match get() {
        Locale::De => format!("{date} um {time}"),
        Locale::En => format!("{date} at {time}"),
    }
}

/// The conversation bell's accessible name; `name` is "#channel" or the
/// person.
pub fn header_bell_label(name: &str, muted: bool) -> String {
    match (get(), muted) {
        (Locale::De, false) => format!("Benachrichtigungen für {name}"),
        (Locale::De, true) => format!("Benachrichtigungen für {name} (stumm)"),
        (Locale::En, false) => format!("Notifications for {name}"),
        (Locale::En, true) => format!("Notifications for {name} (muted)"),
    }
}

/// The person's avatar in a DM's header: "Profil von {name}".
pub fn header_profile_of(name: &str) -> String {
    match get() {
        Locale::De => format!("Profil von {name}"),
        Locale::En => format!("{name}’s profile"),
    }
}

/// The invite link went to the clipboard: "Link kopiert · {terms}".
pub fn link_copied(terms: &str) -> String {
    match get() {
        Locale::De => format!("Link kopiert · {terms}"),
        Locale::En => format!("Link copied · {terms}"),
    }
}
