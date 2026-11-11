//! The language Gelabber itself writes in, German or English, and the
//! native string catalogs (docs/i18n.md). It is the counterpart of
//! web/src/i18n: one module per namespace with the web's namespace names,
//! keys as snake_case functions.
//!
//! The language belongs to the account (`users.locale`); signed out, the
//! app keeps the one it last showed (the device settings), else the
//! system's. [`set`] switches it for the whole process; egui draws every
//! frame anew, so the next frame shows it.
//!
//! ```
//! gelabber_i18n::strings! {
//!     /// Call bar: leave the voice channel.
//!     call_bar_leave = "Verlassen" | "Leave";
//! }
//! # gelabber_i18n::set(gelabber_i18n::Locale::De);
//! assert_eq!(call_bar_leave(), "Verlassen");
//! ```
//!
//! Text with parameters is a plain function that matches on [`get`], so
//! both languages are checked by the compiler:
//!
//! ```
//! use gelabber_i18n::{Locale, get, plural};
//!
//! pub fn people(count: u64) -> String {
//!     match get() {
//!         Locale::De => format!("{count} {}", plural(count, "Person", "Personen")),
//!         Locale::En => format!("{count} {}", plural(count, "person", "people")),
//!     }
//! }
//! ```

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

pub mod intl;
mod system;

pub mod common;
pub mod language;

pub use system::detect_system;

/// A namespace's static entries: key, German, English.
pub type Entries = &'static [(&'static str, &'static str, &'static str)];

/// Every namespace's static entries, for the catalog tests. A new
/// namespace module adds its line here.
pub const NAMESPACES: &[(&str, Entries)] = &[("common", common::ALL), ("language", language::ALL)];

/// A language of the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Locale {
    /// German, the language of every account before v0.10.
    #[default]
    De,
    /// British English ("Colour scheme").
    En,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::De, Locale::En];

    /// `"de"` or `"en"`, as `users.locale` and the `lc` frame write it.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "de" => Some(Self::De),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    /// The BCP 47 tag the web formats this language with (`intlLocale`).
    pub fn intl_tag(self) -> &'static str {
        match self {
            Self::De => "de-DE",
            Self::En => "en-GB",
        }
    }

    fn from_u8(value: u8) -> Self {
        if value == 1 { Self::En } else { Self::De }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::De => 0,
            Self::En => 1,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD: Cell<Option<Locale>> = const { Cell::new(None) };
}

/// The language the interface shows right now. German until something
/// calls [`set`].
pub fn get() -> Locale {
    THREAD
        .with(Cell::get)
        .unwrap_or_else(|| Locale::from_u8(CURRENT.load(Ordering::Relaxed)))
}

/// Shows the interface in `locale` from the next frame on.
pub fn set(locale: Locale) {
    if THREAD.with(Cell::get).is_some() {
        THREAD.with(|t| t.set(Some(locale)));
    } else {
        CURRENT.store(locale.to_u8(), Ordering::Relaxed);
    }
}

/// Gives the calling thread a language of its own until the guard drops:
/// [`get`] and [`set`] on this thread use it and leave the process's
/// alone. Tests that switch the language run side by side with tests that
/// expect German.
pub fn on_this_thread(locale: Locale) -> ThreadLocale {
    let previous = THREAD.with(|t| t.replace(Some(locale)));
    ThreadLocale { previous }
}

/// See [`on_this_thread`].
#[must_use = "the thread's own language ends when this drops"]
pub struct ThreadLocale {
    previous: Option<Locale>,
}

impl Drop for ThreadLocale {
    fn drop(&mut self) {
        THREAD.with(|t| t.set(self.previous));
    }
}

/// The text in the current language. Both are required, so a missing
/// translation does not compile.
pub fn tr(de: &'static str, en: &'static str) -> &'static str {
    match get() {
        Locale::De => de,
        Locale::En => en,
    }
}

/// `one` for exactly one, else `other`: German and English have only these
/// two forms. A different sentence for none is a key of its own.
pub fn plural<T>(n: u64, one: T, other: T) -> T {
    if n == 1 { one } else { other }
}

/// A run of tagged text: `tag` is `Some("link")` inside `<link>…</link>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub text: &'a str,
    pub tag: Option<&'a str>,
}

/// Splits text with inline tags (`Mehr in der <link>Hilfe</link>.`) into
/// pieces for the drawing code. Tags have no attributes and do not nest;
/// a tag that is not closed is text.
pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut plain_from = 0;
    let mut at = 0;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let tagged = after.find('>').and_then(|end| {
            let name = &after[..end];
            let valid = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric());
            if !valid {
                return None;
            }
            let body = &after[end + 1..];
            let close = format!("</{name}>");
            body.find(&close).map(|len| (name, end, len, close.len()))
        });
        match tagged {
            Some((name, end, len, close_len)) => {
                let start = at + open;
                if start > plain_from {
                    out.push(Piece {
                        text: &text[plain_from..start],
                        tag: None,
                    });
                }
                let body_from = start + 1 + end + 1;
                out.push(Piece {
                    text: &text[body_from..body_from + len],
                    tag: Some(&text[start + 1..start + 1 + name.len()]),
                });
                at = body_from + len + close_len;
                plain_from = at;
            }
            None => at += open + 1,
        }
        rest = &text[at..];
    }
    if plain_from < text.len() {
        out.push(Piece {
            text: &text[plain_from..],
            tag: None,
        });
    }
    out
}

/// One `pub fn key() -> &'static str` per entry, in the current language,
/// plus `pub const ALL: &[(key, de, en)]` for the catalog tests.
#[macro_export]
macro_rules! strings {
    ($( $(#[$meta:meta])* $key:ident = $de:literal | $en:literal; )*) => {
        $(
            $(#[$meta])*
            pub fn $key() -> &'static str {
                $crate::tr($de, $en)
            }
        )*

        /// Every entry of this namespace: key, German, English.
        pub const ALL: &[(&str, &str, &str)] = &[$((stringify!($key), $de, $en)),*];
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_parse_and_print_as_the_server_writes_them() {
        for locale in Locale::ALL {
            assert_eq!(Locale::parse(locale.as_str()), Some(locale));
        }
        assert_eq!(Locale::parse("fr"), None);
        assert_eq!(Locale::parse("EN"), None);
        assert_eq!(Locale::En.intl_tag(), "en-GB");
        assert_eq!(Locale::De.intl_tag(), "de-DE");
    }

    #[test]
    fn a_thread_of_its_own_leaves_the_process_alone() {
        let process = get();
        {
            let _english = on_this_thread(Locale::En);
            assert_eq!(get(), Locale::En);
            assert_eq!(tr("Verlassen", "Leave"), "Leave");
            set(Locale::De);
            assert_eq!(get(), Locale::De);
            assert_eq!(tr("Verlassen", "Leave"), "Verlassen");
            std::thread::spawn(move || assert_eq!(get(), process))
                .join()
                .unwrap();
        }
        assert_eq!(get(), process);
    }

    #[test]
    fn plural_has_one_and_other() {
        assert_eq!(plural(1, "Person", "Personen"), "Person");
        assert_eq!(plural(0, "Person", "Personen"), "Personen");
        assert_eq!(plural(2, "Person", "Personen"), "Personen");
    }

    #[test]
    fn pieces_split_tagged_text() {
        assert_eq!(
            pieces("Mehr in der <link>Hilfe</link>."),
            [
                Piece {
                    text: "Mehr in der ",
                    tag: None
                },
                Piece {
                    text: "Hilfe",
                    tag: Some("link")
                },
                Piece {
                    text: ".",
                    tag: None
                },
            ]
        );
        assert_eq!(
            pieces("<b>Lena</b> ist live"),
            [
                Piece {
                    text: "Lena",
                    tag: Some("b")
                },
                Piece {
                    text: " ist live",
                    tag: None
                },
            ]
        );
        // Not closed, not a tag, or a comparison: plain text.
        for plain in ["a <b>open", "1 < 2 > 0", "<>x</>", "< b>x</ b>"] {
            assert_eq!(
                pieces(plain),
                [Piece {
                    text: plain,
                    tag: None
                }]
            );
        }
        assert_eq!(pieces(""), []);
        assert_eq!(
            pieces("<link>ä</link><b>ö</b>"),
            [
                Piece {
                    text: "ä",
                    tag: Some("link")
                },
                Piece {
                    text: "ö",
                    tag: Some("b")
                },
            ]
        );
    }
}
