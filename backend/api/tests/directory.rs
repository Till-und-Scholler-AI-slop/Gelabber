//! The server directory "Entdecken" (#520): the switch, the status route
//! and the owner's listing form.

mod common;

use axum::http::{Method, StatusCode};
use gelabber_api::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::Client;

const PASSWORD: &str = "correct horse";

fn directory_state(pool: PgPool) -> AppState {
    let mut state = common::state(pool);
    state.directory = true;
    state
}

async fn registered(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::from_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, PASSWORD, name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn create_server(client: &mut Client, name: &str) -> Uuid {
    let res = client
        .send(Method::POST, "/api/servers", Some(json!({ "name": name })))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().parse().unwrap()
}

async fn invite_and_join(owner: &mut Client, member: &mut Client, server: Uuid) {
    let invite = owner
        .send(
            Method::POST,
            &format!("/api/servers/{server}/invites"),
            Some(json!({})),
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let code = invite.body["code"].as_str().unwrap();
    let res = member
        .send(Method::POST, &format!("/api/invites/{code}/join"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn put_listing(client: &mut Client, server: Uuid, body: Value) -> common::Response {
    client
        .send(
            Method::PUT,
            &format!("/api/servers/{server}/listing"),
            Some(body),
        )
        .await
}

async fn get_listing(client: &mut Client, server: Uuid) -> common::Response {
    client
        .send(Method::GET, &format!("/api/servers/{server}/listing"), None)
        .await
}

async fn list(client: &mut Client, server: Uuid, language: &str, description: &str) {
    let res = put_listing(
        client,
        server,
        json!({
            "listed": true,
            "description": description,
            "language": language,
            "join_mode": "open",
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// The status route's `listings` per language code.
async fn counts(client: &mut Client) -> Vec<(String, i64)> {
    let res = client.send(Method::GET, "/api/directory", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["enabled"], true);
    res.body["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["code"].as_str().unwrap().to_owned(),
                l["listings"].as_i64().unwrap(),
            )
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

fn pairs(list: &[(&str, i64)]) -> Vec<(String, i64)> {
    list.iter().map(|(c, n)| ((*c).to_owned(), *n)).collect()
}

#[sqlx::test]
async fn while_off_the_status_says_so_and_the_form_is_not_found(pool: PgPool) {
    let state = common::state(pool);
    assert!(!state.directory);
    let mut anonymous = Client::from_state(state.clone());
    anonymous.bootstrap().await;
    let res = anonymous.send(Method::GET, "/api/directory", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.body);

    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let res = ada.send(Method::GET, "/api/directory", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "enabled": false, "languages": [] }));

    let server = create_server(&mut ada, "Filmclub").await;
    let res = get_listing(&mut ada, server).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    assert_eq!(res.body["error"], "not_found");
    let res = put_listing(
        &mut ada,
        server,
        json!({ "listed": true, "description": "x", "language": "de", "join_mode": "open" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
}

#[sqlx::test]
async fn the_status_lists_every_language_with_its_endonym(pool: PgPool) {
    let state = directory_state(pool);
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let res = ada.send(Method::GET, "/api/directory", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let languages = res.body["languages"].as_array().unwrap();
    let codes: Vec<&str> = languages
        .iter()
        .map(|l| l["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["de", "en", "es", "fr", "it", "nl", "pl", "pt", "tr", "uk"]
    );
    assert_eq!(
        languages[0],
        json!({ "code": "de", "name": "Deutsch", "listings": 0 })
    );
    assert_eq!(languages[9]["name"], "Українська");
}

#[sqlx::test]
async fn the_owner_saves_the_form_and_members_read_it(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut bob = registered(&state, "bob@example.test", "Bob").await;
    let mut eve = registered(&state, "eve@example.test", "Eve").await;
    let server = create_server(&mut ada, "Filmclub").await;
    invite_and_join(&mut ada, &mut bob, server).await;

    // Before the first save: the defaults, without a language.
    let res = get_listing(&mut bob, server).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body,
        json!({
            "listed": false,
            "description": "",
            "language": null,
            "join_mode": "request",
            "delisted": false,
            "listed_at": null,
        })
    );
    // A non-member does not learn that the server exists.
    let res = get_listing(&mut eve, server).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    // Saved unlisted first: no listed_at yet.
    let res = put_listing(
        &mut ada,
        server,
        json!({
            "listed": false,
            "description": "",
            "language": "en",
            "join_mode": "request",
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["language"], "en");
    assert_eq!(res.body["listed_at"], Value::Null);

    let res = put_listing(
        &mut ada,
        server,
        json!({
            "listed": true,
            "description": "  Jeden Freitag ein Film.  ",
            "language": "de",
            "join_mode": "open",
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["listed"], true);
    assert_eq!(res.body["description"], "Jeden Freitag ein Film.");
    assert_eq!(res.body["language"], "de");
    assert_eq!(res.body["join_mode"], "open");
    assert_eq!(res.body["delisted"], false);
    let listed_at = res.body["listed_at"].clone();
    assert!(listed_at.is_string(), "{}", res.body);
    let keys: Vec<&String> = res.body.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "delisted",
            "description",
            "join_mode",
            "language",
            "listed",
            "listed_at"
        ]
    );

    // Members read what the owner saved.
    let res = get_listing(&mut bob, server).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["description"], "Jeden Freitag ein Film.");

    // Unlisting and listing again keep the first listed_at.
    for listed in [false, true] {
        let res = put_listing(
            &mut ada,
            server,
            json!({
                "listed": listed,
                "description": "Jeden Freitag ein Film.",
                "language": "de",
                "join_mode": "request",
            }),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body["listed"], listed);
        assert_eq!(res.body["listed_at"], listed_at);
    }
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM server_listings WHERE server_id = $1")
        .bind(server)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[sqlx::test]
async fn only_the_owner_saves_the_form(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut bob = registered(&state, "bob@example.test", "Bob").await;
    let mut eve = registered(&state, "eve@example.test", "Eve").await;
    let server = create_server(&mut ada, "Filmclub").await;
    invite_and_join(&mut ada, &mut bob, server).await;
    // Even a member holding every flag is not the owner.
    sqlx::query("UPDATE servers SET member_permissions = 511 WHERE id = $1")
        .bind(server)
        .execute(&pool)
        .await
        .unwrap();

    let body = json!({
        "listed": true,
        "description": "Kino",
        "language": "de",
        "join_mode": "open",
    });
    let res = put_listing(&mut bob, server, body.clone()).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "forbidden");
    let res = put_listing(&mut eve, server, body.clone()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    let res = put_listing(&mut ada, Uuid::new_v4(), body).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM server_listings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn the_form_is_validated(pool: PgPool) {
    let state = directory_state(pool);
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let server = create_server(&mut ada, "Filmclub").await;

    let cases = [
        (json!({}), "listed", "required"),
        (json!({}), "description", "required"),
        (json!({}), "language", "required"),
        (json!({}), "join_mode", "required"),
        (
            json!({ "listed": true, "description": "  ", "language": "de", "join_mode": "open" }),
            "description",
            "required",
        ),
        (
            json!({ "listed": false, "description": "x".repeat(301), "language": "de", "join_mode": "open" }),
            "description",
            "too_long",
        ),
        (
            json!({ "listed": false, "description": "zwei\nZeilen", "language": "de", "join_mode": "open" }),
            "description",
            "invalid",
        ),
        (
            json!({ "listed": false, "description": "", "language": "xx", "join_mode": "open" }),
            "language",
            "invalid",
        ),
        (
            json!({ "listed": false, "description": "", "language": "de", "join_mode": "closed" }),
            "join_mode",
            "invalid",
        ),
    ];
    for (body, field, code) in cases {
        let res = put_listing(&mut ada, server, body.clone()).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(res.body["error"], "validation_failed", "{body}");
        assert_eq!(res.body["fields"][field], code, "{body}: {}", res.body);
    }
    // 300 characters (not bytes) fit.
    let res = put_listing(
        &mut ada,
        server,
        json!({ "listed": true, "description": "ä".repeat(300), "language": "uk", "join_mode": "request" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[sqlx::test]
async fn the_status_counts_the_listings_the_caller_can_see(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut mira = registered(&state, "mira@example.test", "Mira").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let lena_id: Uuid = lena.user_id().parse().unwrap();
    let ada_id: Uuid = ada.user_id().parse().unwrap();

    let filmclub = create_server(&mut ada, "Filmclub").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    let rust = create_server(&mut mira, "Rust Study Hall").await;
    let hidden = create_server(&mut mira, "Privat").await;
    list(&mut ada, filmclub, "de", "Jeden Freitag ein Film.").await;
    list(&mut ada, keller, "de", "Wir bauen Modelle.").await;
    list(&mut mira, rust, "en", "Learning Rust together.").await;
    // Saved, but not listed.
    let res = put_listing(
        &mut mira,
        hidden,
        json!({ "listed": false, "description": "", "language": "fr", "join_mode": "open" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    let all = pairs(&[("de", 2), ("en", 1)]);
    assert_eq!(counts(&mut lena).await, all);

    // A banned person does not see the server's listing.
    sqlx::query("INSERT INTO server_bans (server_id, user_id, banned_by) VALUES ($1, $2, $3)")
        .bind(keller)
        .bind(lena_id)
        .bind(ada_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(counts(&mut lena).await, pairs(&[("de", 1), ("en", 1)]));
    assert_eq!(counts(&mut mira).await, all);

    // Delisted by the operator: hidden, and the owner's save keeps it so.
    sqlx::query("UPDATE servers SET delisted_at = now() WHERE id = $1")
        .bind(rust)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(counts(&mut mira).await, pairs(&[("de", 2)]));
    let res = put_listing(
        &mut mira,
        rust,
        json!({ "listed": true, "description": "Neu", "language": "en", "join_mode": "open" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["delisted"], true);
    assert_eq!(res.body["description"], "Neu");
    assert_eq!(get_listing(&mut mira, rust).await.body["delisted"], true);
    assert_eq!(counts(&mut mira).await, pairs(&[("de", 2)]));

    // Relisted by the operator: visible again with the owner's fields.
    sqlx::query("UPDATE servers SET delisted_at = NULL WHERE id = $1")
        .bind(rust)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(counts(&mut mira).await, all);

    // A blocked server is never listed, and its form is closed to members.
    sqlx::query("UPDATE servers SET blocked_at = now(), blocked_reason = 'x' WHERE id = $1")
        .bind(filmclub)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(counts(&mut mira).await, pairs(&[("de", 1), ("en", 1)]));
    let res = get_listing(&mut ada, filmclub).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "server_blocked");

    // Unlisting takes it out at once.
    let res = put_listing(
        &mut ada,
        keller,
        json!({ "listed": false, "description": "Wir bauen Modelle.", "language": "de", "join_mode": "open" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(counts(&mut mira).await, pairs(&[("en", 1)]));

    // A deleted server takes its listing with it.
    let res = mira
        .send(Method::DELETE, &format!("/api/servers/{rust}"), None)
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM server_listings WHERE server_id = $1")
        .bind(rust)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}
