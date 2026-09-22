use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::sfu::Sfu;

#[derive(Clone)]
pub struct AppState {
    pub redis: redis::Client,
    pub ready_timeout: Duration,
    pub sfu: Arc<Sfu>,
}

impl AppState {
    pub fn from_config(config: &Config) -> Result<Self, redis::RedisError> {
        let redis = redis::Client::open(config.redis_url.as_str())?;
        Ok(Self {
            redis: redis.clone(),
            ready_timeout: config.ready_timeout,
            sfu: Arc::new(Sfu::with_redis(config, Some(redis))),
        })
    }
}
