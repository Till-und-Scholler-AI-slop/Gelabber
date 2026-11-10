//! Operator area "Betrieb" (#517): who is an operator, the area bootstrap,
//! the audit trail, disabling accounts and blocking and delisting servers.

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

// ---- Beta invitations ----

/// Rows of `operator_audit` as (action, target_invitation).
async fn invitation_audit(pool: &PgPool) -> Vec<(String, Option<Uuid>)> {
    sqlx::query_as(
        "SELECT action, target_invitation FROM operator_audit \
         WHERE action LIKE 'invitations.%' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn invite(op: &mut Client, body: serde_json::Value) -> common::Response {
    op.send(Method::POST, "/api/operator/invitations", Some(body))
        .await
}

#[sqlx::test]
async fn the_invitation_routes_answer_401_and_404_like_the_area(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut tom = registered(&state, TOM, "Tom").await;
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let id = Uuid::new_v4();
    for (method, path, body) in [
        (Method::GET, "/api/operator/invitations".to_owned(), None),
        (
            Method::POST,
            "/api/operator/invitations".to_owned(),
            Some(json!({})),
        ),
        (
            Method::DELETE,
            format!("/api/operator/invitations/{id}"),
            None,
        ),
    ] {
        let res = anonymous.send(method.clone(), &path, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}: {}", res.body);
        let res = tom.send(method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}: {}", res.body);
    }
    let invitations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM beta_invitations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(invitations, 0);
    assert!(invitation_audit(&pool).await.is_empty());
}

#[sqlx::test]
async fn an_operator_creates_lists_and_revokes_invitations(pool: PgPool) {
    let mut state = operator_state(pool.clone());
    state.public_url = Some(url::Url::parse("https://chat.example.org/").unwrap());
    let mut lena = registered(&state, LENA, "Lena").await;
    let lena_id = lena.user_id().to_owned();

    let res = invite(
        &mut lena,
        json!({ "email": " Mia@Example.TEST ", "note": " Mias Runde ", "expires_in_days": 7 }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mia = res.body.clone();
    let code = mia["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 12);
    assert_eq!(
        mia["link"],
        format!("https://chat.example.org/register?invitation={code}")
    );
    assert_eq!(mia["email"], "mia@example.test");
    assert_eq!(mia["note"], "Mias Runde");
    assert_eq!(mia["status"], "open");
    assert_eq!(mia["created_by"], json!({ "id": lena_id, "name": "Lena" }));
    assert!(mia["used_by"].is_null() && mia["used_at"].is_null());
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(mia["expires_at"].clone()).unwrap();
    let days = (expires - chrono::Utc::now()).num_hours();
    assert!((167..=168).contains(&days), "{days}");

    let res = invite(&mut lena, json!({ "expires_in_days": null })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let anyone = res.body.clone();
    assert!(anyone["email"].is_null() && anyone["note"].is_null());
    assert!(anyone["expires_at"].is_null());
    let anyone_id = anyone["id"].as_str().unwrap().to_owned();

    let res = lena
        .send(Method::GET, "/api/operator/invitations", None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 2);
    assert_eq!(
        res.body["invitations"][0]["id"], anyone["id"],
        "newest first"
    );
    assert_eq!(res.body["invitations"][1], mia);

    // Revoking: once with an audit row, a second time without.
    for _ in 0..2 {
        let res = lena
            .send(
                Method::DELETE,
                &format!("/api/operator/invitations/{anyone_id}"),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    }
    let res = lena
        .send(Method::GET, "/api/operator/invitations?status=open", None)
        .await;
    assert_eq!(res.body["total"], 1, "{}", res.body);
    assert_eq!(res.body["invitations"][0]["id"], mia["id"]);
    let res = lena
        .send(Method::GET, "/api/operator/invitations?status=all", None)
        .await;
    assert_eq!(res.body["invitations"][0]["status"], "revoked");

    // Mia registers with hers on the invitation-only instance; then it
    // shows as used and cannot be revoked.
    let mut closed = state.clone();
    closed.registration = RegistrationMode::Invitation;
    let mut invited = Client::from_state(closed);
    invited.bootstrap().await;
    let res = invited
        .send(
            Method::POST,
            "/api/auth/register",
            Some(json!({
                "email": "mia@example.test", "password": PASSWORD, "name": "Mia",
                "invitation": code,
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = lena
        .send(Method::GET, "/api/operator/invitations?status=used", None)
        .await;
    assert_eq!(res.body["total"], 1, "{}", res.body);
    let used = &res.body["invitations"][0];
    assert_eq!(used["status"], "used");
    assert_eq!(
        used["used_by"],
        json!({ "id": invited.user_id(), "name": "Mia" })
    );
    assert!(used["used_at"].is_string());
    let mia_id = mia["id"].as_str().unwrap();
    let res = lena
        .send(
            Method::DELETE,
            &format!("/api/operator/invitations/{mia_id}"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"], json!({ "invitation": "used" }));
    let res = lena
        .send(
            Method::DELETE,
            &format!("/api/operator/invitations/{}", Uuid::new_v4()),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    let mia_uuid: Uuid = mia_id.parse().unwrap();
    let anyone_uuid: Uuid = anyone_id.parse().unwrap();
    assert_eq!(
        invitation_audit(&pool).await,
        vec![
            ("invitations.create".to_owned(), Some(mia_uuid)),
            ("invitations.create".to_owned(), Some(anyone_uuid)),
            ("invitations.list".to_owned(), None),
            ("invitations.revoke".to_owned(), Some(anyone_uuid)),
            ("invitations.list".to_owned(), None),
            ("invitations.list".to_owned(), None),
            ("invitations.list".to_owned(), None),
        ]
    );
    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM operator_audit WHERE action = 'invitations.create' ORDER BY id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        detail,
        json!({ "email": "mia@example.test", "note": "Mias Runde", "expires_in_days": 7, "send_mail": false })
    );
}

#[sqlx::test]
async fn invitation_links_without_a_public_url_use_the_requests_host(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let res = invite(&mut lena, json!({})).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let link = res.body["link"].as_str().unwrap();
    let code = res.body["code"].as_str().unwrap();
    assert!(
        link.ends_with(&format!("/register?invitation={code}")),
        "{link}"
    );
}

#[sqlx::test]
async fn invitation_requests_are_validated(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    registered(&state, TOM, "Tom").await;
    // A pending registration may still be invited; it is taken over.
    sqlx::query(
        "INSERT INTO users (email, name, password_hash, registration_pending) \
         VALUES ('pending@example.test', 'P', 'x', true)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (body, fields) in [
        (json!({ "email": "kein at" }), json!({ "email": "invalid" })),
        (
            json!({ "email": "Tom@Example.test" }),
            json!({ "email": "taken" }),
        ),
        (
            json!({ "expires_in_days": 3 }),
            json!({ "expires_in_days": "invalid" }),
        ),
        (
            json!({ "expires_in_days": 0 }),
            json!({ "expires_in_days": "invalid" }),
        ),
        (
            json!({ "note": "x".repeat(201) }),
            json!({ "note": "too_long" }),
        ),
        (
            json!({ "email": "mia@example.test", "send_mail": true }),
            json!({ "send_mail": "unavailable" }),
        ),
        (
            json!({ "send_mail": true }),
            json!({ "send_mail": "unavailable" }),
        ),
    ] {
        let res = invite(&mut lena, body.clone()).await;
        assert_eq!(
            res.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {}",
            res.body
        );
        assert_eq!(res.body["fields"], fields, "{body}");
    }
    let res = invite(
        &mut lena,
        json!({ "email": "pending@example.test", "note": "x".repeat(200) }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = lena
        .send(
            Method::GET,
            "/api/operator/invitations?status=revoked",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"], json!({ "status": "invalid" }));
    let res = lena
        .send(Method::GET, "/api/operator/invitations?offset=-1", None)
        .await;
    assert_eq!(res.body["fields"], json!({ "offset": "invalid" }));
    let actions: Vec<String> = invitation_audit(&pool)
        .await
        .into_iter()
        .map(|(action, _)| action)
        .collect();
    assert_eq!(actions, ["invitations.create"]);
}

#[sqlx::test]
async fn an_invitation_is_mailed_in_the_operators_language(pool: PgPool) {
    let lena = registered(&operator_state(pool.clone()), LENA, "Lena").await;
    sqlx::query("UPDATE users SET email_verified_at = now(), locale = 'en' WHERE email = $1")
        .bind(LENA)
        .execute(&pool)
        .await
        .unwrap();
    let mut state = operator_state(pool.clone());
    let mailer = Mailer::memory(&url::Url::parse("https://chat.example.org/").unwrap());
    state.mail = Some(mailer.clone());
    state.public_url = Some(url::Url::parse("https://chat.example.org/").unwrap());
    let mut lena = moved(&lena, &state);

    let res = invite(
        &mut lena,
        json!({ "email": "mia@example.test", "send_mail": true, "expires_in_days": 30 }),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let link = res.body["link"].as_str().unwrap().to_owned();
    let sent = outbox(&mailer, 1).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "mia@example.test");
    assert_eq!(sent[0].subject, "Your invitation to Gelabber");
    assert!(sent[0].body.contains(&link), "{}", sent[0].body);

    // Without send_mail nothing goes out.
    let res = invite(&mut lena, json!({ "email": "ole@example.test" })).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mailer.outbox().len(), 1);
}

// ---- Servers: block and delist ----

const BEN: &str = "ben@example.test";

/// [`operator_state`] with a live Redis, for media tickets and sockets.
fn ws_operator_state(pool: PgPool) -> AppState {
    let mut state = common::ws_state(pool);
    state.operators = Arc::new(HashSet::from([LENA.to_owned()]));
    state
}

/// A server of `owner`: its id, text channel, voice channel and an invite.
struct Server {
    id: String,
    text: String,
    voice: String,
    code: String,
}

async fn server_of(owner: &mut Client, name: &str) -> Server {
    let res = owner
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let channel = |kind: &str| {
        res.body["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == kind)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (text, voice) = (channel("text"), channel("voice"));
    let id = res.body["id"].as_str().unwrap().to_owned();
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{id}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    Server {
        id,
        text,
        voice,
        code: invite.body["code"].as_str().unwrap().to_owned(),
    }
}

async fn join_server(client: &mut Client, code: &str) {
    let res = client
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn post_message(client: &mut Client, channel: &str, text: &str) -> String {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": text })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

async fn report(pool: &PgPool, server: &str, reporter: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO reports (server_id, reporter_id, reason) VALUES ($1, $2, 'spam') \
         RETURNING id",
    )
    .bind(Uuid::parse_str(server).unwrap())
    .bind(Uuid::parse_str(reporter).unwrap())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn report_status(pool: &PgPool, id: Uuid) -> (String, Option<Uuid>) {
    sqlx::query_as("SELECT status, resolved_by FROM reports WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Rows of `operator_audit` about servers as (action, target_server, detail).
async fn server_audit(pool: &PgPool) -> Vec<(String, Option<Uuid>, serde_json::Value)> {
    sqlx::query_as(
        "SELECT action, target_server, detail FROM operator_audit \
         WHERE action LIKE 'servers.%' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn operate(
    op: &mut Client,
    server: &str,
    action: &str,
    body: Option<serde_json::Value>,
) -> common::Response {
    op.send(
        Method::POST,
        &format!("/api/operator/servers/{server}/{action}"),
        body,
    )
    .await
}

fn unread_of(rows: &common::Response, channel: &str) -> Option<i64> {
    rows.body
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["channel_id"] == channel)
        .map(|row| row["unread_count"].as_i64().unwrap())
}

#[sqlx::test]
async fn the_server_routes_answer_401_and_404_like_the_area(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut tom = registered(&state, TOM, "Tom").await;
    let abend = server_of(&mut tom, "Abend").await;
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let id = &abend.id;
    let routes = [
        (Method::GET, "/api/operator/servers".to_owned(), None),
        (
            Method::POST,
            format!("/api/operator/servers/{id}/delist"),
            Some(json!({})),
        ),
        (
            Method::POST,
            format!("/api/operator/servers/{id}/relist"),
            None,
        ),
        (
            Method::POST,
            format!("/api/operator/servers/{id}/block"),
            Some(json!({ "reason": "Spam" })),
        ),
        (
            Method::POST,
            format!("/api/operator/servers/{id}/unblock"),
            None,
        ),
    ];
    for (method, path, body) in routes {
        let res = anonymous.send(method.clone(), &path, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}: {}", res.body);
        // Even the owner of the server is no operator.
        let res = tom.send(method, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}: {}", res.body);
    }
    assert!(audit_rows(&pool).await.is_empty());
    let changed: bool = sqlx::query_scalar(
        "SELECT blocked_at IS NOT NULL OR delisted_at IS NOT NULL FROM servers WHERE id = $1",
    )
    .bind(Uuid::parse_str(id).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!changed);
}

#[sqlx::test]
async fn blocking_a_server_locks_its_members_out_until_unblocked(pool: PgPool) {
    let state = ws_operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let mut ben = registered(&state, BEN, "Ben").await;
    let abend = server_of(&mut tom, "Abend").await;
    let control = server_of(&mut tom, "Kontrolle").await;
    join_server(&mut ben, &abend.code).await;
    join_server(&mut ben, &control.code).await;
    post_message(&mut tom, &abend.text, "hallo").await;
    post_message(&mut tom, &control.text, "hallo").await;
    let open = report(&pool, &abend.id, ben.user_id()).await;
    let elsewhere = report(&pool, &control.id, ben.user_id()).await;
    let unread = ben.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(unread_of(&unread, &abend.text), Some(1));

    // The reason is required.
    for body in [json!({}), json!({ "reason": "  " })] {
        let res = operate(&mut lena, &abend.id, "block", Some(body)).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["reason"], "required");
    }
    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": "x".repeat(501) })),
    )
    .await;
    assert_eq!(res.body["fields"]["reason"], "too_long", "{}", res.body);
    assert!(server_audit(&pool).await.is_empty());

    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": " Betrug " })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["id"], abend.id.as_str());
    assert_eq!(res.body["blocked_reason"], "Betrug");
    assert!(res.body["blocked_at"].is_string(), "{}", res.body);
    assert_eq!(res.body["open_reports"], 0);
    assert_eq!(res.body["members"], 2);
    let lena_id = Uuid::parse_str(lena.user_id()).unwrap();
    assert_eq!(
        report_status(&pool, open).await,
        ("actioned".to_owned(), Some(lena_id))
    );
    assert_eq!(report_status(&pool, elsewhere).await.0, "open");
    let audit = server_audit(&pool).await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, "servers.block");
    assert_eq!(audit[0].1, Some(Uuid::parse_str(&abend.id).unwrap()));
    assert_eq!(audit[0].2["reason"], "Betrug");

    // Members, the owner included, get 403 server_blocked everywhere.
    for client in [&mut ben, &mut tom] {
        let res = client
            .send(Method::GET, &format!("/api/servers/{}", abend.id), None)
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
        assert_eq!(res.body["error"], "server_blocked");
        let res = client
            .send(
                Method::GET,
                &format!("/api/channels/{}/messages", abend.text),
                None,
            )
            .await;
        assert_eq!(res.body["error"], "server_blocked", "{}", res.body);
        let res = client
            .send(
                Method::POST,
                &format!("/api/channels/{}/messages", abend.text),
                Some(json!({ "content": "noch da?" })),
            )
            .await;
        assert_eq!(res.body["error"], "server_blocked", "{}", res.body);
        let res = client
            .send(
                Method::POST,
                &format!("/api/channels/{}/media-ticket", abend.voice),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
        assert_eq!(res.body["error"], "server_blocked");
        let res = client
            .send(
                Method::GET,
                &format!("/api/servers/{}/invites", abend.id),
                None,
            )
            .await;
        assert_eq!(res.body["error"], "server_blocked", "{}", res.body);
        // The other server works as before.
        let res = client
            .send(Method::GET, &format!("/api/servers/{}", control.id), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }

    // The list keeps it, marked, so the client can explain.
    let list = ben.send(Method::GET, "/api/servers", None).await;
    let blocked: Vec<(String, bool)> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_owned(),
                s["blocked"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        blocked,
        [(abend.id.clone(), true), (control.id.clone(), false)]
    );

    // Its channels drop out of unread, and read-all leaves them alone.
    let unread = ben.send(Method::GET, "/api/messages/unread", None).await;
    assert_eq!(unread_of(&unread, &abend.text), None, "{}", unread.body);
    assert_eq!(unread_of(&unread, &control.text), Some(1));
    let res = ben
        .send(Method::PUT, "/api/messages/read-all", Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let moved: Vec<&str> = res
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["channel_id"].as_str().unwrap())
        .collect();
    assert_eq!(moved, [control.text.as_str()]);
    let res = ben
        .send(
            Method::PUT,
            &format!("/api/servers/{}/read", abend.id),
            None,
        )
        .await;
    assert_eq!(res.body["error"], "server_blocked", "{}", res.body);

    // Its invites are dead, for newcomers and members alike.
    let mut mia = registered(&state, "mia@example.test", "Mia").await;
    for client in [&mut mia, &mut ben] {
        let res = client
            .send(Method::GET, &format!("/api/invites/{}", abend.code), None)
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
        assert_eq!(res.body["error"], "invite_invalid");
        let res = client
            .send(
                Method::POST,
                &format!("/api/invites/{}/join", abend.code),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    }

    // A second block changes nothing and writes no row.
    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": "Anders" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["blocked_reason"], "Betrug");
    assert_eq!(server_audit(&pool).await.len(), 1);

    // Unblocking restores access.
    let res = operate(&mut lena, &abend.id, "unblock", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body["blocked_at"].is_null(), "{}", res.body);
    assert!(res.body["blocked_reason"].is_null());
    let res = ben
        .send(Method::GET, &format!("/api/servers/{}", abend.id), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["blocked"], false);
    let res = mia
        .send(Method::GET, &format!("/api/invites/{}", abend.code), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let unread = ben.send(Method::GET, "/api/messages/unread", None).await;
    assert!(unread_of(&unread, &abend.text).is_some());
    let res = operate(&mut lena, &abend.id, "unblock", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let actions: Vec<String> = server_audit(&pool)
        .await
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(actions, ["servers.block", "servers.unblock"]);
}

#[sqlx::test]
async fn a_blocked_server_can_still_be_left_and_deleted(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let mut ben = registered(&state, BEN, "Ben").await;
    let abend = server_of(&mut tom, "Abend").await;
    join_server(&mut ben, &abend.code).await;
    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": "x" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // The owner still cannot leave; a member can.
    let res = tom
        .send(
            Method::POST,
            &format!("/api/servers/{}/leave", abend.id),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");
    let res = ben
        .send(
            Method::POST,
            &format!("/api/servers/{}/leave", abend.id),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let list = ben.send(Method::GET, "/api/servers", None).await;
    assert_eq!(list.body, json!([]));

    let res = tom
        .send(Method::DELETE, &format!("/api/servers/{}", abend.id), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = operate(&mut lena, &abend.id, "unblock", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
}

#[sqlx::test]
async fn blocking_ends_seats_and_subscriptions(pool: PgPool) {
    let state = ws_operator_state(pool.clone());
    let addr = common::serve_ws_state(state.clone()).await;
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let mut ben = registered(&state, BEN, "Ben").await;
    let abend = server_of(&mut tom, "Abend").await;
    join_server(&mut ben, &abend.code).await;
    let sid = Uuid::parse_str(&abend.id).unwrap();
    let mut ws = connect(addr, &ben).await;
    ws_send(
        &mut ws,
        json!({ "op": "s", "s": abend.id, "c": abend.text }),
    )
    .await;
    ws_until(&mut ws, |f| {
        f["op"] == "ok" && f["c"] == abend.text.as_str()
    })
    .await;
    ws_send(
        &mut ws,
        json!({ "op": "sig", "t": "j", "s": abend.id, "c": abend.voice }),
    )
    .await;
    ws_until(&mut ws, |f| f["op"] == "sig" && f["t"] == "j").await;
    assert_eq!(state.gateway.voice_snapshot(sid).await.unwrap().len(), 1);

    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": "x" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    ws_until(&mut ws, |f| {
        f["op"] == "err" && f["e"] == "server_blocked" && f["s"] == abend.id.as_str()
    })
    .await;
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());

    // Subscribing again and rejoining voice are refused alike.
    ws_send(
        &mut ws,
        json!({ "op": "s", "s": abend.id, "c": abend.text }),
    )
    .await;
    ws_until(&mut ws, |f| f["op"] == "err" && f["e"] == "server_blocked").await;
    ws_send(
        &mut ws,
        json!({ "op": "sig", "t": "j", "s": abend.id, "c": abend.voice }),
    )
    .await;
    ws_until(&mut ws, |f| f["op"] == "err" && f["e"] == "server_blocked").await;
    assert!(state.gateway.voice_snapshot(sid).await.unwrap().is_empty());

    // After unblocking the same socket subscribes again.
    let res = operate(&mut lena, &abend.id, "unblock", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    ws_send(
        &mut ws,
        json!({ "op": "s", "s": abend.id, "c": abend.text }),
    )
    .await;
    ws_until(&mut ws, |f| {
        f["op"] == "ok" && f["c"] == abend.text.as_str()
    })
    .await;
}

async fn ws_send(ws: &mut Ws, frame: serde_json::Value) {
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}

/// The first frame matching `pred`, answering heartbeats on the way.
async fn ws_until(ws: &mut Ws, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await.expect("open socket").expect("frame") {
                Message::Text(raw) => {
                    let frame: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    if frame["op"] == "h" {
                        ws_send(ws, json!({ "op": "h" })).await;
                    }
                    if pred(&frame) {
                        return frame;
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    })
    .await
    .expect("matching frame")
}

#[sqlx::test]
async fn delisting_closes_reports_and_relisting_clears_it(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let ben = registered(&state, BEN, "Ben").await;
    let abend = server_of(&mut tom, "Abend").await;
    let first = report(&pool, &abend.id, ben.user_id()).await;

    let res = operate(
        &mut lena,
        &abend.id,
        "delist",
        Some(json!({ "reason": "x".repeat(501) })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["reason"], "too_long");
    assert_eq!(report_status(&pool, first).await.0, "open");

    let res = operate(
        &mut lena,
        &abend.id,
        "delist",
        Some(json!({ "reason": "Werbung" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["directory"], "delisted");
    assert_eq!(res.body["open_reports"], 0);
    assert!(res.body["blocked_at"].is_null());
    assert_eq!(report_status(&pool, first).await.0, "actioned");
    // Members keep the server: delisting only concerns the directory.
    let res = tom
        .send(Method::GET, &format!("/api/servers/{}", abend.id), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Again with nothing to do: no row. A new report is closed by the next.
    let res = operate(&mut lena, &abend.id, "delist", Some(json!({}))).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(server_audit(&pool).await.len(), 1);
    let second = report(&pool, &abend.id, ben.user_id()).await;
    let res = operate(&mut lena, &abend.id, "delist", Some(json!({}))).await;
    assert_eq!(res.body["directory"], "delisted", "{}", res.body);
    assert_eq!(report_status(&pool, second).await.0, "actioned");

    let res = operate(&mut lena, &abend.id, "relist", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["directory"], "off");
    let res = operate(&mut lena, &abend.id, "relist", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let audit = server_audit(&pool).await;
    let actions: Vec<&str> = audit.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        actions,
        ["servers.delist", "servers.delist", "servers.relist"]
    );
    assert_eq!(audit[0].2, json!({ "reason": "Werbung", "reports": 1 }));
    assert_eq!(audit[1].2, json!({ "reason": null, "reports": 1 }));
    assert!(
        audit
            .iter()
            .all(|row| row.1 == Some(Uuid::parse_str(&abend.id).unwrap()))
    );

    // Unknown and malformed ids are 404 and write nothing.
    let unknown = Uuid::new_v4().to_string();
    for id in [unknown.as_str(), "nope"] {
        for (action, body) in [
            ("delist", Some(json!({}))),
            ("relist", None),
            ("block", Some(json!({ "reason": "x" }))),
            ("unblock", None),
        ] {
            let res = operate(&mut lena, id, action, body).await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{action}: {}", res.body);
        }
    }
    assert_eq!(server_audit(&pool).await.len(), 3);
}

#[sqlx::test]
async fn the_server_list_searches_filters_and_counts_storage(pool: PgPool) {
    let state = operator_state(pool.clone());
    let mut lena = registered(&state, LENA, "Lena").await;
    let mut tom = registered(&state, TOM, "Tom").await;
    let mut ben = registered(&state, "ben@spiele.test", "Ben").await;
    let abend = server_of(&mut tom, "Abend").await;
    let runde = server_of(&mut ben, "Runde 50%").await;
    join_server(&mut ben, &abend.code).await;
    report(&pool, &runde.id, tom.user_id()).await;

    // 1000 bytes sent with a message; 500 still pending upload.
    let message = post_message(&mut tom, &abend.text, "Bild").await;
    let channel = Uuid::parse_str(&abend.text).unwrap();
    let uploader = Uuid::parse_str(tom.user_id()).unwrap();
    for (message, size) in [
        (Some(Uuid::parse_str(&message).unwrap()), 1000_i64),
        (None, 500),
    ] {
        sqlx::query(
            "INSERT INTO attachments (message_id, channel_id, uploader_id, object_key, \
                 filename, content_type, size_bytes) \
             VALUES ($1, $2, $3, $4, 'x.png', 'image/png', $5)",
        )
        .bind(message)
        .bind(channel)
        .bind(uploader)
        .bind(format!("att/{}", Uuid::new_v4()))
        .bind(size)
        .execute(&pool)
        .await
        .unwrap();
    }

    let list = |q: &str| format!("/api/operator/servers{q}");
    let names = |res: &common::Response| -> Vec<String> {
        res.body["servers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let res = lena.send(Method::GET, &list(""), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["total"], 2);
    assert_eq!(names(&res), ["Runde 50%", "Abend"], "newest first");
    let row = &res.body["servers"][1];
    assert_eq!(row["id"], abend.id.as_str());
    assert_eq!(row["owner"]["id"], tom.user_id());
    assert_eq!(row["owner"]["name"], "Tom");
    assert_eq!(row["owner"]["email"], TOM);
    assert_eq!(row["members"], 2);
    assert_eq!(row["storage_bytes"], 1000);
    assert_eq!(row["directory"], "off");
    assert_eq!(row["open_reports"], 0);
    assert!(row["icon_url"].is_null());
    assert!(row["created_at"].is_string());
    assert!(row["blocked_at"].is_null());
    assert_eq!(res.body["servers"][0]["open_reports"], 1);
    assert_eq!(res.body["servers"][0]["storage_bytes"], 0);

    // By server name (literal %), owner name and owner address.
    for (q, expected) in [
        ("?q=50%25", vec!["Runde 50%"]),
        ("?q=%25", vec!["Runde 50%"]),
        ("?q=abe", vec!["Abend"]),
        ("?q=tom", vec!["Abend"]),
        ("?q=spiele.test", vec!["Runde 50%"]),
        ("?q=niemand", vec![]),
    ] {
        let res = lena.send(Method::GET, &list(q), None).await;
        assert_eq!(res.status, StatusCode::OK, "{q}: {}", res.body);
        assert_eq!(names(&res), expected, "{q}");
        assert_eq!(res.body["total"], expected.len() as i64, "{q}");
    }

    let res = operate(
        &mut lena,
        &abend.id,
        "block",
        Some(json!({ "reason": "x" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    for (q, expected) in [
        ("?filter=reported", vec!["Runde 50%"]),
        ("?filter=blocked", vec!["Abend"]),
        ("?filter=listed", vec![]),
        ("?filter=all&offset=1", vec!["Abend"]),
        ("?offset=2", vec![]),
    ] {
        let res = lena.send(Method::GET, &list(q), None).await;
        assert_eq!(res.status, StatusCode::OK, "{q}: {}", res.body);
        assert_eq!(names(&res), expected, "{q}");
    }
    let res = lena.send(Method::GET, &list("?offset=1"), None).await;
    assert_eq!(res.body["total"], 2);

    let long = "x".repeat(101);
    for (q, field, code) in [
        (format!("?q={long}"), "q", "too_long"),
        ("?filter=gelistet".to_owned(), "filter", "invalid"),
        ("?offset=-1".to_owned(), "offset", "invalid"),
    ] {
        let res = lena.send(Method::GET, &list(&q), None).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{q}");
        assert_eq!(res.body["fields"][field], code, "{q}");
    }

    let audit = server_audit(&pool).await;
    let lists: Vec<&serde_json::Value> = audit
        .iter()
        .filter(|row| row.0 == "servers.list")
        .map(|row| &row.2)
        .collect();
    // One row per successful list, none for the refused ones.
    assert_eq!(lists.len(), 1 + 6 + 5 + 1);
    assert_eq!(lists[0], &json!({ "q": "", "filter": "all", "offset": 0 }));
    assert_eq!(lists[1]["q"], "50%");
    assert!(
        audit
            .iter()
            .filter(|row| row.0 == "servers.list")
            .all(|row| row.1.is_none())
    );
}
