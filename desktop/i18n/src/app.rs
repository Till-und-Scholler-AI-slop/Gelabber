//! The app around the screens: the frame's shared texts (skip link,
//! session recovery, sign-out strip), the server setup, updates, the tray,
//! system notifications, files, push-to-talk and the single instance.
//! Texts the web has too keep its wording (shell, chat, themes); the
//! native-only ones are written in the same tone.

use crate::{Locale, get, plural};

crate::strings! {
    // The frame (web shell).
    skip_link = "Zum Inhalt springen" | "Skip to content";
    recovery_no_answer = "Server antwortet nicht" | "Server is not answering";
    recovery_no_answer_lead = "Gelabber ist nicht erreichbar. Möglicherweise startet der Server neu."
        | "Gelabber can’t be reached. The server may be restarting.";
    recovery_checking = "Wird geprüft…" | "Checking…";
    recovery_retry_now = "Jetzt erneut versuchen" | "Try again now";
    recovery_signing_out = "Wird abgemeldet…" | "Signing out…";
    recovery_sign_out_again = "Erneut abmelden" | "Sign out again";

    // The server setup (native only).
    setup_guidance = "Die Adresse bekommst du von der Person, die den Server betreibt."
        | "You get the address from the person who runs the server.";
    setup_error_refused = "Server antwortet nicht – stimmen Adresse und Port?"
        | "Server is not answering – are the address and port right?";
    setup_error_timed_out = "Keine Antwort vom Server – läuft er und stimmt die Adresse?"
        | "No answer from the server – is it running and is the address right?";
    setup_error_offline = "Kein Netzwerk – ist dein Computer online?"
        | "No network – is your computer online?";
    setup_error_host_unreachable = "Der Server ist von hier aus nicht erreichbar – bist du im richtigen Netz?"
        | "The server can’t be reached from here – are you on the right network?";
    setup_error_unreachable = "Der Server ist nicht erreichbar." | "The server can’t be reached.";
    setup_error_not_gelabber = "Unter dieser Adresse läuft kein Gelabber-Server."
        | "There is no Gelabber server at this address.";
    setup_error_server_trouble = "Der Server hat gerade ein Problem – versuch es gleich noch einmal."
        | "The server has a problem right now – try again in a moment.";
    setup_error_tls = "Keine sichere Verbindung zum Server möglich."
        | "No secure connection to the server is possible.";
    setup_error_tls_certificate = "Keine sichere Verbindung zum Server möglich – sein Zertifikat wird nicht anerkannt."
        | "No secure connection to the server is possible – its certificate isn’t recognised.";
    setup_error_empty = "Gib die Adresse deines Gelabber-Servers ein."
        | "Enter the address of your Gelabber server.";
    setup_error_no_address = "Die Adresse beginnt mit https:// oder http://, zum Beispiel https://gelabber.example.org."
        | "The address starts with https:// or http://, for example https://gelabber.example.org.";
    setup_title_switch = "Server wechseln" | "Switch server";
    setup_title_connect = "Server verbinden" | "Connect a server";
    setup_lead = "Mit welchem Gelabber-Server soll sich die App verbinden?"
        | "Which Gelabber server should the app connect to?";
    setup_address_label = "Serveradresse" | "Server address";
    setup_connect = "Verbinden" | "Connect";
    setup_connecting = "Verbinde…" | "Connecting…";
    setup_updates_title = "Updates" | "Updates";
    setup_install_now = "Jetzt installieren" | "Install now";
    setup_installing = "Wird installiert…" | "Installing…";
    setup_check_updates = "Nach Updates suchen" | "Check for updates";
    setup_checking_updates = "Suche läuft…" | "Checking…";

    // Updates (native only).
    update_in_call = "Während eines Anrufs startet Gelabber nicht neu. Das Update kommt nach dem Auflegen."
        | "Gelabber doesn’t restart during a call. The update comes after you hang up.";
    update_line_checking = " Suche läuft …" | " Checking …";
    update_line_latest = " Das ist die neueste Version." | " This is the latest version.";
    update_installing = "Update wird installiert, Gelabber startet neu …"
        | "Installing the update, Gelabber is restarting …";
    update_restart = "Neu starten" | "Restart";
    update_download = "Herunterladen" | "Download";
    update_later = "Später" | "Later";

    // The tray (native only).
    tray_unread = "Gelabber: ungelesene Nachrichten" | "Gelabber: unread messages";
    tray_open = "Gelabber öffnen" | "Open Gelabber";
    tray_quit = "Beenden" | "Quit";

    // Files (native only).
    file_save_no_name = "Datei nicht gespeichert. Der Dateiname fehlt."
        | "File not saved. The file name is missing.";
    file_no_downloads = "Kein Ordner für Downloads gefunden." | "No folder for downloads found.";
    file_path_label = "Dateipfad" | "File path";
    file_path_placeholder_windows = "C:\\Users\\…\\bild.png" | "C:\\Users\\…\\picture.png";
    file_path_placeholder = "/home/…/bild.png" | "/home/…/picture.png";
    file_open = "Öffnen" | "Open";
    file_cannot_open = "Diese Datei lässt sich nicht öffnen." | "This file can’t be opened.";

    // System notifications (web chat `notify.*`, `toast.*`).
    notify_open = "Öffnen" | "Open";
    notify_new_message = "Neue Nachricht" | "New message";
    notify_file = "Datei" | "File";
    notify_direct_message = "Direktnachricht" | "Direct message";
    notify_channel = "Kanal" | "Channel";
    notify_someone = "Jemand" | "Someone";
    notify_mention_unknown = "Unbekannt" | "Unknown";

    // Push-to-talk through the desktop portal (native only).
    ptt_cancelled = "Abgebrochen." | "Cancelled.";
    ptt_no_portal = "Der Desktop bietet keine globalen Tasten an (GlobalShortcuts-Portal fehlt)."
        | "The desktop offers no global keys (the GlobalShortcuts portal is missing).";
    ptt_no_session = "Das Portal nennt keine Sitzung." | "The portal names no session.";

    // A second start while the first does not answer (native only).
    instance_not_responding = "Gelabber läuft schon, reagiert aber nicht. Beende Gelabber im Task-Manager und starte es dann noch einmal."
        | "Gelabber is already running but not responding. End Gelabber in Task Manager and then start it again.";

    // A theme's contrast problems (web themes `issue.*`).
    theme_issue_text = "Text ist auf mindestens einer Fläche schwer lesbar."
        | "Text is hard to read on at least one surface.";
    theme_issue_muted = "Sekundärtext braucht mehr Kontrast." | "Secondary text needs more contrast.";
    theme_issue_accent_ink = "Schrift auf der Akzentfarbe ist schwer lesbar."
        | "Text on the accent colour is hard to read.";
    theme_issue_accent_text = "Links in der Akzentfarbe heben sich zu wenig ab."
        | "Links in the accent colour don’t stand out enough.";
    theme_issue_border = "Rahmen von Eingabefeldern heben sich zu wenig ab."
        | "Input field borders don’t stand out enough.";
}

/// `submit.cooldown`: a form's submit button while a rate limit lasts.
pub fn submit_cooldown(seconds: u64) -> String {
    match get() {
        Locale::De => format!("Erneut versuchen in {seconds} s"),
        Locale::En => format!("Try again in {seconds} s"),
    }
}

/// `recovery.retryIn`.
pub fn recovery_retry_in(seconds: u64) -> String {
    match get() {
        Locale::De => format!("Neuer Versuch in {seconds} s"),
        Locale::En => format!("Trying again in {seconds} s"),
    }
}

/// The setup's error for a host name that does not resolve.
pub fn setup_error_not_found(host: &str) -> String {
    match get() {
        Locale::De => format!("{host} wurde nicht gefunden – ist die Adresse richtig geschrieben?"),
        Locale::En => format!("{host} wasn’t found – is the address spelt correctly?"),
    }
}

/// The setup's error for a redirect to another origin.
pub fn setup_error_redirect(to: &str) -> String {
    match get() {
        Locale::De => {
            format!(
                "Der Server leitet auf {to} weiter. Gib diese Adresse ein, wenn du ihr vertraust."
            )
        }
        Locale::En => format!("The server redirects to {to}. Enter this address if you trust it."),
    }
}

/// What the system said about a failed setup check.
pub fn setup_details(detail: &str) -> String {
    match get() {
        Locale::De => format!("Details: {detail}"),
        Locale::En => format!("Details: {detail}"),
    }
}

/// The way back to the server in use, after it did not answer.
pub fn setup_reconnect_to(host: &str) -> String {
    match get() {
        Locale::De => format!("Erneut verbinden mit {host}"),
        Locale::En => format!("Reconnect to {host}"),
    }
}

/// The way back to the server in use.
pub fn setup_back_to(host: &str) -> String {
    match get() {
        Locale::De => format!("Zurück zu {host}"),
        Locale::En => format!("Back to {host}"),
    }
}

/// The start of the updates' status line.
pub fn update_line_installed(current: &str) -> String {
    match get() {
        Locale::De => format!("Installiert: {current}."),
        Locale::En => format!("Installed: {current}."),
    }
}

/// The status line's update found, after its start.
pub fn update_line_available(available: &str) -> String {
    match get() {
        Locale::De => format!(" Verfügbar: {available}."),
        Locale::En => format!(" Available: {available}."),
    }
}

/// The status line's failed check, at its end.
pub fn update_line_error(error: &str) -> String {
    match get() {
        Locale::De => format!(" Letzte Suche: {error}"),
        Locale::En => format!(" Last check: {error}"),
    }
}

/// A download over the size cap.
pub fn update_too_large(file: &str) -> String {
    match get() {
        Locale::De => format!("{file}: das Update ist zu groß"),
        Locale::En => format!("{file}: the update is too large"),
    }
}

/// A download whose signature does not check out.
pub fn update_bad_signature(error: &str) -> String {
    match get() {
        Locale::De => format!("Die Signatur des Updates stimmt nicht: {error}"),
        Locale::En => format!("The update’s signature doesn’t match: {error}"),
    }
}

/// The notice where the app installs the update by restarting.
pub fn update_offer_restart(version: &str) -> String {
    match get() {
        Locale::De => format!("Gelabber {version} ist da. Zum Installieren startet die App neu."),
        Locale::En => format!("Gelabber {version} is here. The app restarts to install it."),
    }
}

/// The notice where the update is downloaded from the release page.
pub fn update_offer_download(version: &str) -> String {
    match get() {
        Locale::De => format!("Gelabber {version} ist da."),
        Locale::En => format!("Gelabber {version} is here."),
    }
}

/// The notice where pacman updates the app.
pub fn update_offer_package_manager(version: &str, package: &str) -> String {
    match get() {
        Locale::De => {
            format!("Gelabber {version} ist da. Updates kommen über pacman (Paket {package}).")
        }
        Locale::En => {
            format!("Gelabber {version} is here. Updates come through pacman (package {package}).")
        }
    }
}

/// The notice while the update downloads; `percent` is " 42 %" or empty.
pub fn update_downloading(percent: &str) -> String {
    match get() {
        Locale::De => format!("Update wird geladen …{percent}"),
        Locale::En => format!("Downloading the update …{percent}"),
    }
}

/// The tray's tooltip with `count` new messages (`shell_unread`).
pub fn tray_new_messages(count: u64) -> String {
    match get() {
        Locale::De => format!(
            "Gelabber: {count} {}",
            plural(count, "neue Nachricht", "neue Nachrichten")
        ),
        Locale::En => format!(
            "Gelabber: {count} {}",
            plural(count, "new message", "new messages")
        ),
    }
}

/// A file not saved, and why.
pub fn file_not_saved(reason: &str) -> String {
    match get() {
        Locale::De => format!("Datei nicht gespeichert. {reason}"),
        Locale::En => format!("File not saved. {reason}"),
    }
}

/// A file saved, and where.
pub fn file_saved_to(path: &str) -> String {
    match get() {
        Locale::De => format!("Gespeichert unter {path}."),
        Locale::En => format!("Saved to {path}."),
    }
}

/// `notify.newMessageFrom`: the title when message content is hidden.
pub fn notify_new_message_from(author: &str) -> String {
    match get() {
        Locale::De => format!("Neue Nachricht von {author}"),
        Locale::En => format!("New message from {author}"),
    }
}

/// `notify.mentionedTitle`.
pub fn notify_mentioned_title(author: &str, label: &str) -> String {
    match get() {
        Locale::De => format!("{author} hat dich erwähnt · {label}"),
        Locale::En => format!("{author} mentioned you · {label}"),
    }
}

/// `system.liveStarted`: a Live's system message as a notification.
pub fn notify_live_started(author: &str, voice: &str) -> String {
    match get() {
        Locale::De => format!("{author} ist live in {voice}."),
        Locale::En => format!("{author} is live in {voice}."),
    }
}

/// `system.unknown`: a system message this app does not know.
pub fn notify_system_unknown(author: &str) -> String {
    match get() {
        Locale::De => format!("Systemnachricht von {author}"),
        Locale::En => format!("System message from {author}"),
    }
}

/// The portal refused a request.
pub fn ptt_portal_refuses(method: &str, code: impl std::fmt::Display) -> String {
    match get() {
        Locale::De => format!("Das Portal lehnt ab ({method}, {code})."),
        Locale::En => format!("The portal refuses ({method}, {code})."),
    }
}
