use std::process::ExitCode;

use tokio::net::TcpListener;
use tracing::{error, info};

use gelabber_api::{AppState, Config, app, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
    // `gelabber-api vapid-keys`: a key pair for Web Push (deploy/README.md).
    if std::env::args().nth(1).as_deref() == Some("vapid-keys") {
        let (private, public) = gelabber_api::push::webpush::Vapid::generate();
        println!("VAPID_PRIVATE_KEY={private}");
        println!("# public key, served at /api/push/key: {public}");
        return ExitCode::SUCCESS;
    }
    if let Err(err) = telemetry::init() {
        error!(error = %err, "gelabber-api failed to start");
        return ExitCode::FAILURE;
    }

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(error = %err, "gelabber-api failed to start");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    if let Some(minio) = config.minio.as_ref() {
        if minio.delivery == gelabber_api::config::AttachmentDelivery::Proxy {
            info!("attachments go through the API (ATTACHMENT_DELIVERY=proxy)");
        }
        for warning in minio.public_endpoint_warnings(config.cookie_secure) {
            tracing::warn!("{warning}");
        }
    }
    if config
        .minio
        .as_ref()
        .is_some_and(|minio| minio.root_credentials)
    {
        tracing::warn!(
            "signed in to MinIO as root (MINIO_ROOT_USER): set MINIO_APP_PASSWORD and update deploy/compose so the API uses its bucket-scoped user (deploy/README.md, MinIO user)"
        );
    }
    let state = AppState::from_config(&config)?;

    // Schema first, traffic second. Compose only starts the API once
    // Postgres is healthy, so a failure here is a real misconfiguration.
    state
        .migrate()
        .await
        .map_err(|err| format!("database migration failed: {err}"))?;
    info!("database migrations applied");

    match state.store.ensure_ready().await {
        Ok(()) => info!("object store ready"),
        Err(err) => tracing::warn!(error = %err, "object store not ready; uploads will retry"),
    }

    gelabber_api::password::warm_up()
        .await
        .map_err(|err| format!("password hasher warm-up failed: {}", err.code()))?;

    let listener = TcpListener::bind(config.api_addr)
        .await
        .map_err(|err| format!("failed to bind {}: {err}", config.api_addr))?;
    let local_addr = listener.local_addr()?;

    info!(
        addr = %local_addr,
        ready_timeout_ms = config.ready_timeout.as_millis() as u64,
        db_max_connections = config.db_max_connections,
        "gelabber-api listening"
    );

    // The peer address is the rate-limit key when no proxy wrote
    // `X-Forwarded-For` (`limits::client_ip`).
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("gelabber-api stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(error = %err, "failed to install Ctrl+C handler");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                error!(error = %err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    info!("shutdown signal received, draining connections");
}
