//! Link previews (#254) against a local stand-in site: cards with title,
//! description and an image served by the API, YouTube through oEmbed, the
//! SSRF guard refusing internal targets and redirects to them, slow pages
//! not holding up sending, removal by the author and the off switch.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use gelabber_api::previews::LinkPreviews;
use gelabber_api::previews::net::Policy;
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

const VIDEO: &str = "dQw4w9WgXcQ";

/// PNG signature and an IHDR declaring 1200×630; the rest is never decoded.
fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend_from_slice(&1200u32.to_be_bytes());
    bytes.extend_from_slice(&630u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[derive(Clone)]
struct Site {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
    /// `/changing`'s title; empty answers 500.
    title: Arc<Mutex<String>>,
}

async fn handle(State(site): State<Site>, uri: Uri) -> Response {
    let path = uri.path().to_owned();
    site.seen.lock().unwrap().push(uri.to_string());
    match path.as_str() {
        "/page" => (
            [(CONTENT_TYPE, "text/html; charset=utf-8")],
            format!(
                r#"<!doctype html><html><head><title>Fallback</title>
                <meta property="og:title" content="Kaffee &amp; Kuchen">
                <meta property="og:description" content="Alles über <b>Kuchen</b>.">
                <meta property="og:site_name" content="Backstube">
                <meta property="og:image" content="{}/card.png">
                </head><body>…</body></html>"#,
                site.origin
            ),
        )
            .into_response(),
        "/card.png" | "/vi/dQw4w9WgXcQ/hqdefault.jpg" => {
            ([(CONTENT_TYPE, "image/png")], png()).into_response()
        }
        "/svg-card" => (
            [(CONTENT_TYPE, "text/html")],
            format!(
                r#"<meta property="og:title" content="SVG"><meta property="og:image" content="{}/evil.svg">"#,
                site.origin
            ),
        )
            .into_response(),
        "/evil.svg" => (
            [(CONTENT_TYPE, "image/png")],
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        )
            .into_response(),
        "/oembed" => (
            [(CONTENT_TYPE, "application/json")],
            json!({ "title": "Never Gonna Give You Up", "author_name": "Rick Astley" })
                .to_string(),
        )
            .into_response(),
        "/to-private" => (StatusCode::FOUND, [(LOCATION, "http://10.0.0.1/")]).into_response(),
        "/to-metadata" => (
            StatusCode::FOUND,
            [(LOCATION, "http://169.254.169.254/latest/meta-data/")],
        )
            .into_response(),
        "/loop" => (StatusCode::FOUND, [(LOCATION, "/loop")]).into_response(),
        "/slow" => {
            tokio::time::sleep(Duration::from_secs(20)).await;
            "late".into_response()
        }
        "/text" => ([(CONTENT_TYPE, "text/plain")], "og:title").into_response(),
        "/changing" => {
            let title = site.title.lock().unwrap().clone();
            if title.is_empty() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            (
                [(CONTENT_TYPE, "text/html")],
                format!(r#"<meta property="og:title" content="{title}">"#),
            )
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The stand-in answers as `site.test` on a local port, which only this
/// policy treats as public, and so do `other.test` and `third.test` on the
/// same address. `evil.test` resolves to a private address.
async fn serve_site() -> (Site, Policy) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let site = Site {
        origin: format!("http://site.test:{}", addr.port()),
        seen: Arc::default(),
        title: Arc::default(),
    };
    let router = Router::new().fallback(handle).with_state(site.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut policy = Policy {
        trusted: vec![addr],
        ..Policy::default()
    };
    for host in ["site.test", "other.test", "third.test"] {
        policy.hosts.insert(host.into(), vec![addr.ip()]);
    }
    policy
        .hosts
        .insert("evil.test".into(), vec!["10.1.2.3".parse().unwrap()]);
    (site, policy)
}

async fn signed_in(pool: PgPool, previews: Option<LinkPreviews>) -> (Client, String) {
    let mut state = common::state(pool);
    state.previews = previews;
    let mut client = Client::from_state(state);
    client.bootstrap().await;
    let res = client
        .register("ada@example.com", "password123", "Ada")
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let channel = res.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (client, channel)
}

async fn post(client: &mut Client, channel: &str, content: &str) -> Value {
    let res = client
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "content": content })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body
}

async fn message(client: &mut Client, channel: &str, id: &str) -> Value {
    let res = client
        .send(
            Method::GET,
            &format!("/api/channels/{channel}/messages"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .cloned()
        .expect("message")
}

/// The message once its previews are in, or after `limit` without them.
async fn wait_previews(client: &mut Client, channel: &str, id: &str, limit: Duration) -> Value {
    let start = Instant::now();
    loop {
        let message = message(client, channel, id).await;
        if message.get("previews").is_some() || start.elapsed() > limit {
            return message;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until every URL has a stored result (a card or a refusal).
async fn wait_settled(pool: &PgPool, count: i64) {
    let start = Instant::now();
    loop {
        let done: i64 = sqlx::query_scalar("SELECT count(*) FROM link_previews")
            .fetch_one(pool)
            .await
            .unwrap();
        if done >= count {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{done}/{count} settled"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[sqlx::test]
async fn a_link_gets_a_card_with_an_image_from_the_api(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) = signed_in(pool, Some(LinkPreviews::with_policy(policy))).await;
    let url = format!("{}/page", site.origin);
    let sent = post(&mut client, &channel, &format!("schau: {url} **lecker**")).await;
    assert!(sent.get("previews").is_none(), "sending does not wait");

    let id = sent["id"].as_str().unwrap();
    let message = wait_previews(&mut client, &channel, id, Duration::from_secs(10)).await;
    let previews = message["previews"].as_array().expect("previews");
    assert_eq!(previews.len(), 1);
    let card = &previews[0];
    assert_eq!(card["url"], url);
    assert_eq!(card["title"], "Kaffee & Kuchen");
    assert_eq!(card["description"], "Alles über <b>Kuchen</b>.");
    assert_eq!(card["site_name"], "Backstube");
    assert_eq!(card["image"]["width"], 1200);
    assert_eq!(card["image"]["height"], 630);
    assert!(card.get("youtube_id").is_none());

    let image = card["image"]["url"].as_str().unwrap();
    assert!(image.starts_with("/api/previews/"), "{image}");
    let (status, headers, bytes) = client.send_raw(Method::GET, image, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[CONTENT_TYPE], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(bytes, png());

    // A second message with the same link uses the stored card.
    let fetched = site.seen.lock().unwrap().len();
    let again = post(&mut client, &channel, &url).await;
    let again = wait_previews(
        &mut client,
        &channel,
        again["id"].as_str().unwrap(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(again["previews"][0]["title"], "Kaffee & Kuchen");
    assert_eq!(site.seen.lock().unwrap().len(), fetched);
}

/// A refetch is a new row: messages keep the card they got, and a failed
/// refetch leaves the last good card for new messages.
#[sqlx::test]
async fn old_messages_keep_their_card_when_the_page_changes(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) =
        signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let url = format!("{}/changing", site.origin);
    let card_of = async |client: &mut Client, title: &str| {
        *site.title.lock().unwrap() = title.to_owned();
        sqlx::query("UPDATE link_previews SET fetched_at = fetched_at - interval '2 days'")
            .execute(&pool)
            .await
            .unwrap();
        let sent = post(client, &channel, &url).await;
        let id = sent["id"].as_str().unwrap().to_owned();
        let message = wait_previews(client, &channel, &id, Duration::from_secs(10)).await;
        (id, message["previews"][0]["title"].clone())
    };
    let (first, title) = card_of(&mut client, "Eins").await;
    assert_eq!(title, "Eins");
    let (second, title) = card_of(&mut client, "Zwei").await;
    assert_eq!(title, "Zwei");
    let (_, title) = card_of(&mut client, "").await;
    assert_eq!(title, "Zwei", "a failed refetch keeps the last card");

    assert_eq!(
        message(&mut client, &channel, &first).await["previews"][0]["title"],
        "Eins"
    );
    assert_eq!(
        message(&mut client, &channel, &second).await["previews"][0]["title"],
        "Zwei"
    );
}

#[sqlx::test]
async fn youtube_links_use_oembed_and_the_thumbnail(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let previews = LinkPreviews::with_policy(policy)
        .with_youtube(&format!("{}/oembed", site.origin), &site.origin);
    let (mut client, channel) = signed_in(pool, Some(previews)).await;
    let sent = post(&mut client, &channel, &format!("https://youtu.be/{VIDEO}")).await;
    let message = wait_previews(
        &mut client,
        &channel,
        sent["id"].as_str().unwrap(),
        Duration::from_secs(10),
    )
    .await;
    let card = &message["previews"][0];
    assert_eq!(card["youtube_id"], VIDEO);
    assert_eq!(card["title"], "Never Gonna Give You Up");
    assert_eq!(card["description"], "Rick Astley");
    assert_eq!(card["site_name"], "YouTube");
    assert!(card["image"]["url"].as_str().is_some());
    let seen = site.seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|s| s.starts_with("/oembed?format=json&url=")),
        "{seen:?}"
    );
}

#[sqlx::test]
async fn internal_targets_and_redirects_to_them_are_refused(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) =
        signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let o = &site.origin;
    let targets = [
        "http://127.0.0.1/".to_owned(),
        "http://10.0.0.1/".to_owned(),
        "http://169.254.169.254/latest/meta-data/".to_owned(),
        "http://[::1]/".to_owned(),
        "http://evil.test/".to_owned(),
        "http://localhost/".to_owned(),
        format!("{o}/to-private"),
        format!("{o}/to-metadata"),
        format!("{o}/loop"),
        format!("{o}/text"),
        format!("{o}/svg-card"),
    ];
    let mut ids = Vec::new();
    for chunk in targets.chunks(3) {
        let sent = post(&mut client, &channel, &chunk.join(" ")).await;
        ids.push(sent["id"].as_str().unwrap().to_owned());
    }
    wait_settled(&pool, targets.len() as i64).await;
    // Only the SVG page has something to show, and its "image" is refused.
    let ok: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT url, image_key FROM link_previews WHERE status = 'ok'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(ok, [(format!("{o}/svg-card"), None)]);
    // A redirect loop ends after three hops.
    let seen = site.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().filter(|s| *s == "/loop").count(), 4, "{seen:?}");
}

#[sqlx::test]
async fn a_slow_page_does_not_hold_up_sending(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) =
        signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let start = Instant::now();
    post(&mut client, &channel, &format!("{}/slow", site.origin)).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    wait_settled(&pool, 1).await;
    let status: String = sqlx::query_scalar("SELECT status FROM link_previews")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "failed");
    assert!(
        start.elapsed() < Duration::from_secs(12),
        "{:?}",
        start.elapsed()
    );
}

/// Only the host that timed out is skipped afterwards: not other names on
/// its address (a CDN), and not links that were merely left when the
/// message's budget ran out.
#[sqlx::test]
async fn a_timeout_holds_up_only_its_own_host(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) =
        signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let port = site.origin.rsplit(':').next().unwrap();
    let on = |host: &str, path: &str| format!("http://{host}:{port}{path}");
    // The first link times out on its own, the second runs into the end of
    // the budget, the third is never tried.
    let start = Instant::now();
    post(
        &mut client,
        &channel,
        &format!(
            "{} {} {}",
            on("site.test", "/slow"),
            on("other.test", "/slow"),
            on("third.test", "/text")
        ),
    )
    .await;
    wait_settled(&pool, 1).await;
    // Past the budget (8 s) with some room for the worker to finish.
    tokio::time::sleep(Duration::from_secs(9).saturating_sub(start.elapsed())).await;

    let card = async |client: &mut Client, url: String| {
        let sent = post(client, &channel, &url).await;
        let id = sent["id"].as_str().unwrap().to_owned();
        wait_previews(client, &channel, &id, Duration::from_secs(10)).await["previews"].clone()
    };
    assert_eq!(
        card(&mut client, on("other.test", "/page")).await[0]["title"],
        "Kaffee & Kuchen"
    );
    assert_eq!(
        card(&mut client, on("third.test", "/page")).await[0]["title"],
        "Kaffee & Kuchen"
    );
    let seen = site.seen.lock().unwrap().len();
    let held = post(&mut client, &channel, &on("site.test", "/page")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        message_has_no_previews(&mut client, &channel, held["id"].as_str().unwrap()).await,
        "site.test is in backoff"
    );
    assert_eq!(
        site.seen.lock().unwrap().len(),
        seen,
        "site.test was not asked"
    );
}

#[sqlx::test]
async fn the_author_removes_previews_and_edits_keep_them_off(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut client, channel) =
        signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let url = format!("{}/page", site.origin);
    let sent = post(&mut client, &channel, &url).await;
    let id = sent["id"].as_str().unwrap();
    let message = wait_previews(&mut client, &channel, id, Duration::from_secs(10)).await;
    assert!(message.get("previews").is_some());

    // An edit that drops the link drops its card at once.
    let res = client
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({ "content": "kein Link mehr" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(res.body.get("previews").is_none(), "{}", res.body);

    let res = client
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({ "content": url })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let message = wait_previews(&mut client, &channel, id, Duration::from_secs(10)).await;
    assert!(message.get("previews").is_some());

    let res = client
        .send(
            Method::DELETE,
            &format!("/api/messages/{id}/previews"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(message_has_no_previews(&mut client, &channel, id).await);

    let res = client
        .send(
            Method::PATCH,
            &format!("/api/messages/{id}"),
            Some(json!({ "content": format!("{url} nochmal") })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(message_has_no_previews(&mut client, &channel, id).await);
}

async fn message_has_no_previews(client: &mut Client, channel: &str, id: &str) -> bool {
    message(client, channel, id).await.get("previews").is_none()
}

#[sqlx::test]
async fn strangers_cannot_remove_previews(pool: PgPool) {
    let (site, policy) = serve_site().await;
    let (mut ada, channel) = signed_in(pool.clone(), Some(LinkPreviews::with_policy(policy))).await;
    let sent = post(&mut ada, &channel, &format!("{}/page", site.origin)).await;
    let id = sent["id"].as_str().unwrap();

    let mut bob = Client::new(pool);
    bob.bootstrap().await;
    let res = bob.register("bob@example.com", "password123", "Bob").await;
    assert_eq!(res.status, StatusCode::CREATED);
    let res = bob
        .send(
            Method::DELETE,
            &format!("/api/messages/{id}/previews"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "not a member");
}

#[sqlx::test]
async fn with_previews_off_links_stay_plain(pool: PgPool) {
    let (site, _) = serve_site().await;
    let (mut client, channel) = signed_in(pool.clone(), None).await;
    let url = format!("{}/page", site.origin);
    let sent = post(&mut client, &channel, &url).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let message = message(&mut client, &channel, sent["id"].as_str().unwrap()).await;
    assert_eq!(message["content"], url);
    assert!(message.get("previews").is_none());
    assert!(site.seen.lock().unwrap().is_empty());
}

#[test]
fn previews_are_on_by_default_and_off_by_switch() {
    let config = |off: Option<&str>| {
        gelabber_api::Config::from_source(|key| match key {
            "DATABASE_URL" => Some("postgres://u:p@db.example.org:5432/x".into()),
            "REDIS_URL" => Some("redis://cache.example.org:6379".into()),
            "TURN_URLS" => Some(
                "turn:turn.example.org:3478?transport=udp,turns:[2001:db8::1]:5349,\
                 turn://turn2.example.org:3478"
                    .into(),
            ),
            "API_LINK_PREVIEW_DENY_HOSTS" => Some(" Intranet.Example.org ,".into()),
            "API_LINK_PREVIEWS" => off.map(str::to_owned),
            _ => None,
        })
        .unwrap()
    };
    let on = config(None).previews.expect("on by default");
    assert_eq!(
        on.deny_hosts,
        [
            "db.example.org",
            "cache.example.org",
            "turn.example.org",
            "2001:db8::1",
            "turn2.example.org",
            "intranet.example.org"
        ]
    );
    assert!(!on.deny_hosts.iter().any(|host| host.starts_with("//")));
    assert!(config(Some("false")).previews.is_none());
}
