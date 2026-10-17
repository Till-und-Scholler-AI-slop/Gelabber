//! Mentions in message text (issue #241).
//!
//! The composer writes a mention of a user as `<@user-id>`, a token that
//! survives renames. `@everyone` mentions every member of the conversation,
//! `@here` the members online when the message is written. Text in code
//! (`` `…` `` and ``` fenced ``` blocks) mentions nobody. There are no roles
//! beyond owner and member, so there are no role mentions.
//!
//! Only members are kept and never the author: a token for someone outside
//! the conversation mentions nobody and tells the author nothing.

use uuid::Uuid;

use super::MessagingChannel;
use crate::{error::ApiError, gateway::PresenceStatus, state::AppState};

/// More named users than this are not kept; `@everyone` exists for that.
pub const MAX_USERS: usize = 100;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub users: Vec<Uuid>,
    pub everyone: bool,
    pub here: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub users: Vec<Uuid>,
    pub everyone: bool,
}

/// The text outside inline code and fenced code blocks.
fn outside_code(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let ticks = rest[start..].chars().take_while(|c| *c == '`').count();
        let fence = &rest[start..start + ticks];
        let after = &rest[start + ticks..];
        match after.find(fence) {
            // Code: dropped, a space keeps the words around it apart.
            Some(end) => {
                out.push(' ');
                rest = &after[end + ticks..];
            }
            // An unclosed run of backticks is plain text.
            None => {
                out.push_str(fence);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `@word` on its own: not inside an address (`a@everyone.org`) or a longer
/// word (`@everyoneelse`).
fn has_keyword(text: &str, word: &str) -> bool {
    let needle = format!("@{word}");
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        let word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        !before.is_some_and(word_char) && !after.is_some_and(|c| word_char(c) || c == '@')
    })
}

pub fn parse(content: &str) -> Parsed {
    let text = outside_code(content);
    let mut users = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<@") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('>') else { break };
        if let Ok(id) = Uuid::parse_str(&rest[..end])
            && !users.contains(&id)
        {
            users.push(id);
        }
        rest = &rest[end + 1..];
    }
    users.truncate(MAX_USERS);
    Parsed {
        users,
        everyone: has_keyword(&text, "everyone"),
        here: has_keyword(&text, "here"),
    }
}

/// The members a message by `author` mentions, for storing with it.
pub(crate) async fn resolve(
    state: &AppState,
    access: &MessagingChannel,
    author: Uuid,
    content: &str,
) -> Result<Resolved, ApiError> {
    let parsed = parse(content);
    match access {
        // Two people: `@everyone` and `@here` both mean the other one.
        MessagingChannel::Dm { channel } => {
            let users: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM channel_members WHERE channel_id=$1 AND user_id=ANY($2) \
                 AND user_id<>$3 ORDER BY user_id",
            )
            .bind(channel.id)
            .bind(&parsed.users)
            .bind(author)
            .fetch_all(&state.db)
            .await?;
            Ok(Resolved {
                users,
                everyone: parsed.everyone || parsed.here,
            })
        }
        MessagingChannel::Server { member } => {
            let mut wanted = parsed.users;
            if parsed.here && !parsed.everyone {
                // Best effort: without Redis `@here` reaches nobody, the
                // message itself still goes out.
                match state.gateway.presence_snapshot(member.server.id).await {
                    Ok(online) => wanted.extend(
                        online
                            .into_iter()
                            .filter(|entry| entry.st != PresenceStatus::Offline)
                            .map(|entry| entry.u),
                    ),
                    Err(error) => {
                        tracing::debug!(error = error.code(), "@here without presence")
                    }
                }
            }
            let users: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM server_members WHERE server_id=$1 AND user_id=ANY($2) \
                 AND user_id<>$3 ORDER BY user_id",
            )
            .bind(member.server.id)
            .bind(&wanted)
            .bind(author)
            .fetch_all(&state.db)
            .await?;
            Ok(Resolved {
                users,
                everyone: parsed.everyone,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0190f7a2-0000-7000-8000-00000000000a";
    const B: &str = "0190f7a2-0000-7000-8000-00000000000b";

    fn ids(list: &[&str]) -> Vec<Uuid> {
        list.iter().map(|id| Uuid::parse_str(id).unwrap()).collect()
    }

    #[test]
    fn tokens_name_users_once_in_order() {
        let parsed = parse(&format!("hi <@{A}> and <@{B}>, <@{A}> again"));
        assert_eq!(parsed.users, ids(&[A, B]));
        assert!(!parsed.everyone && !parsed.here);
        assert!(parse("<@not-a-uuid> <@").users.is_empty());
        assert!(parse(&format!("<@{A}")).users.is_empty());
    }

    #[test]
    fn everyone_and_here_stand_alone() {
        assert!(parse("@everyone look").everyone);
        assert!(parse("look, @everyone!").everyone);
        assert!(parse("(@everyone)").everyone);
        assert!(parse("@here.").here);
        assert!(!parse("mail me@everyone.org").everyone);
        assert!(!parse("@everyoneelse").everyone);
        assert!(!parse("@here_and_now").here);
        assert!(!parse("everyone").everyone);
    }

    #[test]
    fn code_mentions_nobody() {
        let parsed = parse(&format!("`<@{A}>` ```\n@everyone\n``` <@{B}> `@here`"));
        assert_eq!(parsed.users, ids(&[B]));
        assert!(!parsed.everyone && !parsed.here);
        // An unclosed backtick is text.
        assert!(parse("it`s @everyone").everyone);
        // Code directly next to a keyword does not glue words together.
        assert!(parse("`x`@here").here);
    }

    #[test]
    fn many_users_are_capped() {
        let text: String = (0..150u128)
            .map(|n| format!("<@{}> ", Uuid::from_u128(n + 1)))
            .collect();
        assert_eq!(parse(&text).users.len(), MAX_USERS);
    }
}
