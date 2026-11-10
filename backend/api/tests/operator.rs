//! Operator area "Betrieb" (#517): who is an operator, the area bootstrap
//! and the audit trail.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use gelabber_api::auth::session;
use gelabber_api::config::RegistrationMode;
use gelabber_api::mail::Mailer;
use gelabber_api::operator::Operator;
use gelabber_api::operator::audit::{self, Action, Targets};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "correct horse";
const LENA: &str = "lena@example.test";

fn operator_state(pool: PgPool) -> AppState {
    let mut state = common::state(pool);
    state.operators = Arc::new(HashSet::from([LENA.to_owned()]));
    state
}

async fn registered(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

/// The same browser (cookies, CSRF token) against another state.
fn moved(client: &Client, state: &AppState) -> Client {
    let mut moved = Client::from_state(state.clone());
    moved.jar = client.jar.clone();
    moved.csrf = client.csrf.clone();
    moved.user_id = client.user_id.clone();
    moved
}

#[sqlx::test]
async fn anonymous_callers_get_401_and_other_accounts_404(pool: PgPool) {
    let state = operator_state(pool);
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let res = anonymous.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    assert_eq!(res.body["error"], "unauthenticated");

    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let res = tom.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    assert_eq!(res.body["error"], "not_found");
    // An unknown route below the area answers alike.
    let res = tom.send(Method::GET, "/api/operator/nothing", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn an_operator_gets_the_area_bootstrap(pool: PgPool) {
    let mut state = operator_state(pool.clone());
    // Listed in another spelling: the address is matched case-insensitively.
    let mut lena = registered(&state, "Lena@Example.TEST", "Lena").await;
    let res = lena.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body,
        json!({ "registration": "open", "mail": false, "open_reports": 0 })
    );

    // Open reports are counted, closed ones are not.
    let server = lena
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Abend" })),
        )
        .await;
    assert_eq!(server.status, StatusCode::CREATED, "{}", server.body);
    let sid: Uuid = server.body["id"].as_str().unwrap().parse().unwrap();
    let tom = registered(&state, "tom@example.test", "Tom").await;
    let tom_id: Uuid = tom.user_id().parse().unwrap();
    let lena_id: Uuid = lena.user_id().parse().unwrap();
    sqlx::query(
        "INSERT INTO reports (server_id, reporter_id, reason, details, snapshot) \
         VALUES ($1, $2, 'spam', '', '{\"name\":\"Abend\"}'), ($1, $3, 'other', 'x', NULL)",
    )
    .bind(sid)
    .bind(tom_id)
    .bind(lena_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE reports SET status = 'dismissed', resolved_at = now() WHERE reporter_id = $1",
    )
    .bind(lena_id)
    .execute(&pool)
    .await
    .unwrap();

    state.registration = RegistrationMode::Invitation;
    let mut lena = moved(&lena, &state);
    let res = lena.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body,
        json!({ "registration": "invitation", "mail": false, "open_reports": 1 })
    );
    // The bootstrap writes no audit row.
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operator_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn the_session_says_who_is_an_operator(pool: PgPool) {
    let state = operator_state(pool);
    let mut lena = Client::from_state(state.clone());
    let res = lena.bootstrap().await;
    assert_eq!(res.body["operator"], false);
    let res = lena.register(LENA, PASSWORD, "Lena").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["operator"], true);
    let res = lena.bootstrap().await;
    assert_eq!(res.body["operator"], true, "{}", res.body);
    let res = lena.login(LENA, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["operator"], true);

    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let res = tom.bootstrap().await;
    assert_eq!(res.body["operator"], false);
    let res = tom.login("tom@example.test", PASSWORD).await;
    assert_eq!(res.body["operator"], false);
}

#[sqlx::test]
async fn with_mail_on_an_unconfirmed_operator_address_is_no_operator(pool: PgPool) {
    // Registered while the instance had no mail: signed in, never confirmed.
    let lena = registered(&operator_state(pool.clone()), LENA, "Lena").await;

    let mut state = operator_state(pool.clone());
    state.mail = Some(Mailer::memory(
        &url::Url::parse("https://chat.example.org/").unwrap(),
    ));
    let mut lena = moved(&lena, &state);
    let res = lena.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = lena.bootstrap().await;
    assert_eq!(res.body["operator"], false, "{}", res.body);

    sqlx::query("UPDATE users SET email_verified_at = now() WHERE email = $1")
        .bind(LENA)
        .execute(&pool)
        .await
        .unwrap();
    let res = lena.send(Method::GET, "/api/operator", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["mail"], true);
    let res = lena.bootstrap().await;
    assert_eq!(res.body["operator"], true, "{}", res.body);
}

#[sqlx::test]
async fn record_writes_one_audit_row(pool: PgPool) {
    let state = operator_state(pool.clone());
    let lena = registered(&state, LENA, "Lena").await;
    let lena_id: Uuid = lena.user_id().parse().unwrap();
    let op = Operator {
        user: session::reload(&pool, lena_id).await.unwrap(),
        session_key: "test".to_owned(),
    };
    let target = Uuid::new_v4();
    let mut conn = pool.acquire().await.unwrap();
    audit::record(
        &mut conn,
        &op,
        Action::ServersBlock,
        Targets {
            server: Some(target),
            ..Targets::default()
        },
        json!({ "reason": "Spam" }),
    )
    .await
    .unwrap();

    let row: (
        Option<Uuid>,
        String,
        String,
        Option<Uuid>,
        Option<Uuid>,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT operator_id, operator_email, action, target_server, target_user, detail \
             FROM operator_audit",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            Some(lena_id),
            LENA.to_owned(),
            "servers.block".to_owned(),
            Some(target),
            None,
            json!({ "reason": "Spam" }),
        )
    );
}

#[sqlx::test]
async fn reports_take_only_the_known_reasons(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let server = lena
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Abend" })),
        )
        .await;
    let sid: Uuid = server.body["id"].as_str().unwrap().parse().unwrap();
    for reason in ["spam", "harassment", "sexual", "illegal", "other"] {
        sqlx::query("INSERT INTO reports (server_id, reason) VALUES ($1, $2)")
            .bind(sid)
            .bind(reason)
            .execute(&pool)
            .await
            .unwrap_or_else(|err| panic!("{reason}: {err}"));
    }
    let refused = sqlx::query("INSERT INTO reports (server_id, reason) VALUES ($1, 'abuse')")
        .bind(sid)
        .execute(&pool)
        .await;
    assert!(refused.is_err());
}
