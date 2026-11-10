//! Operator area "Betrieb" (#517): who is an operator, the area bootstrap,
//! the audit trail, and disabling accounts.

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

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

use common::{Client, SESSION};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

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

// ---- Accounts: disable and enable ----

const TOM: &str = "tom@example.test";

/// Rows of `operator_audit` as (action, target_user).
async fn audit_rows(pool: &PgPool) -> Vec<(String, Option<Uuid>)> {
    sqlx::query_as("SELECT action, target_user FROM operator_audit ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn disable(op: &mut Client, id: &str, body: serde_json::Value) -> common::Response {
    op.send(
        Method::POST,
        &format!("/api/operator/accounts/{id}/disable"),
        Some(body),
    )
    .await
}

async fn enable(op: &mut Client, id: &str) -> common::Response {
    op.send(
        Method::POST,
        &format!("/api/operator/accounts/{id}/enable"),
        None,
    )
    .await
}

#[sqlx::test]
async fn the_account_routes_answer_401_and_404_like_the_area(pool: PgPool) {
    let state = operator_state(pool.clone());
    let tom = registered(&state, TOM, "Tom").await;
    let tom_id = tom.user_id().to_owned();
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let mut ben = registered(&state, "ben@example.test", "Ben").await;
    let routes = [
        (Method::GET, "/api/operator/accounts".to_owned(), None),
        (
            Method::POST,
            format!("/api/operator/accounts/{tom_id}/disable"),
            Some(json!({})),
        ),
        (
            Method::POST,
            format!("/api/operator/accounts/{tom_id}/enable"),
            None,
        ),
    ];
    for (method, path, body) in routes {
        let res = anonymous.send(method.clone(), &path, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}: {}", res.body);
        let res = ben.send(method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}: {}", res.body);
    }
    assert!(audit_rows(&pool).await.is_empty());
    let disabled: bool =
        sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM users WHERE email = $1")
            .bind(TOM)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!disabled);
}

#[sqlx::test]
async fn disabling_ends_every_session_and_refuses_sign_in(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let tom_id = tom.user_id().to_owned();
    let mut tom_phone = Client::from_state(state.clone());
    tom_phone.bootstrap().await;
    assert_eq!(tom_phone.login(TOM, PASSWORD).await.status, StatusCode::OK);

    let res = disable(&mut lena, &tom_id, json!({ "reason": "  Spam  " })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["id"], tom_id);
    assert_eq!(res.body["status"], "disabled");
    assert_eq!(res.body["disabled_reason"], "Spam");
    assert!(res.body["disabled_at"].is_string());
    assert_eq!(res.body["live_sessions"], 0);
    assert_eq!(res.body["operator"], false);

    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = $1")
        .bind(tom_id.parse::<Uuid>().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    // The old cookies sign in nobody.
    for client in [&mut tom, &mut tom_phone] {
        let res = client.send(Method::GET, "/api/servers", None).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
        assert!(client.bootstrap().await.body["user"].is_null());
    }

    // The right password learns why; a wrong one learns nothing.
    let res = tom.login(TOM, PASSWORD).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "account_disabled");
    assert!(res.set_cookie(SESSION).is_none());
    let res = tom.login(TOM, "wrong password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);
    assert_eq!(res.body["error"], "invalid_credentials");

    assert_eq!(
        audit_rows(&pool).await,
        vec![("accounts.disable".to_owned(), Some(tom_id.parse().unwrap()))]
    );
    let detail: serde_json::Value = sqlx::query_scalar("SELECT detail FROM operator_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(detail, json!({ "reason": "Spam" }));

    // A second disable changes nothing and writes no row.
    let res = disable(&mut lena, &tom_id, json!({ "reason": "again" })).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["disabled_reason"], "Spam");
    assert_eq!(audit_rows(&pool).await.len(), 1);

    // Enabling restores sign-in.
    let res = enable(&mut lena, &tom_id).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "active");
    assert!(res.body["disabled_at"].is_null());
    assert!(res.body["disabled_reason"].is_null());
    let res = tom.login(TOM, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    // Enabling an active account changes nothing and writes no row.
    let res = enable(&mut lena, &tom_id).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["live_sessions"], 1);
    assert_eq!(
        audit_rows(&pool)
            .await
            .into_iter()
            .map(|(action, _)| action)
            .collect::<Vec<_>>(),
        ["accounts.disable", "accounts.enable"]
    );
}

#[sqlx::test]
async fn operators_cannot_disable_themselves_or_each_other(pool: PgPool) {
    let mut state = operator_state(pool.clone());
    state.operators = Arc::new(HashSet::from([
        LENA.to_owned(),
        "max@example.test".to_owned(),
    ]));
    let mut lena = registered(&state, LENA, "Lena").await;
    let lena_id = lena.user_id().to_owned();
    let max = registered(&state, "max@example.test", "Max").await;

    let res = disable(&mut lena, &lena_id, json!({})).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["account"], "self");
    let res = disable(&mut lena, max.user_id(), json!({})).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");
    assert_eq!(
        res.body["message"],
        "Operators are set in API_OPERATOR_EMAILS."
    );
    let res = disable(&mut lena, &Uuid::new_v4().to_string(), json!({})).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = enable(&mut lena, &Uuid::new_v4().to_string()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = disable(&mut lena, "not-a-uuid", json!({})).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let tom = registered(&state, TOM, "Tom").await;
    let res = disable(
        &mut lena,
        tom.user_id(),
        json!({ "reason": "x".repeat(501) }),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["reason"], "too_long");

    // Refused mutations write no row and change nobody.
    assert!(audit_rows(&pool).await.is_empty());
    let disabled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE disabled_at IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(disabled, 0);
}

#[sqlx::test]
async fn the_account_list_searches_filters_and_pages(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let tom_id = tom.user_id().to_owned();
    let ben = registered(&state, "ben@example.test", "Ben_50%").await;
    let ben_id = ben.user_id().to_owned();
    let res = tom
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Abend" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    // A pending registration from a time with mail on.
    sqlx::query(
        "INSERT INTO users (email, name, password_hash, registration_pending) \
         VALUES ('pia@example.test', 'Pia', 'x', true)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        disable(&mut lena, &ben_id, json!({})).await.status,
        StatusCode::OK
    );

    let names = |res: &common::Response| -> Vec<String> {
        res.body["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let res = lena.send(Method::GET, "/api/operator/accounts", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 4);
    // Newest first.
    assert_eq!(names(&res), ["Pia", "Ben_50%", "Tom", "Lena"]);
    let accounts = res.body["accounts"].as_array().unwrap();
    assert_eq!(accounts[0]["status"], "pending");
    assert_eq!(accounts[1]["status"], "disabled");
    let tom_row = &accounts[2];
    assert_eq!(tom_row["id"], tom_id);
    assert_eq!(tom_row["email"], TOM);
    assert_eq!(tom_row["status"], "active");
    assert_eq!(tom_row["owned_servers"], 1);
    assert_eq!(tom_row["member_servers"], 1);
    assert_eq!(tom_row["live_sessions"], 1);
    assert_eq!(tom_row["operator"], false);
    assert_eq!(accounts[3]["operator"], true);
    assert!(tom_row.get("password_hash").is_none());

    let res = lena
        .send(Method::GET, "/api/operator/accounts?filter=disabled", None)
        .await;
    assert_eq!(names(&res), ["Ben_50%"]);
    assert_eq!(res.body["total"], 1);
    let res = lena
        .send(Method::GET, "/api/operator/accounts?filter=pending", None)
        .await;
    assert_eq!(names(&res), ["Pia"]);
    // Name or address, case-insensitively; `%` and `_` are literal.
    let res = lena
        .send(Method::GET, "/api/operator/accounts?q=TOM%40", None)
        .await;
    assert_eq!(names(&res), ["Tom"]);
    let res = lena
        .send(Method::GET, "/api/operator/accounts?q=n_5", None)
        .await;
    assert_eq!(names(&res), ["Ben_50%"]);
    let res = lena
        .send(Method::GET, "/api/operator/accounts?q=%25", None)
        .await;
    assert_eq!(names(&res), ["Ben_50%"]);
    let res = lena
        .send(Method::GET, "/api/operator/accounts?q=_", None)
        .await;
    assert_eq!(names(&res), ["Ben_50%"]);
    let res = lena
        .send(Method::GET, "/api/operator/accounts?offset=3", None)
        .await;
    assert_eq!(names(&res), ["Lena"]);
    assert_eq!(res.body["total"], 4);

    let q = "x".repeat(101);
    let res = lena
        .send(Method::GET, &format!("/api/operator/accounts?q={q}"), None)
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["q"], "too_long");
    let res = lena
        .send(
            Method::GET,
            "/api/operator/accounts?filter=blocked&offset=-1",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["filter"], "invalid");
    assert_eq!(res.body["fields"]["offset"], "invalid");

    // One row per successful read, with the query; none for the refused ones.
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action, detail FROM operator_audit WHERE action = 'accounts.list' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(
        rows[3].1,
        json!({ "q": "TOM@", "filter": "all", "offset": 0 })
    );
}

#[sqlx::test]
async fn a_disabled_account_cannot_confirm_or_reset_by_mail(pool: PgPool) {
    let mut state = operator_state(pool.clone());
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    // An operator's address must be confirmed while mail is on.
    sqlx::query(
        "INSERT INTO users (email, name, password_hash, email_verified_at) \
         VALUES ($1, 'Lena', 'x', now())",
    )
    .bind(LENA)
    .execute(&pool)
    .await
    .unwrap();

    let mut tom = Client::from_state(state.clone());
    tom.bootstrap().await;
    let res = tom.register(TOM, PASSWORD, "Tom").await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let confirm = mail_token(&outbox(&mailer, 1).await[0]);
    let res = tom
        .send(
            Method::POST,
            "/api/auth/password-reset",
            Some(json!({ "email": TOM })),
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.body);
    let reset = mail_token(&outbox(&mailer, 2).await[1]);

    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = $1")
        .bind(TOM)
        .execute(&pool)
        .await
        .unwrap();
    let hash_before: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE email = $1")
            .bind(TOM)
            .fetch_one(&pool)
            .await
            .unwrap();

    let res = tom
        .send(
            Method::POST,
            "/api/auth/password-reset/confirm",
            Some(json!({ "token": reset, "password": "another horse" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "account_disabled");
    assert!(res.set_cookie(SESSION).is_none());
    let hash_after: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = $1")
        .bind(TOM)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hash_before, hash_after);

    let res = tom
        .send(
            Method::POST,
            "/api/auth/verify-email",
            Some(json!({ "token": confirm, "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "account_disabled");
    assert!(res.set_cookie(SESSION).is_none());

    // Login with the right password neither signs in nor mails a new link.
    let res = tom.login(TOM, PASSWORD).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "account_disabled");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 2);
}

/// Mails go out in the background; wait until `count` have been taken.
async fn outbox(mailer: &Mailer, count: usize) -> Vec<gelabber_api::mail::Sent> {
    for _ in 0..200 {
        let sent = mailer.outbox();
        if sent.len() >= count {
            return sent;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("expected {count} mails, got {:?}", mailer.outbox());
}

/// The token of the link in `mail` (`…#token=<64 hex>`).
fn mail_token(mail: &gelabber_api::mail::Sent) -> String {
    let (_, rest) = mail.body.split_once("#token=").expect("link in mail");
    rest.chars().take(64).collect()
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr, client: &Client) -> Ws {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut().insert(
        "cookie",
        format!("{SESSION}={}", client.jar[SESSION])
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(req)
        .await
        .expect("connect")
        .0
}

#[sqlx::test]
async fn disabling_closes_the_accounts_open_sockets(pool: PgPool) {
    let mut state = common::ws_state(pool);
    state.operators = Arc::new(HashSet::from([LENA.to_owned()]));
    let addr = common::serve_ws_state(state.clone()).await;
    let mut lena = registered(&state, LENA, "Lena").await;
    let tom = registered(&state, TOM, "Tom").await;
    let mut ws = connect(addr, &tom).await;
    // Open and kept alive until the account is disabled.
    let open = tokio::time::timeout(Duration::from_millis(500), closed(&mut ws)).await;
    assert!(open.is_err(), "socket closed before disabling");

    let res = disable(&mut lena, tom.user_id(), json!({})).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    tokio::time::timeout(Duration::from_secs(3), closed(&mut ws))
        .await
        .expect("socket closed after disabling");
}

/// Answers heartbeats until the socket closes.
async fn closed(ws: &mut Ws) {
    while let Some(frame) = ws.next().await {
        match frame {
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(Message::Text(raw)) => {
                let frame: serde_json::Value = serde_json::from_str(&raw).unwrap();
                if frame["op"] == "h" {
                    let _ = ws
                        .send(Message::Text(json!({ "op": "h" }).to_string().into()))
                        .await;
                }
            }
            _ => {}
        }
    }
}
