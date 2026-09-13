use std::process::ExitCode;

use tokio::net::TcpListener;
use tracing::{error, info};

use gelabber_api::{AppState, Config, app, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
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
    let state = AppState::from_config(&config)?;

    // Schema first, traffic second. Compose only starts the API once
    // Postgres is healthy, so a failure here is a real misconfiguration.
    state
        .migrate()
        .await
        .map_err(|err| format!("database migration failed: {err}"))?;
    info!("database migrations applied");

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

    axum::serve(listener, app(state))
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
