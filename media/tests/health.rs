use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use gelabber_media::{Config, app};
use tower::ServiceExt;

fn state() -> gelabber_media::AppState {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        _ => None,
    })
    .expect("config");
    gelabber_media::AppState::from_config(&config).expect("state")
}

#[tokio::test]
async fn health_is_ok_without_redis() {
    let app = app(state());
    for path in ["/health", "/media/health"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["service"], "media");
        assert_eq!(body["status"], "ok");
        assert_ne!(body["status"], "stub");
    }
}

#[tokio::test]
async fn metrics_export_rooms_peers_bytes_and_ice_fails() {
    let app = app(state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for needle in [
        "gelabber_media_rooms",
        "gelabber_media_peers",
        "gelabber_media_forwarded_bytes_total",
        "gelabber_media_ice_fails_total",
    ] {
        assert!(text.contains(needle), "missing {needle} in {text}");
    }
    assert!(text.contains("gelabber_media_rooms 0"));
    assert!(text.contains("gelabber_media_peers 0"));
}
