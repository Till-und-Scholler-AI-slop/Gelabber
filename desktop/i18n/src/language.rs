//! The language itself: the "Sprache" section of Einstellungen →
//! Darstellung (web/src/i18n/catalogs/language.ts). The web's splash and
//! offline texts belong to pages the native app does not have.

crate::strings! {
    section_title = "Sprache" | "Language";
    /// Each language is named in its own language on its option.
    option_de = "Deutsch" | "Deutsch";
    option_en = "English" | "English";
    /// The language's name in the interface's language, under its option.
    name_de = "Deutsch" | "German";
    name_en = "Englisch" | "English";
    hint = "Gilt auf allen deinen Geräten, auch für Benachrichtigungen und E-Mails von Gelabber. Nachrichten anderer werden nicht übersetzt."
        | "Applies on all your devices, including notifications and emails from Gelabber. Other people’s messages are not translated.";
}

/// The option's label: the language named in itself.
pub fn option(locale: crate::Locale) -> &'static str {
    match locale {
        crate::Locale::De => option_de(),
        crate::Locale::En => option_en(),
    }
}

/// The language's name in the interface's language.
pub fn name(locale: crate::Locale) -> &'static str {
    match locale {
        crate::Locale::De => name_de(),
        crate::Locale::En => name_en(),
    }
}
