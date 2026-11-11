//! The emoji picker (web/src/i18n/catalogs/emoji.ts), for the composer and
//! reactions. The emoji's own names and search words come from CLDR
//! (desktop/ui/assets/emoji/emoji-data.json, and emoji-names.en.json in
//! English), not from here.

use crate::{Locale, get};

crate::strings! {
    picker_search = "Emoji suchen" | "Search emoji";
    picker_group = "Kategorie" | "Category";
    picker_all_groups = "Alle Kategorien" | "All categories";
    picker_grid_label = "Emoji-Auswahl" | "Emoji picker";
    picker_more = "Weitere Emojis anzeigen" | "Show more emoji";
    picker_loading = "Emoji-Auswahl wird geladen…" | "Loading emoji picker…";
    picker_load_failed = "Die Emoji-Auswahl konnte nicht geladen werden." | "Couldn’t load the emoji picker.";
}

/// "{count} Emojis gefunden" (one and other read the same in both).
pub fn picker_found(count: u64) -> String {
    match get() {
        Locale::De => format!("{count} Emojis gefunden"),
        Locale::En => format!("{} emoji found", crate::intl::english_count(count)),
    }
}

/// The licence link: "Unicode {version} / CLDR {cldr} – Lizenz".
pub fn picker_licence(version: &str, cldr: &str) -> String {
    match get() {
        Locale::De => format!("Unicode {version} / CLDR {cldr} – Lizenz"),
        Locale::En => format!("Unicode {version} / CLDR {cldr} – licence"),
    }
}
