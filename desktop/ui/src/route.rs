//! The web client's routes (web/src/routes.tsx), so links, the parity
//! scenes and the "last place" memory use the same paths on both clients.

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Login,
    Register,
    ForgotPassword,
    ResetPassword,
    VerifyEmail,
    Invite(String),
    /// `/`: the last place, or the first server.
    Home,
    Server(Uuid),
    Channel(Uuid, Uuid),
    ServerSettings(Uuid),
    /// `/settings` and `/settings/$area`.
    Settings(Option<String>),
    Dms,
    Dm(Uuid),
    Profile,
    NotFound(String),
    /// `/discover?q=&lang=`: the directory, its search and language filter.
    Discover(DiscoverSearch),
    /// `/sign-in/name#token=`: a first sign-in through a provider names
    /// its new account, signed in or not.
    SignInName,
}

/// The directory's search in the address (`validateDiscoverSearch` in
/// web/src/discover/search.ts), so a reload keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoverSearch {
    /// Trimmed, at most [`DiscoverSearch::QUERY_MAX`] characters, never
    /// empty.
    pub q: Option<String>,
    /// Two lowercase letters, a language code of the directory.
    pub lang: Option<String>,
}

impl DiscoverSearch {
    /// `QUERY_MAX` of web/src/discover/rules.ts.
    pub const QUERY_MAX: usize = 100;

    /// What is left of `q` and `lang` once checked like the web.
    pub fn new(q: Option<&str>, lang: Option<&str>) -> Self {
        let q = q
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| q.chars().take(Self::QUERY_MAX).collect());
        let lang = lang
            .filter(|l| l.len() == 2 && l.bytes().all(|b| b.is_ascii_lowercase()))
            .map(str::to_owned);
        Self { q, lang }
    }

    /// From a query string such as `q=rust&lang=en`, without the `?`.
    fn parse(query: &str) -> Self {
        let mut q = None;
        let mut lang = None;
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "q" if q.is_none() => q = Some(value.into_owned()),
                "lang" if lang.is_none() => lang = Some(value.into_owned()),
                _ => {}
            }
        }
        Self::new(q.as_deref(), lang.as_deref())
    }

    /// `?q=…&lang=…`, or nothing when neither is set.
    fn query(&self) -> String {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        if let Some(q) = &self.q {
            out.append_pair("q", q);
        }
        if let Some(lang) = &self.lang {
            out.append_pair("lang", lang);
        }
        let query = out.finish();
        if query.is_empty() {
            query
        } else {
            format!("?{query}")
        }
    }
}

impl Route {
    /// A path, with its query: only `/discover` reads one.
    pub fn parse(path: &str) -> Self {
        let href = path.split('#').next().unwrap_or("");
        let (path, query) = href.split_once('?').unwrap_or((href, ""));
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let id = |s: &str| Uuid::parse_str(s).ok();
        match parts.as_slice() {
            [] => Self::Home,
            ["login"] => Self::Login,
            ["register"] => Self::Register,
            ["forgot-password"] => Self::ForgotPassword,
            ["reset-password"] => Self::ResetPassword,
            ["verify-email"] => Self::VerifyEmail,
            ["invite", code] => Self::Invite((*code).to_owned()),
            ["profile"] => Self::Profile,
            ["settings"] => Self::Settings(None),
            ["settings", area] => Self::Settings(Some((*area).to_owned())),
            ["d"] => Self::Dms,
            ["d", c] => id(c).map_or_else(|| Self::NotFound(path.to_owned()), Self::Dm),
            ["s", s] => id(s).map_or_else(|| Self::NotFound(path.to_owned()), Self::Server),
            ["s", s, "settings"] => {
                id(s).map_or_else(|| Self::NotFound(path.to_owned()), Self::ServerSettings)
            }
            ["s", s, "c", c] => match (id(s), id(c)) {
                (Some(s), Some(c)) => Self::Channel(s, c),
                _ => Self::NotFound(path.to_owned()),
            },
            ["discover"] => Self::Discover(DiscoverSearch::parse(query)),
            ["sign-in", "name"] => Self::SignInName,
            _ => Self::NotFound(path.to_owned()),
        }
    }

    pub fn path(&self) -> String {
        match self {
            Self::Login => "/login".into(),
            Self::Register => "/register".into(),
            Self::ForgotPassword => "/forgot-password".into(),
            Self::ResetPassword => "/reset-password".into(),
            Self::VerifyEmail => "/verify-email".into(),
            Self::Invite(code) => format!("/invite/{code}"),
            Self::Home => "/".into(),
            Self::Server(s) => format!("/s/{s}"),
            Self::Channel(s, c) => format!("/s/{s}/c/{c}"),
            Self::ServerSettings(s) => format!("/s/{s}/settings"),
            Self::Settings(None) => "/settings".into(),
            Self::Settings(Some(area)) => format!("/settings/{area}"),
            Self::Dms => "/d".into(),
            Self::Dm(c) => format!("/d/{c}"),
            Self::Profile => "/profile".into(),
            Self::NotFound(p) => p.clone(),
            Self::Discover(search) => format!("/discover{}", search.query()),
            Self::SignInName => "/sign-in/name".into(),
        }
    }

    /// Pages that are only for signed-out visitors (`requireAnonymous`).
    pub fn is_anonymous(&self) -> bool {
        matches!(
            self,
            Self::Login
                | Self::Register
                | Self::ForgotPassword
                | Self::ResetPassword
                | Self::VerifyEmail
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip() {
        let s = Uuid::from_u128(1);
        let c = Uuid::from_u128(2);
        for route in [
            Route::Login,
            Route::Home,
            Route::Server(s),
            Route::Channel(s, c),
            Route::ServerSettings(s),
            Route::Settings(None),
            Route::Settings(Some("darstellung".into())),
            Route::Dms,
            Route::Dm(c),
            Route::Invite("abc".into()),
        ] {
            assert_eq!(Route::parse(&route.path()), route);
        }
    }

    fn discover(q: Option<&str>, lang: Option<&str>) -> Route {
        Route::Discover(DiscoverSearch {
            q: q.map(str::to_owned),
            lang: lang.map(str::to_owned),
        })
    }

    /// `/discover` keeps its search like `validateDiscoverSearch`.
    #[test]
    fn discover_reads_its_search() {
        for (href, route) in [
            ("/discover", discover(None, None)),
            ("/discover/", discover(None, None)),
            ("/discover?q=rust", discover(Some("rust"), None)),
            ("/discover?lang=en", discover(None, Some("en"))),
            (
                "/discover?q=Modell+bau&lang=de",
                discover(Some("Modell bau"), Some("de")),
            ),
            (
                "/discover?q=%20%20film%20&lang=de",
                discover(Some("film"), Some("de")),
            ),
            ("/discover?q=2026", discover(Some("2026"), None)),
            ("/discover?q=caf%C3%A9#oben", discover(Some("café"), None)),
            // Blank search and a language that is not a code are dropped.
            ("/discover?q=%20&lang=", discover(None, None)),
            ("/discover?lang=EN", discover(None, None)),
            ("/discover?lang=deu", discover(None, None)),
            ("/discover?lang=d1", discover(None, None)),
            ("/discover?other=1", discover(None, None)),
        ] {
            assert_eq!(Route::parse(href), route, "{href}");
        }
        let long = "ä".repeat(150);
        let Route::Discover(search) = Route::parse(&format!("/discover?q={long}")) else {
            panic!("not discover");
        };
        assert_eq!(search.q.unwrap().chars().count(), DiscoverSearch::QUERY_MAX);
        assert_eq!(
            Route::parse("/discover/x"),
            Route::NotFound("/discover/x".into())
        );
        // Other routes ignore a query.
        assert_eq!(Route::parse("/d?q=1"), Route::Dms);
    }

    /// The name step of a provider sign-in; its token is in the fragment.
    #[test]
    fn sign_in_name_parses_and_writes() {
        assert_eq!(Route::parse("/sign-in/name"), Route::SignInName);
        assert_eq!(Route::parse("/sign-in/name#token=abc"), Route::SignInName);
        assert_eq!(Route::SignInName.path(), "/sign-in/name");
        assert!(!Route::SignInName.is_anonymous());
        assert_eq!(Route::parse("/sign-in"), Route::NotFound("/sign-in".into()));
    }

    #[test]
    fn discover_writes_its_search() {
        assert_eq!(discover(None, None).path(), "/discover");
        assert_eq!(discover(Some("rust"), None).path(), "/discover?q=rust");
        assert_eq!(discover(None, Some("en")).path(), "/discover?lang=en");
        assert_eq!(
            discover(Some("rust & co"), Some("de")).path(),
            "/discover?q=rust+%26+co&lang=de"
        );
        for route in [
            discover(None, None),
            discover(Some("Modellbau-Keller"), Some("de")),
            discover(Some("50% _off_ #1?"), None),
            discover(Some("Українська"), Some("uk")),
        ] {
            assert_eq!(Route::parse(&route.path()), route);
        }
    }
}
