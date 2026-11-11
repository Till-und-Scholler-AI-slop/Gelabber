//! The system's language, for an app that has never shown one: German for
//! a German system, English for any other, nothing where the system names
//! none (the web's `detectLocale` on `navigator.languages`).

use crate::Locale;

/// The system's language: on Windows the user's locale
/// (`GetUserDefaultLocaleName`), elsewhere `LC_ALL`, `LC_MESSAGES`, then
/// `LANG`. `None` when none is set or it is `C` / `POSIX`.
pub fn detect_system() -> Option<Locale> {
    #[cfg(windows)]
    {
        windows_locale()
    }
    #[cfg(not(windows))]
    {
        from_env(|name| std::env::var(name).ok())
    }
}

/// The POSIX rule: the first of `LC_ALL`, `LC_MESSAGES` and `LANG` that is
/// set and not empty names the language of messages.
#[cfg_attr(windows, allow(dead_code))]
fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Locale> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(var)
        .find(|v| !v.is_empty())?;
    from_tag(&value)
}

/// `de_DE.UTF-8`, `de-AT`, `en_GB`, `fr`: German for `de`, else English;
/// `C` and `POSIX` (with or without a codeset) name no language.
fn from_tag(tag: &str) -> Option<Locale> {
    let language = tag
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .split(['_', '-'])
        .next()
        .unwrap_or_default();
    if language.is_empty() || language == "C" || language == "POSIX" {
        return None;
    }
    Some(if language.eq_ignore_ascii_case("de") {
        Locale::De
    } else {
        Locale::En
    })
}

#[cfg(windows)]
fn windows_locale() -> Option<Locale> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    // LOCALE_NAME_MAX_LENGTH.
    let mut name = [0u16; 85];
    // SAFETY: the buffer is as long as the length passed.
    let len = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if len <= 1 {
        return None;
    }
    // The length counts the terminating zero.
    from_tag(&String::from_utf16_lossy(&name[..len as usize - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> Option<Locale> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        from_env(|name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
    }

    #[test]
    fn german_systems_are_german_and_all_others_english() {
        assert_eq!(env(&[("LANG", "de_DE.UTF-8")]), Some(Locale::De));
        assert_eq!(env(&[("LANG", "de_AT")]), Some(Locale::De));
        assert_eq!(env(&[("LANG", "de")]), Some(Locale::De));
        assert_eq!(env(&[("LANG", "en_GB.UTF-8")]), Some(Locale::En));
        assert_eq!(env(&[("LANG", "fr_FR.UTF-8")]), Some(Locale::En));
        assert_eq!(env(&[("LANG", "de_CH.UTF-8@euro")]), Some(Locale::De));
    }

    #[test]
    fn lc_all_wins_over_lc_messages_over_lang() {
        let all = [
            ("LC_ALL", "en_US.UTF-8"),
            ("LC_MESSAGES", "de_DE.UTF-8"),
            ("LANG", "de_DE.UTF-8"),
        ];
        assert_eq!(env(&all), Some(Locale::En));
        assert_eq!(env(&all[1..]), Some(Locale::De));
        assert_eq!(
            env(&[("LC_MESSAGES", "fr_FR"), ("LANG", "de_DE")]),
            Some(Locale::En)
        );
        // Set but empty is not set.
        assert_eq!(
            env(&[("LC_ALL", ""), ("LANG", "de_DE.UTF-8")]),
            Some(Locale::De)
        );
    }

    #[test]
    fn unset_c_and_posix_name_no_language() {
        assert_eq!(env(&[]), None);
        assert_eq!(env(&[("LANG", "C")]), None);
        assert_eq!(env(&[("LANG", "C.UTF-8")]), None);
        assert_eq!(env(&[("LC_ALL", "POSIX")]), None);
        assert_eq!(env(&[("LANG", "")]), None);
        // Windows names: `de-DE`, `en-US`.
        assert_eq!(from_tag("de-DE"), Some(Locale::De));
        assert_eq!(from_tag("en-US"), Some(Locale::En));
    }
}
