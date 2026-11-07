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
}

impl Route {
    pub fn parse(path: &str) -> Self {
        let path = path.split(['?', '#']).next().unwrap_or("");
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
}
