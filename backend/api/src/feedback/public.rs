//! The public feedback form (#542): visitors without an account post a
//! static form from `/{lang}/feedback`, get a confirmation link by mail and
//! reach the operators only once they open it.
//!
//! These routes live outside the CSRF layer, which a plain HTML form cannot
//! pass. They refuse `cross-site` and `same-site` posts themselves, count
//! posts per client address (`feedback_public`) and mails per address
//! (`MailBudget::Feedback`), and lean on the confirmation for the rest.
//! Every answer but the off-state `404` and the cross-site `403` is a
//! `303 See Other` to a static page in the form's language.
//!
//! The token travels in the query string, because a page without script
//! cannot read a fragment. Its hash stays until it runs out, so a second
//! open (a mail scanner, a second click) lands on the thank-you page and
//! changes nothing. The sweeper ([`start`]) deletes pending entries whose
//! link ran out and clears the hashes of confirmed ones.

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::rejection::{FormRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Form, Query, State};
use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::{HeaderMap, HeaderName};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use super::{enabled, notify};
use crate::auth::oidc::routes::Peer;
use crate::auth::{self, validate};
use crate::error::{ApiError, FieldErrors};
use crate::i18n::Locale;
use crate::limits::{self, FEEDBACK_PENDING_MAX, FEEDBACK_PUBLIC_PER_HOUR, HOUR, MailBudget};
use crate::mail::Mail;
use crate::state::AppState;
use crate::token;

/// Largest form body: 4,000 characters of text, percent-encoded, and the
/// other fields fit with room to spare.
const BODY_MAX: usize = 32 * 1024;
/// How long a confirmation link holds.
const CONFIRM_HOURS: i32 = 24;
/// How often the sweeper runs.
const SWEEP_EVERY: Duration = Duration::from_secs(10 * 60);

const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/public/feedback", post(submit))
        .route("/api/public/feedback/confirm", get(confirm))
        .layer(DefaultBodyLimit::max(BODY_MAX))
}

/// The form is on while feedback is on and account mail is set up: without
/// mail, nothing could confirm an entry.
fn available(state: &AppState) -> bool {
    enabled(state) && state.mail.is_some()
}

/// The static pages the routes answer with.
#[derive(Debug, Clone, Copy)]
enum Page {
    Sent,
    Thanks,
    Expired,
    Error,
}

/// `303 See Other` to `page` in `locale`, as a relative location.
fn to(locale: Locale, page: Page) -> Response {
    let slug = match (locale, page) {
        (Locale::De, Page::Sent) => "gesendet",
        (Locale::De, Page::Thanks) => "danke",
        (Locale::De, Page::Expired) => "abgelaufen",
        (Locale::De, Page::Error) => "fehler",
        (Locale::En, Page::Sent) => "sent",
        (Locale::En, Page::Thanks) => "thanks",
        (Locale::En, Page::Expired) => "expired",
        (Locale::En, Page::Error) => "error",
    };
    Redirect::to(&format!("/{}/feedback/{slug}", locale.as_str())).into_response()
}

/// The form's fields. All optional: a missing field is the error page, not
/// axum's plain-text rejection.
#[derive(Debug, Default, Deserialize)]
pub struct FormBody {
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// The honeypot: people leave it empty, some bots fill it.
    #[serde(default)]
    website: Option<String>,
}

/// `POST /api/public/feedback`: stores a pending entry and mails its
/// confirmation link.
async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    form: Result<Form<FormBody>, FormRejection>,
) -> Result<Response, ApiError> {
    if !available(&state) {
        return Err(ApiError::NotFound);
    }
    // A page elsewhere could otherwise make a visitor's browser mail a
    // confirmation to any address. Bots without the header meet the rest.
    if let Some(site) = headers.get(&SEC_FETCH_SITE)
        && matches!(site.as_bytes(), b"cross-site" | b"same-site")
    {
        return Err(ApiError::Csrf);
    }
    let form = form.map(|Form(form)| form);
    let locale = form
        .as_ref()
        .ok()
        .and_then(|form| form.lang.as_deref())
        .and_then(Locale::parse)
        .unwrap_or(Locale::En);

    let ip = limits::client_ip(&headers, peer);
    if state
        .limiter
        .hit("feedback_public", &ip, FEEDBACK_PUBLIC_PER_HOUR, HOUR)
        .is_err()
    {
        state.metrics.rate_limited("feedback_public");
        return Ok(to(locale, Page::Error));
    }
    let Ok(form) = form else {
        return Ok(to(locale, Page::Error));
    };
    if form.website.as_deref().is_some_and(|value| !value.is_empty()) {
        info!("feedback form: honeypot");
        return Ok(to(locale, Page::Sent));
    }

    let mut errors = FieldErrors::new();
    let kind = super::kind(form.kind.as_deref(), &mut errors);
    let text = super::text(form.text.as_deref(), &mut errors);
    let email = validate::new_email(form.email.as_deref().unwrap_or_default(), &mut errors);
    let (Some(kind), Some(text), Some(email)) = (kind, text, email) else {
        return Ok(to(locale, Page::Error));
    };

    // Same page as success: the answer does not tell who has been mailed.
    if !limits::mail_allowed(&state, MailBudget::Feedback, &email) {
        return Ok(to(locale, Page::Sent));
    }
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM feedback WHERE status = 'pending'")
            .fetch_one(&state.db)
            .await
            .inspect_err(|_| limits::mail_unsent(&state, MailBudget::Feedback, &email))?;
    if pending >= FEEDBACK_PENDING_MAX {
        limits::mail_unsent(&state, MailBudget::Feedback, &email);
        warn!("feedback: too many pending entries");
        return Ok(to(locale, Page::Error));
    }

    let token = token::generate();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO feedback (kind, visitor_email, visitor_locale, client, locale, body, \
                               status, confirm_hash, confirm_expires_at) \
         VALUES ($1, $2, $3, 'form', $3, $4, 'pending', $5, now() + make_interval(hours => $6)) \
         RETURNING id",
    )
    .bind(kind.as_str())
    .bind(&email)
    .bind(locale.as_str())
    .bind(&text)
    .bind(token::hash(&token))
    .bind(CONFIRM_HOURS)
    .fetch_one(&state.db)
    .await
    .inspect_err(|_| limits::mail_unsent(&state, MailBudget::Feedback, &email))?;

    let base = state
        .mail
        .as_ref()
        .map(|mailer| mailer.public_url().to_owned())
        .unwrap_or_default();
    let link = format!("{base}/api/public/feedback/confirm?token={token}");
    auth::send_mail_to(
        &state,
        &email,
        locale,
        Mail::FeedbackConfirm { link },
        MailBudget::Feedback,
    );
    info!(feedback_id = %id, kind = kind.as_str(), "feedback form: entry pending");
    Ok(to(locale, Page::Sent))
}

#[derive(Debug, Default, Deserialize)]
pub struct ConfirmQuery {
    #[serde(default)]
    token: Option<String>,
}

/// `GET /api/public/feedback/confirm?token=…`: confirms the entry once and
/// lands on the thank-you page, also on every later open while the link
/// holds.
async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    query: Result<Query<ConfirmQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    if !available(&state) {
        return Err(ApiError::NotFound);
    }
    let asked = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(Locale::from_accept_language)
        .unwrap_or(Locale::En);
    if limits::anonymous_hit(&state, &headers, peer).is_err() {
        return Ok(to(asked, Page::Error));
    }
    let raw = query
        .ok()
        .and_then(|Query(query)| query.token)
        .filter(|raw| token::is_valid(raw));
    let Some(raw) = raw else {
        return Ok(to(asked, Page::Expired));
    };
    let hash = token::hash(&raw);

    let confirmed: Option<(Uuid, Locale)> = sqlx::query_as(
        "UPDATE feedback SET status = 'open', last_activity_at = now() \
         WHERE confirm_hash = $1 AND status = 'pending' AND confirm_expires_at > now() \
         RETURNING id, visitor_locale",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;
    if let Some((id, locale)) = confirmed {
        info!(feedback_id = %id, "feedback form: entry confirmed");
        notify::new_entry(&state, id);
        return Ok(to(locale, Page::Thanks));
    }

    // Confirmed before (a scanner, a second click), or ran out.
    let row: Option<(Locale, bool)> = sqlx::query_as(
        "SELECT visitor_locale, status <> 'pending' AND confirm_expires_at > now() \
         FROM feedback WHERE confirm_hash = $1",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;
    Ok(match row {
        Some((locale, true)) => to(locale, Page::Thanks),
        Some((locale, false)) => to(locale, Page::Expired),
        None => to(asked, Page::Expired),
    })
}

/// Starts the sweeper once per process: every [`SWEEP_EVERY`] it deletes
/// pending entries whose link ran out and clears the run-out hashes of
/// confirmed ones.
pub fn start(state: AppState) {
    if state.feedback_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        while !state.db.is_closed() {
            if let Err(err) = sweep(&state).await {
                warn!(error = ?err, "feedback sweep failed");
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}

/// One sweep. Returns how many pending entries were deleted and how many
/// hashes were cleared.
pub async fn sweep(state: &AppState) -> Result<(u64, u64), ApiError> {
    let deleted = sqlx::query(
        "DELETE FROM feedback WHERE status = 'pending' AND confirm_expires_at <= now()",
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    let cleared = sqlx::query(
        "UPDATE feedback SET confirm_hash = NULL, confirm_expires_at = NULL \
         WHERE status <> 'pending' AND confirm_expires_at <= now()",
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    if deleted > 0 || cleared > 0 {
        info!(deleted, cleared, "feedback sweep");
    }
    Ok((deleted, cleared))
}
