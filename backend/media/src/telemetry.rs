//! JSON tracing to stdout. Level filter comes from `RUST_LOG` (default `info`).

use tracing_subscriber::EnvFilter;

use crate::config::{ConfigError, RUST_LOG};

pub fn init() -> Result<(), ConfigError> {
    let (filter, error) = match std::env::var(RUST_LOG) {
        Ok(raw) if !raw.trim().is_empty() => match EnvFilter::try_new(&raw) {
            Ok(filter) => (filter, None),
            Err(err) => (
                EnvFilter::new("info"),
                Some(ConfigError::Invalid {
                    key: RUST_LOG,
                    value: raw,
                    reason: err.to_string(),
                }),
            ),
        },
        _ => (EnvFilter::new("info"), None),
    };

    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false)
        .with_target(true)
        .with_env_filter(filter)
        .init();

    error.map_or(Ok(()), Err)
}
