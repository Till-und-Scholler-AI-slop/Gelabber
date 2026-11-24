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

use super::{Message, MessageRow, lock_access, message_for, persist_event};
use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, FieldErrors},
    gateway::{EventKind, delivery},
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
    let access = lock_access(&mut tx, initial.channel_id, user).await?;
    // The one blocked reacts on as before (#619): on the peer's messages the
    // reaction is stored for them alone, like a sender-only message. The
    // blocker gets `blocked` from the precheck.
    let mut hidden = false;
    if add {
        access.require_send()?;
        // A reaction answers too: not before a message request is accepted,
        // and not by someone who blocks the peer (#565).
        if let super::MessagingChannel::Dm { .. } = &access {
            hidden = crate::blocks::in_dm(&mut *tx, initial.channel_id, user).await?
                == crate::blocks::Block::Theirs
                && !initial.sender_only;
            let draft = crate::dms::request::Draft {
                content: "",
                files: false,
                gif: false,
            };
            crate::dms::request::precheck_in(&mut tx, initial.channel_id, user, &draft).await?;
        }
    }
    let row = sqlx::query_as::<_, MessageRow>(super::MESSAGE_BY_ID_SQL)
        .bind(id)
        .bind(user)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut current = Message::from(row);
    current.attachments = initial.attachments;
    super::replies::for_messages(&mut *tx, std::slice::from_mut(&mut current), Some(user)).await?;
    if add {
        // The channel lock serializes the limit check with every reaction write.
        // Existing emoji and idempotent retries remain valid at the limit.
        // Only what the caller sees counts: hidden reactions of someone the
        // caller blocks must not use up the caller's slots (#619).
        let emojis: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT emoji FROM message_reactions \
             WHERE message_id=$1 AND (NOT sender_only OR user_id=$2)",
        )
        .bind(id)
        .bind(user)
        .fetch_all(&mut *tx)
        .await?;
        if emojis.len() >= 20 && !emojis.iter().any(|existing| existing == emoji) {
            return Err(ApiError::Validation(FieldErrors::from([(
                "emoji", "limit",
            )])));
        }
    }
    // A reaction given hidden and later given openly becomes open.
    let written: Option<bool> = if add {
        sqlx::query_scalar(
            "INSERT INTO message_reactions(message_id,user_id,emoji,sender_only) VALUES($1,$2,$3,$4) \
             ON CONFLICT (message_id,user_id,emoji) DO UPDATE SET sender_only=false \
             WHERE message_reactions.sender_only AND NOT EXCLUDED.sender_only \
             RETURNING sender_only",
        )
        .bind(id)
        .bind(user)
        .bind(emoji)
        .bind(hidden)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            "DELETE FROM message_reactions WHERE message_id=$1 AND user_id=$2 AND emoji=$3 \
             RETURNING sender_only",
        )
        .bind(id)
        .bind(user)
        .bind(emoji)
        .fetch_optional(&mut *tx)
        .await?
    };
    let changed = written.is_some();
    // Only the open change of an open reaction goes out.
    let announce = written == Some(false);
    if announce {
        persist_event(
            &mut tx,
            access.event_server_id(),
            EventKind::E,
            &mut current,
            Some(user),
        )
        .await?;
    }
    if changed {
        // The answer shows the caller's own hidden reactions too.
        populate_for(&mut tx, &mut current, Some(user)).await?;
    }
    if !announce {
        // Unchanged, the reply is still the full message: a client applying
        // it must not lose the link previews.
        crate::previews::for_messages(&mut *tx, std::slice::from_mut(&mut current)).await?;
    }
    tx.commit().await?;
    if announce {
        delivery::wake(state);
    }
    Ok(current)
}

/// The reactions everyone sees, for an event: without hidden ones (#619).
pub(super) async fn populate(db: &mut PgConnection, message: &mut Message) -> Result<(), ApiError> {
    populate_for(db, message, None).await
}

/// The reactions `viewer` sees: the open ones and their own hidden ones.
pub(super) async fn populate_for(
    db: &mut PgConnection,
    message: &mut Message,
    viewer: Option<Uuid>,
) -> Result<(), ApiError> {
    let rows: Vec<(String, Vec<Uuid>)> = sqlx::query_as("SELECT emoji,array_agg(user_id ORDER BY user_id) FROM message_reactions WHERE message_id=$1 AND (NOT sender_only OR user_id=$2) GROUP BY emoji ORDER BY emoji")
        .bind(message.id).bind(viewer).fetch_all(db).await?;
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
