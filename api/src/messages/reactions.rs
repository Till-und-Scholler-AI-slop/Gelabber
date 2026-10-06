//! Idempotent reactions use the same channel write lock/revision/outbox as edits.
use std::{collections::HashMap, sync::OnceLock};

use axum::{
    Json, Router,
    extract::{Path, State, rejection::PathRejection},
    routing::put,
};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{Message, MessageRow, MessagingChannel, message_for, persist_event};
use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, FieldErrors},
    gateway::{EventKind, delivery},
    servers::{channel, membership},
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Reaction {
    pub emoji: String,
    pub user_ids: Vec<Uuid>,
}

#[derive(Deserialize)]
struct EmojiData {
    emoji: Vec<EmojiEntry>,
}
#[derive(Deserialize)]
struct EmojiEntry {
    emoji: String,
}

fn canonical(value: &str) -> Option<&'static str> {
    static EMOJI: OnceLock<HashMap<String, String>> = OnceLock::new();
    if value.len() > 128 {
        return None;
    }
    let emoji = EMOJI.get_or_init(|| {
        let data: EmojiData =
            serde_json::from_str(include_str!("../../../shared/data/emoji-18.0.json"))
                .expect("checked-in Unicode data");
        let mut map = HashMap::new();
        for entry in data.emoji {
            map.insert(entry.emoji.replace('\u{fe0f}', ""), entry.emoji.clone());
            map.insert(entry.emoji.clone(), entry.emoji);
        }
        map
    });
    emoji.get(value).map(String::as_str)
}

pub(super) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/messages/{id}/reactions/{emoji}",
        put(add).delete(remove),
    )
}

async fn add(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    path: Result<Path<(Uuid, String)>, PathRejection>,
) -> Result<Json<Message>, ApiError> {
    let Path((id, emoji)) = path.map_err(|_| ApiError::NotFound)?;
    mutate(&state, id, user.id, &emoji, true).await.map(Json)
}
async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    path: Result<Path<(Uuid, String)>, PathRejection>,
) -> Result<Json<Message>, ApiError> {
    let Path((id, emoji)) = path.map_err(|_| ApiError::NotFound)?;
    mutate(&state, id, user.id, &emoji, false).await.map(Json)
}

async fn mutate(
    state: &AppState,
    id: Uuid,
    user: Uuid,
    emoji: &str,
    add: bool,
) -> Result<Message, ApiError> {
    let (_, initial) = message_for(&state.db, id, user).await?;
    let emoji = canonical(emoji)
        .ok_or_else(|| ApiError::Validation(FieldErrors::from([("emoji", "invalid")])))?;
    let mut tx = state.db.begin().await?;
    // Membership writers serialize through the server row. Keep membership and
    // permissions stable through this write; DM membership is held likewise.
    sqlx::query("SELECT s.id FROM servers s JOIN channels c ON c.server_id=s.id WHERE c.id=$1 FOR SHARE OF s")
        .bind(initial.channel_id).fetch_optional(&mut *tx).await?;
    delivery::lock_channel(&mut tx, initial.channel_id).await?;
    let channel = channel::get(&mut *tx, initial.channel_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let access = if let Some(server) = channel.server_id {
        MessagingChannel::Server {
            member: membership::load(&mut *tx, server, user).await?,
        }
    } else {
        let participant: Option<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM channel_members WHERE channel_id=$1 AND user_id=$2 FOR SHARE",
        )
        .bind(channel.id)
        .bind(user)
        .fetch_optional(&mut *tx)
        .await?;
        participant.ok_or(ApiError::NotFound)?;
        MessagingChannel::Dm { channel }
    };
    if add {
        access.require_send()?;
    }
    let row = sqlx::query_as::<_, MessageRow>(super::MESSAGE_BY_ID_SQL)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut current = Message::from(row);
    current.attachments = initial.attachments;
    if add {
        // The channel lock serializes the limit check with every reaction write.
        // Existing emoji and idempotent retries remain valid at the limit.
        let emojis: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT emoji FROM message_reactions WHERE message_id=$1")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        if emojis.len() >= 20 && !emojis.iter().any(|existing| existing == emoji) {
            return Err(ApiError::Validation(FieldErrors::from([(
                "emoji", "limit",
            )])));
        }
    }
    let changed = if add {
        sqlx::query("INSERT INTO message_reactions(message_id,user_id,emoji) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
    } else {
        sqlx::query("DELETE FROM message_reactions WHERE message_id=$1 AND user_id=$2 AND emoji=$3")
    }.bind(id).bind(user).bind(emoji).execute(&mut *tx).await?.rows_affected() > 0;
    if changed {
        persist_event(
            &mut tx,
            access.event_server_id(),
            EventKind::E,
            &mut current,
        )
        .await?;
    }
    tx.commit().await?;
    if changed {
        let _ = delivery::deliver_pending(state, 32).await;
    }
    Ok(current)
}

pub(super) async fn populate(db: &mut PgConnection, message: &mut Message) -> Result<(), ApiError> {
    let rows: Vec<(String, Vec<Uuid>)> = sqlx::query_as("SELECT emoji,array_agg(user_id ORDER BY user_id) FROM message_reactions WHERE message_id=$1 GROUP BY emoji ORDER BY emoji")
        .bind(message.id).fetch_all(db).await?;
    message.reactions = rows
        .into_iter()
        .map(|(emoji, user_ids)| Reaction { emoji, user_ids })
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::canonical;
    #[test]
    fn canonical_unicode_sequences_and_variants() {
        assert_eq!(canonical("❤"), Some("❤️"));
        assert_eq!(canonical("👍🏽"), Some("👍🏽"));
        assert_eq!(canonical("👩‍💻"), Some("👩‍💻"));
        assert!(canonical("x").is_none());
        assert!(canonical("😀😀").is_none());
        assert!(canonical(&"x".repeat(129)).is_none());
    }
}
