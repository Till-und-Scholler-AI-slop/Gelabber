//! Members (web/src/i18n/catalogs/members.ts): the member list, profile
//! dialogs, and kick and ban. Where a row says where someone sits, the
//! wording is the web's voice `stateIcons.*`.

use crate::{Locale, get};

crate::strings! {
    panel_title = "Mitglieder" | "Members";
    row_you = "Du" | "You";
    row_live = "Live" | "Live";
    row_a_voice_channel = "einem Sprachkanal" | "a voice channel";
    row_in_voice = "Im Sprachkanal" | "In a voice channel";

    card_away = "Abwesend" | "Away";

    profile_own = "Dein Profil" | "Your profile";
    profile_server = "Server" | "Server";
    profile_membership = "Mitgliedschaft" | "Membership";
    profile_owner = "Inhaber" | "Owner";
    profile_member = "Mitglied" | "Member";
    profile_since = "Dabei seit" | "Member since";
    profile_edit = "Profil bearbeiten" | "Edit profile";
    profile_message = "Nachricht senden" | "Send message";
    profile_opening = "Unterhaltung wird geöffnet…" | "Opening conversation…";

    moderation_title = "Moderation" | "Moderation";
    moderation_kick = "Vom Server entfernen" | "Remove from server";
    moderation_ban = "Sperren" | "Ban";
    moderation_kick_message = "Mit einem neuen Einladungslink ist ein erneuter Beitritt möglich."
        | "They can rejoin with a new invite link.";
    moderation_ban_message = "Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink."
        | "They can’t rejoin, not even with an invite link.";
}

/// A member row's accessible name and tooltip.
pub fn row_profile_label(name: &str) -> String {
    match get() {
        Locale::De => format!("Profil von {name}"),
        Locale::En => format!("{name}’s profile"),
    }
}

/// "Live in {channel}".
pub fn row_live_in(channel: &str) -> String {
    match get() {
        Locale::De => format!("Live in {channel}"),
        Locale::En => format!("Live in {channel}"),
    }
}

/// The profile dialog's title for someone else.
pub fn profile_title(name: &str) -> String {
    match get() {
        Locale::De => format!("Profil von {name}"),
        Locale::En => format!("{name}’s profile"),
    }
}

/// The voice channel the person sits in.
pub fn profile_in_room(channel: &str) -> String {
    match get() {
        Locale::De => format!("In {channel}"),
        Locale::En => format!("In {channel}"),
    }
}

/// "{name} vom Server entfernen?"
pub fn moderation_kick_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} vom Server entfernen?"),
        Locale::En => format!("Remove {name} from the server?"),
    }
}

/// "{name} sperren?"
pub fn moderation_ban_title(name: &str) -> String {
    match get() {
        Locale::De => format!("{name} sperren?"),
        Locale::En => format!("Ban {name}?"),
    }
}

/// A row's voice state (web voice `stateIcons.deafenedIn`).
pub fn row_deafened_in(channel: &str) -> String {
    match get() {
        Locale::De => format!("Taub in {channel}"),
        Locale::En => format!("Deafened in {channel}"),
    }
}

/// web voice `stateIcons.mutedIn`.
pub fn row_muted_in(channel: &str) -> String {
    match get() {
        Locale::De => format!("Stumm in {channel}"),
        Locale::En => format!("Muted in {channel}"),
    }
}

/// web voice `stateIcons.inChannel`.
pub fn row_in_channel(channel: &str) -> String {
    match get() {
        Locale::De => format!("In {channel}"),
        Locale::En => format!("In {channel}"),
    }
}
