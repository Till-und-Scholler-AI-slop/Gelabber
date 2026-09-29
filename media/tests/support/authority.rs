//! Test-only API lease writer. Never used by the SFU to renew authority.
use gelabber_shared::ticket::TicketClaim;
use tokio::sync::watch;
use uuid::Uuid;

pub struct TestAuthority {
    stop: watch::Sender<bool>,
    redis: redis::Client,
    keys: Vec<String>,
}
impl Drop for TestAuthority {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        let redis = self.redis.clone();
        let keys = self.keys.clone();
        tokio::spawn(async move {
            if let Ok(mut conn) = redis.get_multiplexed_async_connection().await {
                let _: Result<(), _> = redis::cmd("DEL").arg(keys).query_async(&mut conn).await;
            }
        });
    }
}

pub async fn mint(redis: &redis::Client, code: &str, claim: TicketClaim) -> TestAuthority {
    use gelabber_shared::ticket::*;
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let member_key = member_authority_key(claim.s, claim.u);
    let channel_key = channel_authority_key(claim.c);
    let nonce = Uuid::new_v4().to_string();
    for key in [&member_key, &channel_key] {
        let _: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(&nonce)
            .arg("NX")
            .query_async(&mut conn)
            .await
            .unwrap();
    }
    let member: String = redis::cmd("GET")
        .arg(&member_key)
        .query_async(&mut conn)
        .await
        .unwrap();
    let channel: String = redis::cmd("GET")
        .arg(&channel_key)
        .query_async(&mut conn)
        .await
        .unwrap();
    let now: (u64, u64) = redis::cmd("TIME").query_async(&mut conn).await.unwrap();
    let authority = AuthorizedTicketClaim {
        claim,
        auth: TicketAuthorization {
            session: Uuid::new_v4().simple().to_string().repeat(2),
            expires_at: now.0 + 300,
            member: member.parse().unwrap(),
            channel: channel.parse().unwrap(),
        },
    };
    let lease_key = session_authority_key(&authority.auth.session);
    let user = authority.claim.u.to_string();
    let _: () = redis::cmd("SET")
        .arg(&lease_key)
        .arg(&user)
        .arg("PX")
        .arg(3000)
        .query_async(&mut conn)
        .await
        .unwrap();
    let _: () = redis::cmd("SET")
        .arg(redis_key(code))
        .arg(serde_json::to_string(&authority).unwrap())
        .arg("EX")
        .arg(60)
        .query_async(&mut conn)
        .await
        .unwrap();
    let keys = vec![
        member_key,
        channel_key,
        lease_key.clone(),
        media_demand_key(&authority.auth.session),
    ];
    let (stop, mut ended) = watch::channel(false);
    let writer = redis.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = ended.changed() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    if let Ok(mut conn) = writer.get_multiplexed_async_connection().await {
                        let _: Result<(), _> = redis::cmd("SET").arg(&lease_key).arg(&user).arg("PX").arg(3000).query_async(&mut conn).await;
                    }
                }
            }
        }
    });
    TestAuthority {
        stop,
        redis: redis.clone(),
        keys,
    }
}
