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
use crate::{
    error::ApiError, gateway::PresenceStatus, previews::urls::prose,
    servers::permissions::Permission, state::AppState,
};

/// More named users than this are not kept; `@everyone` exists for that.
pub const MAX_USERS: usize = 100;
/// `@here` stores who was online; beyond this many it reaches the first
/// ones only, and `@everyone` is the way to reach a crowd.
pub const MAX_HERE: usize = 250;
/// `@everyone` / `@here` per author and server within [`EVERYONE_WINDOW`];
/// more are sent as plain text.
pub const EVERYONE_PER_WINDOW: u32 = 3;
pub const EVERYONE_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub users: Vec<Uuid>,
    pub everyone: bool,
    pub here: bool,
}

/// What an earlier version of an edited message already reached. An edit
/// that keeps such a ping does not spend the limit again: fixing a typo in
/// an `@everyone` message must not drop the ping.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Granted {
    pub everyone: bool,
    pub here: bool,
    /// Whom an earlier `@here` reached, where that is known. A kept `@here`
    /// reaches them again instead of whoever is online at the edit, so
    /// editing cannot ping more people.
    pub here_users: Option<Vec<Uuid>>,
}

impl Granted {
    /// From the stored message. `here_users` is the message's own record of
    /// whom `@here` reached (`messages.here_users`); a message written
    /// before it was kept (`None`) shows `@here` as mentions beyond the
    /// users the text names, which loses a named user `@here` also reached.
    pub fn of(
        content: &str,
        mentions_everyone: bool,
        mentions: &[Uuid],
        here_users: Option<&[Uuid]>,
    ) -> Self {
        let here_users = match here_users {
            Some(users) => Some(users.to_vec()),
            None => {
                let parsed = parse(content);
                let beyond: Vec<Uuid> = if parsed.here {
                    mentions
                        .iter()
                        .filter(|id| !parsed.users.contains(id))
                        .copied()
                        .collect()
                } else {
                    Vec::new()
                };
                (!beyond.is_empty()).then_some(beyond)
            }
        };
        Self {
            everyone: mentions_everyone,
            here: mentions_everyone || here_users.is_some(),
            here_users,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub users: Vec<Uuid>,
    pub everyone: bool,
    /// Whom `@here` reached in a server channel, named or not; `None`
    /// without a granted `@here`. Stored so an edit can keep them.
    pub here_users: Option<Vec<Uuid>>,
}

/// Escaped with a backslash (`\@everyone`, `\<@id>`): plain text, as the
/// web client's Markdown shows it.
fn escaped(text: &str, at: usize) -> bool {
    text[..at].chars().rev().take_while(|c| *c == '\\').count() % 2 == 1
}

/// `@word` on its own: not inside an address (`a@everyone.org`), a path
/// (`mastodon.social/@here`), a longer word (`@everyoneelse`) or after another `@` (`@@here`), and not
/// escaped.
fn has_keyword(text: &str, word: &str) -> bool {
    let needle = format!("@{word}");
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        let word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        !before.is_some_and(|c| word_char(c) || c == '@' || c == '/')
            && !after.is_some_and(|c| word_char(c) || c == '@')
            && !escaped(text, at)
    })
}

pub fn parse(content: &str) -> Parsed {
    let text = prose(content);
    let mut users = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<@") {
        let plain = escaped(rest, start);
        rest = &rest[start + 2..];
        let Some(end) = rest.find('>') else { break };
        if !plain
            && let Ok(id) = Uuid::parse_str(&rest[..end])
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
/// `granted` is what the message reached before an edit.
pub(crate) async fn resolve(
    state: &AppState,
    access: &MessagingChannel,
    author: Uuid,
    content: &str,
    granted: Granted,
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
                here_users: None,
            })
        }
        MessagingChannel::Server { member } => {
            // `@everyone` / `@here` need the right and stay within a few per
            // window; otherwise the message goes out without the ping. The
            // limiter is in memory, so with several API nodes it counts per
            // node. An edit keeping a ping it already had is not counted.
            let kept = if parsed.everyone {
                granted.everyone
            } else {
                granted.here
            };
            let crowd = (parsed.everyone || parsed.here)
                && member.can(Permission::MentionEveryone)
                && (kept
                    || state
                        .limiter
                        .hit(
                            "mention-everyone",
                            &format!("{}:{author}", member.server.id),
                            EVERYONE_PER_WINDOW,
                            EVERYONE_WINDOW,
                        )
                        .is_ok());
            let everyone = crowd && parsed.everyone;
            let here = crowd && parsed.here && !parsed.everyone;
            let mut reached = Vec::new();
            if here && let Some(before) = &granted.here_users {
                // An edit keeping `@here`: the same people as before.
                reached.extend(before.iter().copied());
            } else if here {
                // Best effort: without Redis `@here` reaches nobody, the
                // message itself still goes out.
                match state.gateway.presence_snapshot(member.server.id).await {
                    Ok(online) => reached.extend(
                        online
                            .into_iter()
                            .filter(|entry| {
                                entry.st != PresenceStatus::Offline && entry.u != author
                            })
                            .map(|entry| entry.u)
                            .take(MAX_HERE),
                    ),
                    Err(error) => {
                        tracing::debug!(error = error.code(), "@here without presence")
                    }
                }
            }
            let mut wanted = parsed.users;
            wanted.extend(reached.iter().copied());
            let users: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM server_members WHERE server_id=$1 AND user_id=ANY($2) \
                 AND user_id<>$3 ORDER BY user_id",
            )
            .bind(member.server.id)
            .bind(&wanted)
            .bind(author)
            .fetch_all(&state.db)
            .await?;
            // Members only: someone who left since an earlier `@here` drops out.
            let here_users = here.then(|| {
                users
                    .iter()
                    .filter(|id| reached.contains(id))
                    .copied()
                    .collect()
            });
            Ok(Resolved {
                users,
                everyone,
                here_users,
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
    fn granted_reads_here_from_mentions_beyond_the_named() {
        let named = format!("@here <@{A}>");
        assert_eq!(
            Granted::of(&named, false, &ids(&[A]), None),
            Granted::default()
        );
        assert_eq!(
            Granted::of(&named, false, &ids(&[A, B]), None),
            Granted {
                everyone: false,
                here: true,
                here_users: Some(ids(&[B])),
            }
        );
        assert_eq!(
            Granted::of("@everyone", true, &[], None),
            Granted {
                everyone: true,
                here: true,
                here_users: None,
            }
        );
    }

    /// The stored record wins: a named user `@here` also reached stays, and
    /// an `@here` that reached nobody is still granted.
    #[test]
    fn granted_prefers_the_stored_here_users() {
        let named = format!("@here <@{A}>");
        assert_eq!(
            Granted::of(&named, false, &ids(&[A, B]), Some(&ids(&[A, B]))),
            Granted {
                everyone: false,
                here: true,
                here_users: Some(ids(&[A, B])),
            }
        );
        assert_eq!(
            Granted::of("@here", false, &[], Some(&[])),
            Granted {
                everyone: false,
                here: true,
                here_users: Some(Vec::new()),
            }
        );
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
        assert!(!parse("@@here").here);
        assert!(!parse("https://mastodon.social/@here").here);
        assert!(!parse("see https://example.org/?to=@everyone").everyone);
        assert!(!parse("mastodon.social/@here").here);
        assert!(parse("https://example.org @here").here);
    }

    /// A ping inside text the web client draws as a link is part of the
    /// link; anything it does not link is text and pings (#276).
    #[test]
    fn links_follow_the_web_client() {
        assert!(!parse("<https://example.org/?q=@here>").here);
        assert!(!parse("<mailto:x.@here>").here);
        assert!(!parse("https://x`c`@here").here);
        assert!(!parse("(https://example.org/@here)").here);
        assert!(parse("ftp://x/?q=@here").here);
        assert!(parse("https://x.org\"@here").here);
        assert!(parse("xhttps://x.org?q=@here").here);
        assert!(parse("https:// @here").here);
        // An escaped backtick opens no code.
        assert!(parse("\\`@here`").here);
    }

    #[test]
    fn escaped_mentions_are_text() {
        assert!(!parse(r"\@everyone").everyone);
        assert!(parse(r"\\@everyone").everyone);
        assert!(parse(&format!(r"\<@{A}>")).users.is_empty());
        assert_eq!(parse(&format!(r"\<@{A}> <@{B}>")).users, ids(&[B]));
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
