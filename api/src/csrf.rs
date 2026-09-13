//! CSRF protection for every mutating `/api` request.
//!
//! Double-submit with a body-delivered token: the server sets an `HttpOnly`
//! `gelabber_csrf` cookie and returns the same value in the JSON body of
//! `GET /api/auth/session` (and of login/register/logout, which rotate it).
//! The web client sends it back as `X-CSRF-Token` on every non-safe request.
//! A cross-site page can neither read that JSON (same-origin policy) nor set
//! the header without a CORS preflight, so it cannot forge a matching pair.
//!
//! As a second, independent check the `Sec-Fetch-Site` header — which
//! browsers attach and pages cannot spoof — rejects `cross-site` mutations
//! outright. Non-browser clients do not send it and are unaffected.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, Method};
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::cookies::{self, CSRF_COOKIE};
use crate::error::ApiError;

pub const CSRF_HEADER: HeaderName = HeaderName::from_static("x-csrf-token");
const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// Axum middleware: lets safe methods through, verifies everything else.
pub async fn require(request: Request, next: Next) -> Result<Response, ApiError> {
    if is_safe(request.method()) {
        return Ok(next.run(request).await);
    }
    check(request.headers())?;
    Ok(next.run(request).await)
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The pure check, separated so it can be unit-tested without a router.
pub fn check(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers
        .get(&SEC_FETCH_SITE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|site| site.eq_ignore_ascii_case("cross-site"))
    {
        return Err(ApiError::Csrf);
    }

    let cookie = cookies::get(headers, CSRF_COOKIE).ok_or(ApiError::Csrf)?;
    let header = headers
        .get(&CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Csrf)?;

    if cookie.is_empty() || !bool::from(cookie.as_bytes().ct_eq(header.as_bytes())) {
        return Err(ApiError::Csrf);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::COOKIE;

    fn headers(cookie: Option<&str>, header: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(cookie) = cookie {
            map.insert(COOKIE, format!("{CSRF_COOKIE}={cookie}").parse().unwrap());
        }
        if let Some(header) = header {
            map.insert(CSRF_HEADER, header.parse().unwrap());
        }
        map
    }

    #[test]
    fn accepts_matching_pair() {
        assert!(check(&headers(Some("abc"), Some("abc"))).is_ok());
    }

    #[test]
    fn rejects_missing_header() {
        assert!(matches!(
            check(&headers(Some("abc"), None)),
            Err(ApiError::Csrf)
        ));
    }

    #[test]
    fn rejects_missing_cookie() {
        assert!(matches!(
            check(&headers(None, Some("abc"))),
            Err(ApiError::Csrf)
        ));
    }

    #[test]
    fn rejects_mismatch_and_empty() {
        assert!(matches!(
            check(&headers(Some("abc"), Some("abd"))),
            Err(ApiError::Csrf)
        ));
        assert!(matches!(
            check(&headers(Some(""), Some(""))),
            Err(ApiError::Csrf)
        ));
    }

    #[test]
    fn rejects_cross_site_even_with_matching_pair() {
        let mut map = headers(Some("abc"), Some("abc"));
        map.insert(SEC_FETCH_SITE, "cross-site".parse().unwrap());
        assert!(matches!(check(&map), Err(ApiError::Csrf)));

        map.insert(SEC_FETCH_SITE, "same-origin".parse().unwrap());
        assert!(check(&map).is_ok());
    }

    #[test]
    fn safe_methods_are_safe() {
        assert!(is_safe(&Method::GET));
        assert!(is_safe(&Method::HEAD));
        assert!(!is_safe(&Method::POST));
        assert!(!is_safe(&Method::PATCH));
        assert!(!is_safe(&Method::DELETE));
    }
}
