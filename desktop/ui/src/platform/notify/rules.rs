//! The web's rules for message notifications, without any UI:
//! messages/notify.ts, notifyLevel.ts, notificationSettings.ts
//! (`conversationSetting`) and notificationPrivacy.ts.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use gelabber_client::{NotificationLevel, NotificationSetting, ReadState, SettingScope};
use uuid::Uuid;

/// Level and mute of a conversation, from its unread row or its settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub level: NotificationLevel,
    pub muted: bool,
    pub muted_until: Option<DateTime<Utc>>,
}

impl Setting {
    pub fn of_row(row: &ReadState) -> Self {
        Self {
            level: row.level,
            muted: row.muted,
            muted_until: row.muted_until,
        }
    }

    /// `isMuted`: a mute whose end has passed is over.
    pub fn is_muted(&self, now: DateTime<Utc>) -> bool {
        self.muted && self.muted_until.is_none_or(|until| until > now)
    }
}

fn setting_muted(s: &NotificationSetting, now: DateTime<Utc>) -> bool {
    s.muted && s.muted_until.is_none_or(|until| until > now)
}

/// `conversationSetting`: the conversation's own level, else its server's,
/// muted while either mute holds.
pub fn conversation_setting(
    settings: &[NotificationSetting],
    channel: Uuid,
    server: Option<Uuid>,
    now: DateTime<Utc>,
) -> Setting {
    let find =
        |scope: SettingScope, id: Uuid| settings.iter().find(|s| s.scope == scope && s.id == id);
    let own = find(SettingScope::Channel, channel);
    let srv = server.and_then(|s| find(SettingScope::Server, s));
    let holding: Vec<&NotificationSetting> = [own, srv]
        .into_iter()
        .flatten()
        .filter(|s| setting_muted(s, now))
        .collect();
    let muted_until = if holding.is_empty() || holding.iter().any(|s| s.muted_until.is_none()) {
        None
    } else {
        holding.iter().filter_map(|s| s.muted_until).max()
    };
    Setting {
        level: own
            .and_then(|s| s.level)
            .or_else(|| srv.and_then(|s| s.level))
            .unwrap_or_default(),
        muted: !holding.is_empty(),
        muted_until,
    }
}

/// `wantsNotification`.
pub fn wants_notification(
    setting: Option<Setting>,
    dm: bool,
    mentioned: bool,
    now: DateTime<Utc>,
) -> bool {
    let Some(setting) = setting else {
        return true;
    };
    if setting.is_muted(now) {
        return false;
    }
    match setting.level {
        NotificationLevel::None => false,
        NotificationLevel::Mentions => dm || mentioned,
        NotificationLevel::All => true,
    }
}

/// The desktop half of `messageNotificationDecision`: on, the window not
/// in front, someone else's message.
pub fn wants_desktop(enabled: bool, in_front: bool, own: bool) -> bool {
    enabled && !in_front && !own
}

/// `conversationPath`.
pub fn conversation_path(dm: bool, server: Uuid, channel: Uuid) -> String {
    if dm {
        format!("/d/{channel}")
    } else {
        format!("/s/{server}/c/{channel}")
    }
}

/// pwa/notifications.ts `tagOf`: one per account and conversation; the
/// account's prefix closes all of them.
pub fn tag_of(user: Uuid, channel: Option<Uuid>) -> String {
    match channel {
        Some(c) => format!("gelabber:{user}:{c}"),
        None => format!("gelabber:{user}:"),
    }
}

/// `previewText`.
pub fn preview_text(content: &str, has_attachment: bool, gif: bool) -> String {
    let trimmed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if !trimmed.is_empty() {
        // UTF-16 units, as `String.length` counts them.
        let units: Vec<u16> = trimmed.encode_utf16().collect();
        if units.len() > 140 {
            return format!("{}…", String::from_utf16_lossy(&units[..137]));
        }
        return trimmed;
    }
    if gif {
        "GIF".into()
    } else if has_attachment {
        "Datei".into()
    } else {
        "Neue Nachricht".into()
    }
}

/// `readableMentions`: `<@id>` as `@Name`, unknown ones as "@Unbekannt".
pub fn readable_mentions(text: &str, names: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        if tail.len() > 36 && tail.as_bytes()[36] == b'>' && Uuid::parse_str(&tail[..36]).is_ok() {
            let id = tail[..36].to_ascii_lowercase();
            out.push('@');
            out.push_str(names.get(&id).map_or("Unbekannt", String::as_str));
            rest = &tail[37..];
        } else {
            out.push_str("<@");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// `messageNotificationText` / `notificationText`: only who wrote when the
/// account hides the message (#268).
pub fn notification_text(
    hide: bool,
    author: &str,
    mentioned: bool,
    label: &str,
    preview: &str,
) -> (String, String) {
    if hide {
        return (format!("Neue Nachricht von {author}"), String::new());
    }
    let mention = if mentioned { " hat dich erwähnt" } else { "" };
    (format!("{author}{mention} · {label}"), preview.to_owned())
}

/// `createNotificationDedupe`: each message once per account.
pub struct Dedupe {
    seen: HashSet<(Uuid, Uuid)>,
    order: VecDeque<(Uuid, Uuid)>,
    limit: usize,
}

impl Default for Dedupe {
    fn default() -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            limit: 2048,
        }
    }
}

impl Dedupe {
    pub fn first(&mut self, channel: Uuid, message: Uuid) -> bool {
        if !self.seen.insert((channel, message)) {
            return false;
        }
        self.order.push_back((channel, message));
        if self.order.len() > self.limit
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        true
    }
}

/// `createNotificationPacing`: one per key in `gap` milliseconds.
pub struct Pacing {
    taken: HashMap<Uuid, i64>,
    gap: i64,
}

impl Pacing {
    pub fn new(gap_ms: i64) -> Self {
        Self {
            taken: HashMap::new(),
            gap: gap_ms,
        }
    }

    pub fn take(&mut self, key: Uuid, now: i64) -> bool {
        if self
            .taken
            .get(&key)
            .is_some_and(|last| now - last < self.gap)
        {
            return false;
        }
        self.taken.insert(key, now);
        true
    }
}

impl Default for Pacing {
    fn default() -> Self {
        Self::new(5_000)
    }
}

/// Live deliveries only: replays older than this on the server clock do
/// not notify.
pub const LIVE_WINDOW_MS: i64 = 30_000;

/// What the window and the switches say about a Live notice (`LiveDevice`).
#[derive(Clone, Copy, Debug)]
pub struct LiveDevice {
    /// The `messageToasts` switch.
    pub toasts: bool,
    /// The `desktopNotify` switch.
    pub desktop: bool,
    /// Focused, not minimized and not in the tray.
    pub in_front: bool,
    /// The channel this window shows.
    pub viewing: Option<Uuid>,
}

/// `liveNoticeDecision`: (toast, system notification) for a new Live. Not
/// my own, not in the voice channel I sit in (`here`), only in a voice
/// channel of a server I may join, and only at the server's level "all"
/// (a Live mentions nobody); then as a message in that channel would be.
pub fn live_decision(
    state: &gelabber_client::State,
    start: &gelabber_client::store::LiveStart,
    here: bool,
    device: LiveDevice,
    now: DateTime<Utc>,
) -> (bool, bool) {
    let none = (false, false);
    if state.me().map(|m| m.id) == Some(start.user) || here {
        return none;
    }
    let Some(detail) = state.detail(start.server) else {
        return none;
    };
    let voice = detail
        .channels
        .iter()
        .any(|c| c.id == start.channel && c.kind == gelabber_client::ChannelKind::Voice);
    let may_join = detail.server.role == gelabber_client::Role::Owner
        || detail.server.can(gelabber_client::Permission::JoinVoice);
    if !voice || !may_join {
        return none;
    }
    let setting = state
        .notification_settings
        .iter()
        .find(|s| s.scope == SettingScope::Server && s.id == start.server)
        .map(|s| Setting {
            level: s.level.unwrap_or_default(),
            muted: s.muted,
            muted_until: s.muted_until,
        });
    if !wants_notification(setting, false, false, now) {
        return none;
    }
    (
        device.toasts && device.viewing != Some(start.channel),
        wants_desktop(device.desktop, device.in_front, false),
    )
}

/// `liveText`: "Ada ist live in Lounge.".
pub fn live_text(author: &str, voice: &str) -> String {
    let sentence = crate::screens::voice::summary::live_sentence(&[author.to_owned()], false);
    format!("{sentence} in {voice}.")
}

/// `liveNotificationText`: with message content hidden (#268) only who,
/// not where.
pub fn live_notification_text(
    hide: bool,
    author: &str,
    server: &str,
    text: &str,
) -> (String, String) {
    if hide {
        let sentence = crate::screens::voice::summary::live_sentence(&[author.to_owned()], false);
        return (sentence, String::new());
    }
    (format!("{author} · {server}"), text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod live {
        use super::*;
        use gelabber_client::store::LiveStart;
        use gelabber_client::{NotificationLevel, NotificationSetting, Session, State};

        fn id(n: u128) -> Uuid {
            Uuid::from_u128(n)
        }

        const ME: u128 = 7;
        const SRV: u128 = 1;
        const LOUNGE: u128 = 2;
        const CHAT: u128 = 4;
        const ADA: u128 = 3;

        fn state(role: &str, permissions: &[&str]) -> State {
            let mut s = State::default();
            s.session = Session::Authenticated(Box::new(
                serde_json::from_value(serde_json::json!({
                    "id": id(ME), "email": "ich@example.test", "name": "Ich",
                    "created_at": "2026-10-01T00:00:00Z",
                }))
                .unwrap(),
            ));
            let channel = |n: u128, name: &str, kind: &str| {
                serde_json::json!({
                    "id": id(n), "server_id": id(SRV), "category_id": null, "name": name,
                    "kind": kind, "created_at": "2026-10-01T00:00:00Z",
                })
            };
            let detail = serde_json::from_value(serde_json::json!({
                "id": id(SRV), "name": "Runde", "owner_id": id(99), "icon_url": null,
                "created_at": "2026-10-01T00:00:00Z", "role": role,
                "permissions": permissions, "member_permissions": permissions,
                "categories": [],
                "channels": [channel(LOUNGE, "Lounge", "voice"), channel(CHAT, "allgemein", "text")],
                "members": [{
                    "user_id": id(ADA), "name": "Ada", "avatar_url": null,
                    "joined_at": "2026-10-01T00:00:00Z", "role": "member",
                }],
            }))
            .unwrap();
            s.server_details.insert(id(SRV), detail);
            s
        }

        fn start(user: u128, channel: u128) -> LiveStart {
            LiveStart {
                seq: 1,
                server: id(SRV),
                channel: id(channel),
                user: id(user),
                heard_ms: 0,
                at: chrono::Utc::now(),
            }
        }

        /// A hidden window with toasts and system notifications on.
        const HIDDEN: LiveDevice = LiveDevice {
            toasts: true,
            desktop: true,
            in_front: false,
            viewing: None,
        };

        fn at(s: &str) -> DateTime<Utc> {
            s.parse().unwrap()
        }

        // liveNotices.test.ts "whether a new Live is announced".
        #[test]
        fn someone_elses_live_where_i_am_not_is_announced() {
            let s = state("member", &["join_voice", "send_messages"]);
            let now = chrono::Utc::now();
            assert_eq!(
                live_decision(&s, &start(ADA, LOUNGE), false, HIDDEN, now),
                (true, true)
            );
            assert_eq!(
                live_decision(&s, &start(ME, LOUNGE), false, HIDDEN, now),
                (false, false)
            );
            assert_eq!(
                live_decision(&s, &start(ADA, LOUNGE), true, HIDDEN, now),
                (false, false)
            );
        }

        #[test]
        fn only_the_servers_level_all_announces_and_a_mute_holds() {
            let now = at("2026-10-29T12:00:00Z");
            let decide = |level: Option<NotificationLevel>, muted: bool, until: Option<&str>| {
                let mut s = state("member", &["join_voice"]);
                s.notification_settings = vec![NotificationSetting {
                    scope: SettingScope::Server,
                    id: id(SRV),
                    level,
                    muted,
                    muted_until: until.map(at),
                }];
                live_decision(&s, &start(ADA, LOUNGE), false, HIDDEN, now).0
            };
            assert!(decide(Some(NotificationLevel::All), false, None));
            assert!(!decide(Some(NotificationLevel::Mentions), false, None));
            assert!(!decide(Some(NotificationLevel::None), false, None));
            assert!(!decide(None, true, None));
            assert!(!decide(None, true, Some("2026-10-29T13:00:00Z")));
            // A mute that has run out is over.
            assert!(decide(None, true, Some("2026-10-29T11:00:00Z")));
        }

        #[test]
        fn it_needs_a_voice_channel_of_a_server_i_may_join() {
            let now = chrono::Utc::now();
            let none = (false, false);
            assert_eq!(
                live_decision(&State::default(), &start(ADA, LOUNGE), false, HIDDEN, now),
                none
            );
            let s = state("member", &["send_messages"]);
            assert_eq!(
                live_decision(&s, &start(ADA, LOUNGE), false, HIDDEN, now),
                none
            );
            let owner = state("owner", &[]);
            assert!(live_decision(&owner, &start(ADA, LOUNGE), false, HIDDEN, now).0);
            let s = state("member", &["join_voice"]);
            assert_eq!(live_decision(&s, &start(ADA, 66), false, HIDDEN, now), none);
            assert_eq!(
                live_decision(&s, &start(ADA, CHAT), false, HIDDEN, now),
                none
            );
        }

        #[test]
        fn the_switches_decide_like_for_a_message_in_that_channel() {
            let s = state("member", &["join_voice"]);
            let now = chrono::Utc::now();
            let device = |patch: fn(&mut LiveDevice)| {
                let mut d = HIDDEN;
                patch(&mut d);
                live_decision(&s, &start(ADA, LOUNGE), false, d, now)
            };
            assert_eq!(device(|d| d.toasts = false), (false, true));
            assert_eq!(device(|d| d.desktop = false), (true, false));
            // In front, the toast is enough; the room itself needs neither.
            assert_eq!(device(|d| d.in_front = true), (true, false));
            assert_eq!(
                device(|d| {
                    d.in_front = true;
                    d.viewing = Some(id(LOUNGE));
                }),
                (false, false)
            );
        }

        // "the words".
        #[test]
        fn the_words_say_who_is_live_where() {
            let text = live_text("Ada", "Lounge");
            assert_eq!(text, "Ada ist live in Lounge.");
            assert_eq!(
                live_notification_text(false, "Ada", "Runde", &text),
                ("Ada · Runde".into(), "Ada ist live in Lounge.".into())
            );
            assert_eq!(
                live_notification_text(true, "Ada", "Runde", &text),
                ("Ada ist live".into(), String::new())
            );
        }
    }
    use chrono::Duration;

    fn setting(
        scope: SettingScope,
        id: Uuid,
        level: Option<NotificationLevel>,
        muted: bool,
        until: Option<DateTime<Utc>>,
    ) -> NotificationSetting {
        NotificationSetting {
            scope,
            id,
            level,
            muted,
            muted_until: until,
        }
    }

    #[test]
    fn levels_and_mutes_as_the_web_decides() {
        let now = Utc::now();
        let all = Setting {
            level: NotificationLevel::All,
            muted: false,
            muted_until: None,
        };
        assert!(wants_notification(None, false, false, now));
        assert!(wants_notification(Some(all), false, false, now));
        let mentions = Setting {
            level: NotificationLevel::Mentions,
            ..all
        };
        assert!(!wants_notification(Some(mentions), false, false, now));
        assert!(wants_notification(Some(mentions), false, true, now));
        assert!(wants_notification(Some(mentions), true, false, now));
        let none = Setting {
            level: NotificationLevel::None,
            ..all
        };
        assert!(!wants_notification(Some(none), true, true, now));
        let muted = Setting { muted: true, ..all };
        assert!(!wants_notification(Some(muted), true, true, now));
        let ended = Setting {
            muted: true,
            muted_until: Some(now - Duration::minutes(1)),
            ..all
        };
        assert!(wants_notification(Some(ended), false, false, now));
    }

    #[test]
    fn a_channel_inherits_its_servers_level_and_either_mute_holds() {
        let now = Utc::now();
        let (s, c) = (Uuid::new_v4(), Uuid::new_v4());
        let later = now + Duration::hours(1);
        let latest = now + Duration::hours(2);
        let list = vec![
            setting(
                SettingScope::Server,
                s,
                Some(NotificationLevel::Mentions),
                true,
                Some(later),
            ),
            setting(SettingScope::Channel, c, None, true, Some(latest)),
        ];
        let got = conversation_setting(&list, c, Some(s), now);
        assert_eq!(got.level, NotificationLevel::Mentions);
        assert!(got.muted);
        assert_eq!(got.muted_until, Some(latest));
        // A DM has no server.
        let dm = conversation_setting(&list, Uuid::new_v4(), None, now);
        assert_eq!(dm.level, NotificationLevel::All);
        assert!(!dm.muted);
        // A mute without end wins.
        let forever = vec![
            setting(SettingScope::Server, s, None, true, None),
            setting(
                SettingScope::Channel,
                c,
                Some(NotificationLevel::All),
                true,
                Some(later),
            ),
        ];
        let got = conversation_setting(&forever, c, Some(s), now);
        assert!(got.muted && got.muted_until.is_none());
        assert_eq!(got.level, NotificationLevel::All);
    }

    #[test]
    fn texts_previews_and_paths() {
        assert_eq!(preview_text("  hallo \n  du ", false, false), "hallo du");
        assert_eq!(preview_text("", true, false), "Datei");
        assert_eq!(preview_text(" ", true, true), "GIF");
        assert_eq!(preview_text("", false, false), "Neue Nachricht");
        let long = "ä".repeat(200);
        let cut = preview_text(&long, false, false);
        assert_eq!(cut.chars().count(), 138);
        assert!(cut.ends_with('…'));
        assert_eq!(
            notification_text(false, "Anna", true, "#allgemein", "hi"),
            ("Anna hat dich erwähnt · #allgemein".into(), "hi".into())
        );
        assert_eq!(
            notification_text(true, "Anna", true, "#allgemein", "hi"),
            ("Neue Nachricht von Anna".into(), String::new())
        );
        let (s, c) = (Uuid::nil(), Uuid::max());
        assert_eq!(conversation_path(true, c, c), format!("/d/{c}"));
        assert_eq!(conversation_path(false, s, c), format!("/s/{s}/c/{c}"));
        assert_eq!(tag_of(s, None), format!("gelabber:{s}:"));
        assert!(tag_of(s, Some(c)).starts_with(&tag_of(s, None)));
    }

    #[test]
    fn once_per_message_and_once_per_five_seconds() {
        let mut dedupe = Dedupe::default();
        let (c, m) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(dedupe.first(c, m));
        assert!(!dedupe.first(c, m));
        let mut pacing = Pacing::default();
        assert!(pacing.take(c, 0));
        assert!(!pacing.take(c, 4_999));
        assert!(pacing.take(Uuid::new_v4(), 1));
        assert!(pacing.take(c, 5_000));
        assert!(wants_desktop(true, false, false));
        assert!(!wants_desktop(true, true, false));
        assert!(!wants_desktop(true, false, true));
        assert!(!wants_desktop(false, false, false));
    }
}
