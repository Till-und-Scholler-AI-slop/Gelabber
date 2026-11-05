mod common;
use axum::http::{Method, StatusCode};
use common::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
fn doc() -> Value {
    json!({"version":1,"revision":0,"active":"nord","customThemes":[]})
}
fn custom() -> Value {
    json!({"version":1,"id":"custom-11111111-1111-4111-8111-111111111111","name":"Mein Theme","mode":"dark","style":"soft","colors":{"background":"#17191a","panel":"#1d2021","surface":"#272b2d","rail":"#131617","text":"#eef0ee","muted":"#a4abb1","accent":"#f3b752","border":"#2c3032"}})
}
#[sqlx::test]
async fn themes_are_private_revision_checked_and_cascade(pool: PgPool) {
    let mut a = Client::new(pool.clone());
    a.bootstrap().await;
    assert_eq!(
        a.send(Method::GET, "/api/me/themes", None).await.status,
        StatusCode::UNAUTHORIZED
    );
    a.register("themes-a@example.com", "correct horse battery", "Themes A")
        .await;
    assert_eq!(
        a.send(Method::GET, "/api/me/themes", None).await.body["revision"],
        0
    );
    let first = a.send(Method::PUT, "/api/me/themes", Some(doc())).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.body["revision"], 1);
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(doc()))
            .await
            .status,
        StatusCode::CONFLICT
    );
    let mut changed = first.body;
    changed["active"] = custom()["id"].clone();
    changed["customThemes"] = json!([custom()]);
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(changed.clone()))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(changed))
            .await
            .status,
        StatusCode::CONFLICT
    );
    let saved = a.send(Method::GET, "/api/me/themes", None).await.body;
    assert_eq!(saved["revision"], 2);
    assert_eq!(saved["customThemes"][0], custom());
    let mut b = Client::new(pool.clone());
    b.bootstrap().await;
    b.register("themes-b@example.com", "correct horse battery", "Themes B")
        .await;
    assert_eq!(
        b.send(Method::GET, "/api/me/themes", None).await.body["customThemes"],
        json!([])
    );
    a.csrf = None;
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(saved))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    sqlx::query("DELETE FROM users WHERE email = 'themes-a@example.com'")
        .execute(&pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM account_themes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[sqlx::test]
async fn rejects_invalid_themes_without_writing(pool: PgPool) {
    let mut a = Client::new(pool);
    a.bootstrap().await;
    a.register(
        "themes-invalid@example.com",
        "correct horse battery",
        "Theme",
    )
    .await;
    let mut invalid = doc();
    invalid["customThemes"] = json!([custom()]);
    invalid["customThemes"][0]["colors"]["accent"] = json!("url(https://bad)");
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(invalid))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = doc();
    invalid["customThemes"] = json!([custom()]);
    invalid["customThemes"][0]["css"] = json!("body{}");
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(invalid))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = doc();
    invalid["active"] = json!("unknown");
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(invalid))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = doc();
    invalid["customThemes"] = Value::Array(vec![custom(); 51]);
    assert_eq!(
        a.send(Method::PUT, "/api/me/themes", Some(invalid))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        a.send(Method::GET, "/api/me/themes", None).await.body["revision"],
        0
    );
}

#[sqlx::test]
async fn concurrent_devices_cannot_overwrite_each_other(pool: PgPool) {
    let mut a = Client::new(pool.clone());
    a.bootstrap().await;
    a.register("themes-race@example.com", "correct horse battery", "Theme")
        .await;
    let initial = a
        .send(Method::PUT, "/api/me/themes", Some(doc()))
        .await
        .body;
    let mut b = Client::new(pool);
    b.jar = a.jar.clone();
    b.csrf = a.csrf.clone();
    let mut other = initial.clone();
    other["active"] = json!("gruvbox");
    let (one, two) = tokio::join!(
        a.send(Method::PUT, "/api/me/themes", Some(initial)),
        b.send(Method::PUT, "/api/me/themes", Some(other))
    );
    assert!(
        (one.status == StatusCode::OK && two.status == StatusCode::CONFLICT)
            || (two.status == StatusCode::OK && one.status == StatusCode::CONFLICT)
    );
    let read = b.send(Method::GET, "/api/me/themes", None).await;
    assert_eq!(read.body["revision"], 2);
    let winning = if one.status == StatusCode::OK {
        one.body
    } else {
        two.body
    };
    assert_eq!(read.body, winning);
}
