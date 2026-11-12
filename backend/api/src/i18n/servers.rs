//! A new server's first categories and channels, in its creator's language.
//! After creation they are ordinary data: renaming is up to the server, and
//! nobody else's language changes them.

use super::Locale;

/// Names for `servers::create_server`: a text category with its first
/// channel and a voice category with its first room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultChannels {
    pub text_category: &'static str,
    pub text_channel: &'static str,
    pub voice_category: &'static str,
    pub voice_channel: &'static str,
}

pub fn default_channels(locale: Locale) -> DefaultChannels {
    match locale {
        Locale::De => DefaultChannels {
            text_category: "Textkanäle",
            text_channel: "allgemein",
            voice_category: "Sprachkanäle",
            voice_channel: "Wohnzimmer",
        },
        Locale::En => DefaultChannels {
            text_category: "Text channels",
            text_channel: "general",
            voice_category: "Voice channels",
            voice_channel: "Living room",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages() {
        let de = default_channels(Locale::De);
        assert_eq!(
            [
                de.text_category,
                de.text_channel,
                de.voice_category,
                de.voice_channel
            ],
            ["Textkanäle", "allgemein", "Sprachkanäle", "Wohnzimmer"]
        );
        let en = default_channels(Locale::En);
        assert_eq!(
            [
                en.text_category,
                en.text_channel,
                en.voice_category,
                en.voice_channel
            ],
            ["Text channels", "general", "Voice channels", "Living room"]
        );
    }
}
