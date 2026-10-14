//! GIF search and sending through the API's Klipy relay (v0.7), against a
//! local Klipy stand-in: the key stays on the server, media only from the
//! allowlisted origin, GIFs as ordinary messages in channels and DMs.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH, LOCATION};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use gelabber_api::config::GifConfig;
use gelabber_api::{AppState, Config};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::Client;

const KEY: &str = "test-klipy-key";
const WEBP: &[u8] = b"RIFF\x10\x00\x00\x00WEBPVP8 fake";

#[derive(Clone)]
struct Klipy {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Klipy {
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn item(&self, slug: &str, title: &str) -> Value {
        let media = |name: &str, ext: &str, w: u32| json!({ "url": format!("{}/media/{slug}-{name}.{ext}", self.origin), "width": w, "height": w / 2, "size": 10 });
        json!({
            "id": 8041071659142944_u64,
            "slug": slug,
            "title": title,
            "type": "gif",
            "file": {
                "hd": { "webp": media("hd", "webp", 480), "jpg": media("hd", "jpg", 480) },
                "sm": { "webp": media("sm", "webp", 220), "jpg": media("sm", "jpg", 220) },
            },
        })
    }
}

async fn feed(
    State(klipy): State<Klipy>,
    Path((key, kind)): Path<(String, String)>,
    Query(query): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> Response {
    let q = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    klipy
        .seen
        .lock()
        .unwrap()
        .push(format!("GET {key} {kind} {q}"));
    let _ = headers;
    let title = query
        .iter()
        .find(|(k, _)| k == "q")
        .map_or("Trending", |(_, v)| v.as_str());
    match kind.as_str() {
        "trending" | "search" => axum::Json(json!({
            "result": true,
            "data": {
                "data": [{ "type": "ad", "content": "<div>ad</div>" }, klipy.item("hello-hi-662", title)],
                "current_page": 1, "per_page": 24, "has_next": true
            }
        }))
        .into_response(),
        "items" => {
            let slugs = query
                .iter()
                .find(|(k, _)| k == "slugs")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let data: Vec<Value> = if slugs == "hello-hi-662" {
                vec![klipy.item("hello-hi-662", "Hello")]
            } else {
                vec![]
            };
            axum::Json(json!({ "result": true, "data": { "data": data } })).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn share(
    State(klipy): State<Klipy>,
    Path((key, slug)): Path<(String, String)>,
    body: String,
) -> axum::Json<Value> {
    klipy
        .seen
        .lock()
        .unwrap()
        .push(format!("SHARE {key} {slug} {body}"));
    axum::Json(json!({ "result": true }))
}

async fn media(Path(name): Path<String>, headers: HeaderMap) -> Response {
    match name.as_str() {
        "redirect.webp" => {
            (StatusCode::FOUND, [(LOCATION, "http://127.0.0.1:1/x")]).into_response()
        }
        "page.webp" => ([(CONTENT_TYPE, "text/html")], "<script>").into_response(),
        "gone.webp" => StatusCode::GONE.into_response(),
        _ if headers.get(IF_NONE_MATCH).is_some_and(|v| v == "\"v1\"") => {
            (StatusCode::NOT_MODIFIED, [(ETAG, "\"v1\"")]).into_response()
        }
        _ => ([(CONTENT_TYPE, "image/webp"), (ETAG, "\"v1\"")], WEBP).into_response(),
    }
}

async fn klipy() -> Klipy {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = Klipy {
        origin: format!("http://{}", listener.local_addr().unwrap()),
        seen: Arc::default(),
    };
    let router = Router::new()
        .route("/api/v1/{key}/gifs/{kind}", get(feed))
        .route("/api/v1/{key}/gifs/share/{slug}", post(share))
        .route("/media/{name}", get(media))
        .with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state
}

fn app_state(pool: PgPool, klipy: Option<&Klipy>, extra: &[(&str, &str)]) -> AppState {
    let extra: Vec<(String, String)> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let mut config = Config::from_source(|key| {
        if let Some((_, value)) = extra.iter().find(|(k, _)| k == key) {
            return Some(value.clone());
        }
        match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
            "REDIS_URL" => Some(common::redis_url()),
            _ => None,
        }
    })
    .expect("test config");
    config.gifs = klipy.map(|klipy| GifConfig {
        api_key: KEY.to_owned(),
        api_base: klipy.origin.clone(),
        media_origins: vec![klipy.origin.clone()],
        content_filter: "medium".to_owned(),
    });
    AppState::with_pool(&config, pool).expect("state")
}

async fn user(state: &AppState, email: &str, name: &str) -> Client {
    let mut client = Client::with_state(state.clone());
    client.bootstrap().await;
    let res = client.register(email, "password123", name).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    client
}

async fn text_channel(client: &mut Client) -> String {
    let res = client
        .send(
            Method::POST,
            "/api/servers",
            Some(json!({ "name": "Team" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn media_path(url: &str) -> String {
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    encoded.append_pair("url", url);
    format!("/api/gifs/media?{}", encoded.finish())
}

#[sqlx::test]
async fn without_a_key_gifs_are_off_and_nothing_is_relayed(pool: PgPool) {
    let state = app_state(pool, None, &[]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;
    let res = ada.send(Method::GET, "/api/gifs", None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, json!({ "enabled": false, "provider": null }));
    for path in [
        "/api/gifs/trending",
        "/api/gifs/search?q=hi",
        "/api/gifs/media?url=https%3A%2F%2Fstatic.klipy.com%2Fa.webp",
    ] {
        let res = ada.send(Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    let channel = text_channel(&mut ada).await;
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{channel}/messages"),
            Some(json!({ "gif": { "slug": "hello-hi-662" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["gif"], "unavailable");
}

#[sqlx::test]
async fn routes_require_a_session(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[]);
    let mut anonymous = Client::with_state(state);
    anonymous.bootstrap().await;
    for path in ["/api/gifs", "/api/gifs/trending", "/api/gifs/search?q=hi"] {
        let res = anonymous.send(Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let res = anonymous
        .send(
            Method::GET,
            &media_path(&format!("{}/media/a.webp", klipy.origin)),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(klipy.seen().is_empty());
}

#[sqlx::test]
async fn trending_and_search_are_relayed_without_exposing_the_key(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;

    let res = ada.send(Method::GET, "/api/gifs", None).await;
    assert_eq!(res.body, json!({ "enabled": true, "provider": "klipy" }));

    let res = ada
        .send_with(Method::GET, "/api/gifs/trending", None, |builder| {
            builder
                .headers_mut()
                .unwrap()
                .insert("accept-language", "de-AT,de;q=0.9".parse().unwrap());
        })
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["next_page"], 2);
    let gifs = res.body["gifs"].as_array().unwrap();
    assert_eq!(gifs.len(), 1, "the ad is not passed on: {}", res.body);
    assert_eq!(gifs[0]["slug"], "hello-hi-662");
    assert_eq!(gifs[0]["provider"], "klipy");
    assert_eq!(gifs[0]["width"], 220);
    assert_eq!(
        gifs[0]["url"],
        format!("{}/media/hello-hi-662-sm.webp", klipy.origin)
    );
    assert!(!res.body.to_string().contains(KEY));

    let res = ada
        .send(Method::GET, "/api/gifs/search?q=hallo%20welt&page=2", None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["gifs"][0]["title"], "hallo welt");
    assert_eq!(res.body["next_page"], 3);

    let seen = klipy.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].starts_with(&format!(
        "GET {KEY} trending page=1&per_page=24&customer_id="
    )));
    assert!(seen[0].contains("content_filter=medium"));
    assert!(seen[0].ends_with("locale=at"), "{}", seen[0]);
    assert!(seen[1].contains("page=2") && seen[1].contains("q=hallo welt"));
    let user_id = ada.user_id().to_owned();
    assert!(seen.iter().all(|line| !line.contains(&user_id)));

    for (path, code) in [
        ("/api/gifs/search", "required"),
        ("/api/gifs/search?q=%20", "required"),
        (
            &*format!("/api/gifs/search?q={}", "a".repeat(101)),
            "too_long",
        ),
    ] {
        let res = ada.send(Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(res.body["fields"]["q"], code, "{path}");
    }
    let res = ada
        .send(Method::GET, "/api/gifs/trending?page=0", None)
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(klipy.seen().len(), 2, "invalid requests stay local");
}

#[sqlx::test]
async fn search_has_its_own_rate_limit(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[("API_RATE_GIF_PER_MIN", "2")]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;
    for _ in 0..2 {
        let res = ada.send(Method::GET, "/api/gifs/trending", None).await;
        assert_eq!(res.status, StatusCode::OK);
    }
    let res = ada.send(Method::GET, "/api/gifs/search?q=hi", None).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], "rate_limited");
    assert_eq!(klipy.seen().len(), 2);
}

#[sqlx::test]
async fn a_gif_is_sent_as_its_own_message(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;
    let channel = text_channel(&mut ada).await;
    let path = format!("/api/channels/{channel}/messages");

    let res = ada
        .send(
            Method::POST,
            &path,
            Some(json!({ "gif": { "slug": "hello-hi-662", "query": "hallo" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let message = res.body;
    assert_eq!(message["content"], "");
    assert_eq!(message["gif"]["slug"], "hello-hi-662");
    assert_eq!(message["gif"]["title"], "Hello");
    assert_eq!(
        message["gif"]["url"],
        format!("{}/media/hello-hi-662-hd.webp", klipy.origin)
    );
    assert_eq!(
        message["gif"]["still"],
        format!("{}/media/hello-hi-662-hd.jpg", klipy.origin)
    );
    assert_eq!(
        (
            message["gif"]["width"].clone(),
            message["gif"]["height"].clone()
        ),
        (json!(480), json!(240))
    );

    let res = ada.send(Method::GET, &path, None).await;
    let listed = &res.body["messages"][0];
    assert_eq!(listed["id"], message["id"]);
    assert_eq!(listed["gif"], message["gif"]);

    let res = ada
        .send(Method::POST, &path, Some(json!({ "content": "plain" })))
        .await;
    assert!(res.body.get("gif").is_none(), "{}", res.body);

    let res = ada
        .send(
            Method::PATCH,
            &format!("/api/messages/{}", message["id"].as_str().unwrap()),
            Some(json!({ "content": "caption" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["fields"]["gif"], "not_editable");

    for (body, code) in [
        (
            json!({ "content": "hi", "gif": { "slug": "hello-hi-662" } }),
            "exclusive",
        ),
        (json!({ "gif": { "slug": "unknown-slug" } }), "invalid"),
        (json!({ "gif": { "slug": "../items?slugs=x" } }), "invalid"),
    ] {
        let res = ada.send(Method::POST, &path, Some(body.clone())).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(res.body["fields"]["gif"], code, "{body}");
    }

    let mut share = None;
    for _ in 0..50 {
        share = klipy
            .seen()
            .into_iter()
            .find(|line| line.starts_with("SHARE"));
        if share.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let share = share.expect("share trigger");
    assert!(
        share.starts_with(&format!("SHARE {KEY} hello-hi-662 ")),
        "{share}"
    );
    let body: Value = serde_json::from_str(share.splitn(4, ' ').nth(3).unwrap()).unwrap();
    assert_eq!(body["q"], "hallo");
    assert_eq!(body["customer_id"].as_str().unwrap().len(), 32);
}

#[sqlx::test]
async fn a_gif_can_be_sent_in_a_dm(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;
    let mut bob = user(&state, "bob@example.com", "Bob").await;
    let res = ada
        .send(
            Method::POST,
            "/api/dms",
            Some(json!({ "user_id": bob.user_id() })),
        )
        .await;
    let dm = res.body["id"].as_str().unwrap().to_owned();
    let res = ada
        .send(
            Method::POST,
            &format!("/api/channels/{dm}/messages"),
            Some(json!({ "gif": { "slug": "hello-hi-662" } })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = bob
        .send(Method::GET, &format!("/api/channels/{dm}/messages"), None)
        .await;
    assert_eq!(res.body["messages"][0]["gif"]["slug"], "hello-hi-662");
}

#[sqlx::test]
async fn media_is_streamed_only_from_the_allowed_origin(pool: PgPool) {
    let klipy = klipy().await;
    let state = app_state(pool, Some(&klipy), &[]);
    let mut ada = user(&state, "ada@example.com", "Ada").await;

    let path = media_path(&format!("{}/media/a.webp", klipy.origin));
    let (status, headers, bytes) = ada.send_raw(Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, WEBP);
    assert_eq!(headers[CONTENT_TYPE], "image/webp");
    assert_eq!(headers[ETAG], "\"v1\"");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .starts_with("private")
    );

    let res = ada
        .send_with(Method::GET, &path, None, |builder| {
            builder
                .headers_mut()
                .unwrap()
                .insert(IF_NONE_MATCH, "\"v1\"".parse().unwrap());
        })
        .await;
    assert_eq!(res.status, StatusCode::NOT_MODIFIED);

    for (url, status) in [
        (
            "https://static.klipy.com.evil.example/a.webp".to_owned(),
            StatusCode::NOT_FOUND,
        ),
        (
            "http://127.0.0.1:1/a.webp".to_owned(),
            StatusCode::NOT_FOUND,
        ),
        ("not a url".to_owned(), StatusCode::NOT_FOUND),
        (
            format!("{}/media/redirect.webp", klipy.origin),
            StatusCode::BAD_GATEWAY,
        ),
        (
            format!("{}/media/page.webp", klipy.origin),
            StatusCode::BAD_GATEWAY,
        ),
        (
            format!("{}/media/gone.webp", klipy.origin),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (got, _, _) = ada.send_raw(Method::GET, &media_path(&url), None).await;
        assert_eq!(got, status, "{url}");
    }
}
