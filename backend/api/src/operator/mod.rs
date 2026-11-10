//! `/api/operator/*`: the instance operators' area "Betrieb" (v0.10, #517).
//! Every route goes through the [`Operator`] extractor: `401` without a
//! session, `404` for anyone who is not an operator.

pub mod access;
pub mod accounts;
pub mod admission;
pub mod audit;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;

use crate::config::RegistrationMode;
use crate::error::ApiError;
use crate::state::AppState;

pub use self::access::Operator;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/operator", get(overview))
        .merge(accounts::router())
}

/// `GET /api/operator`: what the area needs before its first page. The web
/// client refetches it for the reports badge, so it writes no audit row.
#[derive(Debug, Serialize)]
pub struct Overview {
    pub registration: RegistrationMode,
    pub mail: bool,
    pub open_reports: i64,
}

async fn overview(
    State(state): State<AppState>,
    _op: Operator,
) -> Result<Json<Overview>, ApiError> {
    let open_reports: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE status = 'open'")
            .fetch_one(&state.db)
            .await?;
    Ok(Json(Overview {
        registration: state.registration,
        mail: state.mail.is_some(),
        open_reports,
    }))
}
