//! Minimal cookie helpers: read one cookie out of the request and build a
//! `Set-Cookie` value. Both cookies this API issues are `HttpOnly` and
//! `SameSite=Lax` on `Path=/`; `Secure` follows `API_COOKIE_SECURE`.

use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue};

pub const SESSION_COOKIE: &str = "gelabber_session";
pub const CSRF_COOKIE: &str = "gelabber_csrf";

/// Returns the value of `name` from the request's `Cookie` header(s). The
/// first occurrence wins, matching browser behaviour for equal-path cookies.
pub fn get(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_owned())
        })
        .next()
}

#[derive(Debug, Clone)]
pub struct SetCookie {
    name: &'static str,
    value: String,
    max_age_secs: u64,
    secure: bool,
}

impl SetCookie {
    pub fn new(name: &'static str, value: impl Into<String>, max_age_secs: u64) -> Self {
        Self {
            name,
            value: value.into(),
            max_age_secs,
            secure: false,
        }
    }

    /// Expires the cookie immediately.
    pub fn clear(name: &'static str) -> Self {
        Self::new(name, "", 0)
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn header_value(&self) -> HeaderValue {
        let mut out = format!(
            "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
            self.name, self.value, self.max_age_secs
        );
        if self.secure {
            out.push_str("; Secure");
        }
        HeaderValue::from_str(&out).expect("cookie values are ASCII tokens")
    }

    pub fn header(&self) -> (axum::http::HeaderName, HeaderValue) {
        (SET_COOKIE, self.header_value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_named_cookie_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "a=1; gelabber_session=abc ;b=2".parse().unwrap());
        assert_eq!(get(&headers, SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(get(&headers, "b").as_deref(), Some("2"));
        assert_eq!(get(&headers, "missing"), None);
    }

    #[test]
    fn ignores_prefix_matches() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "gelabber_session_old=x".parse().unwrap());
        assert_eq!(get(&headers, SESSION_COOKIE), None);
    }

    #[test]
    fn set_cookie_has_required_attributes() {
        let value = SetCookie::new(SESSION_COOKIE, "tok", 60).header_value();
        let text = value.to_str().unwrap();
        assert_eq!(
            text,
            "gelabber_session=tok; Path=/; Max-Age=60; HttpOnly; SameSite=Lax"
        );

        let secure = SetCookie::new(SESSION_COOKIE, "tok", 60)
            .secure(true)
            .header_value();
        assert!(secure.to_str().unwrap().ends_with("; Secure"));
    }

    #[test]
    fn clear_sets_zero_max_age() {
        let value = SetCookie::clear(CSRF_COOKIE).header_value();
        assert_eq!(
            value.to_str().unwrap(),
            "gelabber_csrf=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
        );
    }
}
