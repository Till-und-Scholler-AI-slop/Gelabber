//! One reconnecting connection per media engine; no TCP handshake per RPC.
use redis::aio::{ConnectionLike, ConnectionManager, MultiplexedConnection};
use std::future::Future;
use tokio::sync::OnceCell;

pub trait ConnectionSource: Sync {
    type Connection: ConnectionLike + Send;
    fn connection(&self) -> impl Future<Output = redis::RedisResult<Self::Connection>> + Send;
}
impl ConnectionSource for redis::Client {
    type Connection = MultiplexedConnection;
    async fn connection(&self) -> redis::RedisResult<Self::Connection> {
        self.get_multiplexed_async_connection().await
    }
}
pub struct CachedRedis {
    client: redis::Client,
    manager: OnceCell<ConnectionManager>,
}
impl CachedRedis {
    pub fn new(client: redis::Client) -> Self {
        Self {
            client,
            manager: OnceCell::new(),
        }
    }
}
impl ConnectionSource for CachedRedis {
    type Connection = ConnectionManager;
    async fn connection(&self) -> redis::RedisResult<Self::Connection> {
        self.manager
            .get_or_try_init(|| self.client.get_connection_manager())
            .await
            .cloned()
    }
}
