//! Benchmark-only lease minting around the unchanged production media library.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use gelabber_media::{AppState, Config};
use gelabber_shared::ticket::*;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone)]
struct Mint {
    redis: redis::Client,
    sfu: Arc<gelabber_media::sfu::Sfu>,
    token: Arc<String>,
    server: Uuid,
    channel: Uuid,
    generation: Uuid,
}
async fn rpc(
    State(mint): State<Mint>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let failure = |text: &str| (StatusCode::BAD_REQUEST, Json(json!({"error":text})));
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {}", mint.token))
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        ));
    }
    if request["op"] == "summary" {
        let metrics = mint.sfu.metrics_text();
        let values: serde_json::Map<String, Value> = metrics
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((
                    fields.next()?.to_owned(),
                    json!(fields.next()?.parse::<f64>().ok()?),
                ))
            })
            .collect();
        return Ok(Json(
            json!({"peers":values.get("gelabber_media_peers"),"rooms":values.get("gelabber_media_rooms"),"metrics":values}),
        ));
    }
    if request["op"] != "join" {
        return Err(failure("only benchmark join is supported"));
    }
    let user = Uuid::new_v4();
    let session = Uuid::new_v4().simple().to_string().repeat(2);
    let mut conn = mint
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| failure("redis unavailable"))?;
    let now: (u64, u64) = redis::cmd("TIME")
        .query_async(&mut conn)
        .await
        .map_err(|_| failure("redis clock unavailable"))?;
    // Independent test Redis only. TTLs cover one bounded benchmark run; this is
    // deliberately not a production authentication/authorization implementation.
    let claim = AuthorizedTicketClaim {
        claim: TicketClaim {
            u: user,
            s: mint.server,
            c: mint.channel,
            g: false,
        },
        auth: TicketAuthorization {
            session: session.clone(),
            expires_at: now.0 + 3600,
            member: mint.generation,
            channel: mint.generation,
        },
    };
    let code = generate();
    let mut pipe = redis::pipe();
    for (key, value) in [
        (
            member_authority_key(mint.server, user),
            mint.generation.to_string(),
        ),
        (
            channel_authority_key(mint.channel),
            mint.generation.to_string(),
        ),
        (session_authority_key(&session), user.to_string()),
        (redis_key(&code), serde_json::to_string(&claim).unwrap()),
    ] {
        pipe.cmd("SET")
            .arg(key)
            .arg(value)
            .arg("EX")
            .arg(3600)
            .ignore();
    }
    pipe.query_async::<()>(&mut conn)
        .await
        .map_err(|_| failure("redis mint failed"))?;
    Ok(Json(
        json!({"tk":code,"user":user,"channel":mint.channel,"production_feature_acceptance":false}),
    ))
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    gelabber_media::telemetry::init()?;
    let token = std::env::var("BENCH_TOKEN")?;
    if token.len() < 32 {
        return Err("BENCH_TOKEN must contain at least 32 bytes".into());
    }
    let config = Config::from_env()?;
    let state = AppState::from_config(&config)?;
    let routes = Router::new()
        .route("/rpc", post(rpc))
        .with_state(Mint {
            redis: state.redis.clone(),
            sfu: state.sfu.clone(),
            token: Arc::new(token),
            server: Uuid::new_v4(),
            channel: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        })
        .merge(gelabber_media::app(state));
    let listener = tokio::net::TcpListener::bind(config.media_addr).await?;
    println!(
        "current benchmark adapter listening on {}",
        listener.local_addr()?
    );
    axum::serve(listener, routes)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
