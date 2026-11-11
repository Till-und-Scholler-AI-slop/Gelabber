//! The server directory "Entdecken" (#520): the switch, the status route,
//! the owner's listing form, the search, joining and requests to join.

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

async fn list_with(client: &mut Client, server: Uuid, language: &str, join_mode: &str) {
    let res = put_listing(
        client,
        server,
        json!({
            "listed": true,
            "description": "Kommt vorbei.",
            "language": language,
            "join_mode": join_mode,
        }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn search(client: &mut Client, query: &str) -> common::Response {
    client
        .send(
            Method::GET,
            &format!("/api/directory/listings{query}"),
            None,
        )
        .await
}

/// The names of the search's page, in order.
async fn names(client: &mut Client, query: &str) -> Vec<String> {
    let res = search(client, query).await;
    assert_eq!(res.status, StatusCode::OK, "{query}: {}", res.body);
    res.body["listings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The listing of `server` in the caller's unfiltered search.
async fn listing_of(client: &mut Client, server: Uuid) -> Option<Value> {
    let res = search(client, "").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["listings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["server_id"] == server.to_string())
        .cloned()
}

async fn join(client: &mut Client, server: Uuid) -> common::Response {
    client
        .send(
            Method::POST,
            &format!("/api/directory/listings/{server}/join"),
            None,
        )
        .await
}

async fn ask(client: &mut Client, server: Uuid, message: &str) -> common::Response {
    client
        .send(
            Method::POST,
            &format!("/api/directory/listings/{server}/request"),
            Some(json!({ "message": message })),
        )
        .await
}

async fn withdraw(client: &mut Client, server: Uuid) -> common::Response {
    client
        .send(
            Method::DELETE,
            &format!("/api/directory/listings/{server}/request"),
            None,
        )
        .await
}

async fn request_rows(pool: &PgPool, server: Uuid) -> Vec<(Uuid, String)> {
    sqlx::query_as(
        "SELECT user_id, status FROM server_join_requests WHERE server_id = $1 ORDER BY user_id",
    )
    .bind(server)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn is_member(pool: &PgPool, server: Uuid, user: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(server)
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn id_of(client: &Client) -> Uuid {
    client.user_id().parse().unwrap()
}

#[sqlx::test]
async fn while_off_the_listing_routes_are_not_found(pool: PgPool) {
    let state = common::state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let server = create_server(&mut ada, "Filmclub").await;
    // Listed while the directory was on, then switched off.
    sqlx::query(
        "INSERT INTO server_listings (server_id, listed, description, language, join_mode, listed_at) \
         VALUES ($1, true, 'Kino', 'de', 'open', now())",
    )
    .bind(server)
    .execute(&pool)
    .await
    .unwrap();

    for res in [
        search(&mut lena, "").await,
        join(&mut lena, server).await,
        ask(&mut lena, server, "").await,
        withdraw(&mut lena, server).await,
    ] {
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
        assert_eq!(res.body["error"], "not_found");
    }
    assert!(!is_member(&pool, server, id_of(&lena)).await);
}

#[sqlx::test]
async fn the_search_shows_only_visible_listings_and_nothing_more(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut bob = registered(&state, "bob@example.test", "Bob").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let filmclub = create_server(&mut ada, "Filmclub").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    let privat = create_server(&mut ada, "Privat").await;
    invite_and_join(&mut ada, &mut bob, filmclub).await;
    list(&mut ada, filmclub, "de", "Jeden Freitag ein Film.").await;
    list_with(&mut ada, keller, "de", "request").await;
    let res = put_listing(
        &mut ada,
        privat,
        json!({ "listed": false, "description": "", "language": "de", "join_mode": "open" }),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Biggest first; the unlisted one is missing.
    let res = search(&mut lena, "").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["next_offset"], Value::Null);
    let listings = res.body["listings"].as_array().unwrap();
    assert_eq!(listings.len(), 2, "{}", res.body);
    assert_eq!(
        listings[0],
        json!({
            "server_id": filmclub.to_string(),
            "name": "Filmclub",
            "icon_url": null,
            "description": "Jeden Freitag ein Film.",
            "language": "de",
            "member_count": 2,
            "join_mode": "open",
            "member": false,
            "request": null,
        })
    );
    assert_eq!(listings[1]["name"], "Modellbau-Keller");
    assert_eq!(listings[1]["join_mode"], "request");
    // A member sees the listing as one.
    assert_eq!(
        listing_of(&mut bob, filmclub).await.unwrap()["member"],
        true
    );
    // Seeing the listing does not open the server itself.
    let res = lena
        .send(Method::GET, &format!("/api/servers/{filmclub}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    // Banned, delisted and blocked servers are out, and every listing route
    // answers 404 for them.
    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{filmclub}/ban"),
            Some(json!({ "user_id": bob.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(listing_of(&mut bob, filmclub).await.is_none());
    assert!(listing_of(&mut lena, filmclub).await.is_some());
    for res in [
        join(&mut bob, filmclub).await,
        ask(&mut bob, filmclub, "").await,
    ] {
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    }
    assert!(!is_member(&pool, filmclub, id_of(&bob)).await);

    sqlx::query("UPDATE servers SET delisted_at = now() WHERE id = $1")
        .bind(keller)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE servers SET blocked_at = now(), blocked_reason = 'x' WHERE id = $1")
        .bind(filmclub)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(names(&mut lena, "").await, Vec::<String>::new());
    for server in [filmclub, keller, privat, Uuid::new_v4()] {
        let res = join(&mut lena, server).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
        let res = ask(&mut lena, server, "").await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);
    }
    assert!(request_rows(&pool, keller).await.is_empty());
}

#[sqlx::test]
async fn the_search_matches_name_and_description_literally(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let filmclub = create_server(&mut ada, "Filmclub").await;
    let rust = create_server(&mut ada, "Rust Study Hall").await;
    let percent = create_server(&mut ada, "100% Kaffee").await;
    let under = create_server(&mut ada, "a_b").await;
    list(&mut ada, filmclub, "de", "Jeden Freitag ein Film.").await;
    list(&mut ada, rust, "en", "Learning RUST together.").await;
    list(&mut ada, percent, "de", "Bohnen").await;
    list(&mut ada, under, "de", "Unterstrich").await;

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(names(&mut lena, "?q=FILM").await, ["Filmclub"]);
    // Name and description both match, either case.
    assert_eq!(names(&mut lena, "?q=rust").await, ["Rust Study Hall"]);
    assert_eq!(names(&mut lena, "?q=freitag").await, ["Filmclub"]);
    // % and _ are no wildcards.
    assert_eq!(names(&mut lena, "?q=%25").await, ["100% Kaffee"]);
    assert_eq!(names(&mut lena, "?q=_").await, ["a_b"]);
    assert_eq!(names(&mut lena, "?q=a%25b").await, Vec::<String>::new());
    // Blank is no search; trimmed.
    assert_eq!(names(&mut lena, "?q=%20%20").await.len(), 4);
    assert_eq!(names(&mut lena, "?q=%20film%20").await, ["Filmclub"]);

    assert_eq!(
        sorted(names(&mut lena, "?language=de").await),
        ["100% Kaffee", "Filmclub", "a_b"]
    );
    assert_eq!(names(&mut lena, "?language=en").await, ["Rust Study Hall"]);
    assert_eq!(names(&mut lena, "?language=fr").await, Vec::<String>::new());
    assert_eq!(
        names(&mut lena, "?language=en&q=film").await,
        Vec::<String>::new()
    );
    assert_eq!(names(&mut lena, "?language=").await.len(), 4);

    for (query, field, code) in [
        ("?language=xx", "language", "invalid"),
        ("?offset=15", "offset", "invalid"),
        ("?offset=-30", "offset", "invalid"),
        ("?offset=1020", "offset", "invalid"),
        ("?offset=x", "offset", "invalid"),
        ("?q=a%0Ab", "q", "invalid"),
    ] {
        let res = search(&mut lena, query).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
        assert_eq!(res.body["fields"][field], code, "{query}: {}", res.body);
    }
    let long = "q".repeat(101);
    let res = search(&mut lena, &format!("?q={long}")).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["fields"]["q"], "too_long");
}

#[sqlx::test]
async fn the_search_pages_thirty_at_a_time(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let big = create_server(&mut ada, "Zuerst").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    invite_and_join(&mut ada, &mut tom, big).await;
    list(&mut ada, big, "de", "Am meisten Mitglieder.").await;
    // 31 more without members, listed one second apart.
    sqlx::query(
        "WITH s AS (INSERT INTO servers (name, owner_id) \
                    SELECT 'Bulk ' || lpad(g::text, 2, '0'), $1 FROM generate_series(1, 31) g \
                    RETURNING id, name) \
         INSERT INTO server_listings (server_id, listed, description, language, join_mode, listed_at) \
         SELECT id, true, 'Viele', 'en', 'open', \
                now() + make_interval(secs => substr(name, 6)::int) FROM s",
    )
    .bind(id_of(&ada))
    .execute(&pool)
    .await
    .unwrap();

    let res = search(&mut lena, "").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let first: Vec<&str> = res.body["listings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(first.len(), 30);
    // Most members first, then the oldest listing.
    assert_eq!(first[0], "Zuerst");
    assert_eq!(first[1], "Bulk 01");
    assert_eq!(first[29], "Bulk 29");
    assert_eq!(res.body["next_offset"], 30);

    let res = search(&mut lena, "?offset=30").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let rest: Vec<&str> = res.body["listings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(rest, ["Bulk 30", "Bulk 31"]);
    assert_eq!(res.body["next_offset"], Value::Null);

    // Filtered searches page the same way.
    let res = search(&mut lena, "?q=bulk&offset=30").await;
    assert_eq!(res.body["listings"].as_array().unwrap().len(), 1);
    let res = search(&mut lena, "?language=en").await;
    assert_eq!(res.body["next_offset"], 30);
    let res = search(&mut lena, "?offset=990").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["listings"], json!([]));
    assert_eq!(res.body["next_offset"], Value::Null);
}

#[sqlx::test]
async fn an_open_server_is_joined_at_once(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let filmclub = create_server(&mut ada, "Filmclub").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list(&mut ada, filmclub, "de", "Kino").await;
    list_with(&mut ada, keller, "de", "request").await;

    let res = join(&mut lena, filmclub).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["id"], filmclub.to_string());
    assert_eq!(res.body["name"], "Filmclub");
    assert_eq!(res.body["role"], "member");
    assert!(is_member(&pool, filmclub, id_of(&lena)).await);
    // Idempotent.
    let res = join(&mut lena, filmclub).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["id"], filmclub.to_string());
    let listing = listing_of(&mut lena, filmclub).await.unwrap();
    assert_eq!(listing["member"], true);
    assert_eq!(listing["member_count"], 2);
    let res = lena
        .send(Method::GET, &format!("/api/servers/{filmclub}"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // Request mode takes no direct joins.
    let res = join(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "join_request_required");
    assert!(!is_member(&pool, keller, id_of(&lena)).await);

    // A kicked person joins again; a request left from before goes away.
    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{filmclub}/kick"),
            Some(json!({ "user_id": lena.user_id() })),
        )
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    let res = ask(&mut lena, filmclub, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = join(&mut lena, filmclub).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(request_rows(&pool, filmclub).await.is_empty());
}

#[sqlx::test]
async fn requests_are_filed_repeated_and_withdrawn(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list_with(&mut ada, keller, "de", "request").await;
    let lena_id = id_of(&lena);

    let res = ask(&mut lena, keller, "  Ich baue Schiffe.  ").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body, json!({ "request": "pending" }));
    // Again: 200, and the first message stays.
    let res = ask(&mut lena, keller, "Anders").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "request": "pending" }));
    let message: String = sqlx::query_scalar(
        "SELECT message FROM server_join_requests WHERE server_id = $1 AND user_id = $2",
    )
    .bind(keller)
    .bind(lena_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(message, "Ich baue Schiffe.");
    assert_eq!(
        listing_of(&mut lena, keller).await.unwrap()["request"],
        "pending"
    );

    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(request_rows(&pool, keller).await.is_empty());
    assert_eq!(
        listing_of(&mut lena, keller).await.unwrap()["request"],
        Value::Null
    );
    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    // An open server takes a request too; a missing body message is empty.
    let filmclub = create_server(&mut ada, "Filmclub").await;
    list(&mut ada, filmclub, "de", "Kino").await;
    let res = lena
        .send(
            Method::POST,
            &format!("/api/directory/listings/{filmclub}/request"),
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    // Withdrawing still works after the operator delisted the server.
    let res = ask(&mut lena, keller, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    sqlx::query("UPDATE servers SET delisted_at = now() WHERE id = $1")
        .bind(keller)
        .execute(&pool)
        .await
        .unwrap();
    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);

    // The message is one line of at most 300 characters.
    for (message, code) in [
        ("x".repeat(301), "too_long"),
        ("a\nb".to_owned(), "invalid"),
    ] {
        let res = ask(&mut lena, filmclub, &message).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["fields"]["message"], code);
    }

    // A member cannot ask.
    let res = ask(&mut ada, filmclub, "").await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "already_member");
}

#[sqlx::test]
async fn a_declined_request_waits_seven_days(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list_with(&mut ada, keller, "de", "request").await;
    let res = ask(&mut lena, keller, "Erstes Mal").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    sqlx::query(
        "UPDATE server_join_requests SET status = 'declined', decided_at = now(), decided_by = $2 \
         WHERE server_id = $1",
    )
    .bind(keller)
    .bind(id_of(&ada))
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(
        listing_of(&mut lena, keller).await.unwrap()["request"],
        "declined"
    );
    let res = ask(&mut lena, keller, "Zweites Mal").await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "join_request_declined");
    let retry_after = res.body["retry_after"].as_u64().unwrap();
    assert!(
        (7 * 86_400 - 60..=7 * 86_400).contains(&retry_after),
        "{retry_after}"
    );
    // A declined request is no pending one to withdraw.
    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.body);

    // Once the seven days are over, a new request replaces the row.
    sqlx::query(
        "UPDATE server_join_requests SET decided_at = now() - interval '7 days 1 second' \
         WHERE server_id = $1",
    )
    .bind(keller)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        listing_of(&mut lena, keller).await.unwrap()["request"],
        Value::Null
    );
    let res = ask(&mut lena, keller, "Zweites Mal").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let row: (String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT status, message, decided_by FROM server_join_requests WHERE server_id = $1",
    )
    .bind(keller)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, ("pending".to_owned(), "Zweites Mal".to_owned(), None));
}

#[sqlx::test]
async fn pending_requests_are_capped_per_person_and_per_server(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list_with(&mut ada, keller, "de", "request").await;

    // Lena has 10 pending requests elsewhere (a declined one does not count).
    sqlx::query(
        "WITH s AS (INSERT INTO servers (name, owner_id) \
                    SELECT 'Andere ' || g, $1 FROM generate_series(1, 11) g RETURNING id) \
         INSERT INTO server_join_requests (server_id, user_id, status, decided_at) \
         SELECT id, $2, CASE WHEN row_number() OVER () = 1 THEN 'declined' ELSE 'pending' END, \
                CASE WHEN row_number() OVER () = 1 THEN now() END FROM s",
    )
    .bind(id_of(&ada))
    .bind(id_of(&lena))
    .execute(&pool)
    .await
    .unwrap();
    let res = ask(&mut lena, keller, "").await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "join_requests_full");
    sqlx::query(
        "DELETE FROM server_join_requests WHERE user_id = $1 AND server_id = \
         (SELECT server_id FROM server_join_requests WHERE user_id = $1 AND status = 'pending' LIMIT 1)",
    )
    .bind(id_of(&lena))
    .execute(&pool)
    .await
    .unwrap();
    let res = ask(&mut lena, keller, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    // The server holds 100 pending requests, Lena's included.
    sqlx::query(
        "WITH u AS (INSERT INTO users (email, name, password_hash) \
                    SELECT 'bulk' || g || '@example.test', 'Bulk ' || g, 'x' \
                    FROM generate_series(1, 99) g RETURNING id) \
         INSERT INTO server_join_requests (server_id, user_id) SELECT $1, id FROM u",
    )
    .bind(keller)
    .execute(&pool)
    .await
    .unwrap();
    let res = ask(&mut tom, keller, "").await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "join_requests_full");
    // Lena's own pending request still answers 200.
    let res = ask(&mut lena, keller, "").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = ask(&mut tom, keller, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[sqlx::test]
async fn joins_and_requests_share_ten_per_hour(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let filmclub = create_server(&mut ada, "Filmclub").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list(&mut ada, filmclub, "de", "Kino").await;
    list_with(&mut ada, keller, "de", "request").await;

    for _ in 0..5 {
        let res = join(&mut lena, filmclub).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        let res = ask(&mut lena, keller, "").await;
        assert!(res.status.is_success(), "{}", res.body);
    }
    for res in [
        join(&mut lena, filmclub).await,
        ask(&mut lena, keller, "").await,
    ] {
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.body);
        assert_eq!(res.body["error"], "rate_limited");
        assert!(res.body["retry_after"].as_u64().unwrap() > 0);
    }
    // Per person: Tom still gets in, and withdrawing is not limited.
    let res = join(&mut tom, filmclub).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = withdraw(&mut lena, keller).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
}

#[sqlx::test]
async fn an_invite_join_and_a_ban_remove_the_request(pool: PgPool) {
    let state = directory_state(pool.clone());
    let mut ada = registered(&state, "ada@example.test", "Ada").await;
    let mut lena = registered(&state, "lena@example.test", "Lena").await;
    let mut tom = registered(&state, "tom@example.test", "Tom").await;
    let keller = create_server(&mut ada, "Modellbau-Keller").await;
    list_with(&mut ada, keller, "de", "request").await;

    let res = ask(&mut lena, keller, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = ask(&mut tom, keller, "").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);

    invite_and_join(&mut ada, &mut lena, keller).await;
    assert_eq!(
        request_rows(&pool, keller).await,
        [(id_of(&tom), "pending".to_owned())]
    );
    assert_eq!(listing_of(&mut lena, keller).await.unwrap()["member"], true);

    let res = ada
        .send(
            Method::POST,
            &format!("/api/servers/{keller}/ban"),
            Some(json!({ "user_id": tom.user_id() })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert!(request_rows(&pool, keller).await.is_empty());
}
