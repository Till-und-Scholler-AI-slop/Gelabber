use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use gelabber_media::{Config, app};
use tower::ServiceExt;

async fn state() -> gelabber_media::AppState {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        _ => None,
    })
    .expect("config");
    gelabber_media::AppState::from_config(&config)
        .await
        .expect("state")
}

#[tokio::test]
async fn health_is_ok_without_redis() {
    let app = app(state().await);
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
    let app = app(state().await);
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
    let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for needle in [
        "gelabber_media_rooms",
        "gelabber_media_peers",
        "gelabber_media_forwarded_bytes_total",
        "gelabber_mediasoup_ice_disconnects_total",
        "gelabber_media_rtp_packets_total",
        "gelabber_media_rtp_lost_total",
        "gelabber_media_rtp_jitter_ms",
    ] {
        assert!(text.contains(needle), "missing {needle} in {text}");
    }
    assert!(text.contains("gelabber_media_rooms 0"));
    assert!(text.contains("gelabber_media_peers 0"));
}

#[tokio::test]
async fn media_prefixed_metrics_alias_is_not_served() {
    // A proxy that forwards `/media/*` must not publish the scrape.
    let response = app(state().await)
        .oneshot(
            Request::builder()
                .uri("/media/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ready_and_metrics_report_the_worker_pool() {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        "MEDIA_WORKERS" => Some("2".to_owned()),
        _ => None,
    })
    .expect("config");
    let app = app(gelabber_media::AppState::from_config(&config)
        .await
        .expect("state"));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Redis is unreachable on purpose; the media engine itself is ready.
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["checks"]["media"]["status"], "ok");
    assert_eq!(body["workers"]["live"], 2);
    assert_eq!(body["workers"]["total"], 2);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for needle in [
        "gelabber_media_workers 2\n",
        "gelabber_media_workers_live 2\n",
        "gelabber_media_worker_failures_total 0\n",
        "gelabber_media_worker_rooms{worker=\"1\"} 0\n",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
}

#[tokio::test]
async fn ready_probes_redis_at_most_once_a_second() {
    // A counting TCP proxy in front of the test Redis.
    let url = std::env::var("REDIS_URL").expect("isolated REDIS_URL required");
    let info = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection_info()
        .clone();
    let redis::ConnectionAddr::Tcp(host, port) = info.addr().clone() else {
        panic!("local TCP Redis required");
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let target = (host.clone(), port);
            tokio::spawn(async move {
                let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    let redis_url = format!("redis://{proxy}");
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some(redis_url.clone()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".to_owned()),
        _ => None,
    })
    .expect("config");
    let app = app(gelabber_media::AppState::from_config(&config)
        .await
        .expect("state"));
    let ready = || {
        app.clone().oneshot(
            Request::builder()
                .uri("/media/ready")
                .body(Body::empty())
                .unwrap(),
        )
    };
    let before = accepted.load(std::sync::atomic::Ordering::SeqCst);
    let answers = futures_util::future::join_all((0..10).map(|_| ready())).await;
    for response in answers {
        assert_eq!(response.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(
        accepted.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "a burst shares one probe"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(ready().await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        accepted.load(std::sync::atomic::Ordering::SeqCst) - before,
        2,
        "a second later the answer is fresh"
    );
}
