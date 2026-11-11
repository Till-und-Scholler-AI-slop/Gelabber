//! Every shape in `models.rs` against JSON captured from a live v0.8.9 stack
//! (web/scripts/parity seed plus a scratch server): decoding must succeed
//! and encoding must give back the same JSON.

use gelabber_client::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// Decodes `name` as `T` and checks the round trip is lossless.
fn round_trip<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let json = fixture(name);
    let value: T = serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let back = serde_json::to_value(&value).unwrap();
    assert_eq!(back, json, "{name} does not round-trip");
    value
}

#[test]
fn auth_and_account() {
    let session: SessionResponse = round_trip("session");
    assert_eq!(session.user.unwrap().name, "Lena");
    let me: User = round_trip("me");
    assert_eq!(me.email, "lena@example.test");
    // From a server older than v0.10: no language, and none written back.
    assert_eq!(me.locale, None);
    let mut english = fixture("me");
    english["locale"] = "en".into();
    let user: User = serde_json::from_value(english.clone()).unwrap();
    assert_eq!(user.locale.as_deref(), Some("en"));
    assert_eq!(serde_json::to_value(&user).unwrap(), english);
    let saved: LocaleBody = serde_json::from_value(serde_json::json!({"locale": "en"})).unwrap();
    assert_eq!(saved.locale, "en");
    let profile: PublicProfile = round_trip("user_profile");
    assert_eq!(profile.name, "Jonas");
    let mail: MailStatus = round_trip("mail");
    assert!(!mail.enabled);
    let themes: ThemeDocument = round_trip("themes");
    assert_eq!(themes.active, "dark");
    let privacy: NotificationPrivacy = round_trip("notification_privacy");
    assert!(!privacy.hide_content);
    let gifs: GifStatus = round_trip("gifs");
    assert!(!gifs.enabled && gifs.provider.is_none());
}

#[test]
fn servers_channels_members_invites() {
    let servers: Vec<Server> = round_trip("servers");
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(&names[..2], ["Wohnzimmer", "Spieleabend"]);
    assert_eq!(servers[0].role, Role::Owner);
    assert!(servers[0].can(Permission::ManageServer));
    assert_eq!(servers[1].role, Role::Member);
    assert!(!servers[1].can(Permission::ManageServer));

    let home: ServerDetail = round_trip("server_wohnzimmer");
    assert_eq!(home.members.len(), 5);
    let channels: Vec<(&str, ChannelKind)> = home
        .channels
        .iter()
        .map(|c| (c.name.as_str(), c.kind))
        .collect();
    assert!(channels.contains(&("allgemein", ChannelKind::Text)));
    assert!(channels.contains(&("Sofa", ChannelKind::Voice)));
    let created: ServerDetail = round_trip("server_create");
    assert_eq!(created.categories.len(), 2);
    let _: ServerDetail = round_trip("server_detail");
    let _: Invite = round_trip("invite");
    let _: Vec<Invite> = round_trip("invites");
    let preview: InvitePreview = round_trip("invite_preview");
    assert!(!preview.member && preview.server.member_count == 1);
    let _: Server = round_trip("join");
    let ticket: MediaTicket = round_trip("ticket");
    assert_eq!(ticket.media_path, "/media/ws");
    let presign: Presigned = round_trip("presign");
    assert_eq!(presign.headers["Content-Length"], "5");
}

#[test]
fn messages_and_dms() {
    let page: MessagePage = round_trip("messages_allgemein");
    assert!(!page.has_more);
    assert_eq!(page.messages.len(), 10);
    assert_eq!(page.messages[0].content, "Hallo zusammen! 👋");
    let reply = page.messages.iter().find_map(|m| m.reply.as_ref()).unwrap();
    assert_eq!(reply.author.as_ref().unwrap().name, "Mira");
    let code = page
        .messages
        .iter()
        .find(|m| m.content.contains("```"))
        .unwrap();
    assert_eq!(code.reactions.len(), 2);
    let _: MessagePage = round_trip("messages_dm");
    let _: MessagePage = round_trip("search");
    let context: MessageContext = round_trip("context");
    assert!(context.messages.iter().any(|m| m.id == context.target_id));
    let created: Message = round_trip("message_create");
    assert!(created.nonce.is_some());
    let replied: Message = round_trip("message_reply");
    assert_eq!(replied.mentions.len(), 1);
    assert_eq!(
        replied.reply.unwrap().excerpt.as_deref(),
        Some("Erste **Nachricht** ▒▒▒")
    );
    let reacted: Message = round_trip("reaction");
    assert_eq!(reacted.reactions[0].emoji, "👍");
    let edited: Message = round_trip("edit");
    assert!(edited.edited_at.is_some() && edited.is_newer_than(&reacted));
    let dms: Vec<DirectMessage> = round_trip("dms");
    assert_eq!(dms[0].peer.name, "Jonas");
    let _: DirectMessage = round_trip("dm");
}

#[test]
fn read_state_and_notifications() {
    let rows: Vec<ReadState> = round_trip("unread_seeded");
    assert!(rows.iter().any(|r| r.server_id.is_none()));
    let _: Vec<ReadState> = round_trip("unread");
    let row: ReadState = round_trip("read");
    assert_eq!(row.unread_count, 0);
    let setting: NotificationSetting = round_trip("ns_set");
    assert_eq!(setting.level, Some(NotificationLevel::Mentions));
    assert!(setting.muted && setting.muted_until.is_some());
    let _: Vec<NotificationSetting> = round_trip("ns_list");
}

#[test]
fn gateway_frames() {
    let frames = fixture("frames");
    let mut ops = std::collections::BTreeSet::new();
    for frame in frames.as_array().unwrap() {
        let decoded: ServerFrame =
            serde_json::from_value(frame.clone()).unwrap_or_else(|e| panic!("{frame}: {e}"));
        assert_ne!(decoded, ServerFrame::Unknown, "{frame}");
        assert_eq!(serde_json::to_value(&decoded).unwrap(), *frame);
        if let ServerFrame::Event(event) = &decoded
            && event.c.is_some()
            && event.t != EventKind::D
        {
            let message = event.message().expect("message payload");
            assert_eq!(Some(message.revision), event.r);
        }
        ops.insert(frame["op"].as_str().unwrap().to_owned());
    }
    for op in ["ok", "p", "sig", "y", "e", "rd", "ns", "lc"] {
        assert!(ops.contains(op), "no {op} frame captured");
    }
    let locale: ServerFrame = serde_json::from_str(r#"{"op":"lc","l":"en"}"#).unwrap();
    assert_eq!(locale, ServerFrame::Locale { l: "en".into() });
}

#[test]
fn live_events_accept_cjson_empty_objects() {
    let mut event = fixture("frames")
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["op"] == "e" && f["t"] == "c")
        .unwrap()
        .clone();
    event["d"]["attachments"] = serde_json::json!({});
    event["d"]["reactions"] = serde_json::json!({});
    event["d"]["mentions"] = serde_json::json!({});
    event["d"]["previews"] = serde_json::json!({});
    let ServerFrame::Event(event) = serde_json::from_value(event).unwrap() else {
        panic!("not an event");
    };
    let message = event.message().unwrap();
    assert!(message.attachments.is_empty() && message.reactions.is_empty());
}

/// A newer server: an unknown role, channel kind, status or event kind, or
/// one element the client cannot read, costs that element at most, never
/// the whole answer.
#[test]
fn a_newer_server_degrades_per_element() {
    let mut servers = fixture("servers");
    let list = servers.as_array_mut().unwrap();
    list[0]["role"] = "admin".into();
    list.push(serde_json::json!({"id": "nope"}));
    let count = list.len();
    let Lenient(decoded) = serde_json::from_value::<Lenient<Server>>(servers).unwrap();
    assert_eq!(decoded.len(), count - 1);
    assert_eq!(decoded[0].role, Role::Member);

    let mut detail = fixture("server_detail");
    let channels = detail["channels"].as_array_mut().unwrap();
    let count = channels.len();
    channels[0]["kind"] = "forum".into();
    let detail: ServerDetail = serde_json::from_value(detail).unwrap();
    assert_eq!(detail.channels.len(), count - 1);

    let mut unread = fixture("unread");
    let count = unread.as_array().unwrap().len();
    unread[0]["level"] = "digest".into();
    let Lenient(rows) = serde_json::from_value::<Lenient<ReadState>>(unread).unwrap();
    assert_eq!(rows.len(), count);
    assert_eq!(rows[0].level, NotificationLevel::Unknown);

    let frame: ServerFrame = serde_json::from_value(serde_json::json!({
        "op": "p", "s": "00000000-0000-0000-0000-000000000001",
        "snap": [
            {"u": "00000000-0000-0000-0000-000000000002", "st": "d"},
            {"u": "00000000-0000-0000-0000-000000000003", "st": "o"},
            {"u": 7, "st": "o"},
        ],
    }))
    .unwrap();
    let ServerFrame::Presence {
        snap: Some(snap), ..
    } = frame
    else {
        panic!("{frame:?}")
    };
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].st, PresenceStatus::Online);

    let frame: ServerFrame = serde_json::from_value(serde_json::json!({
        "op": "e", "t": "x", "s": "00000000-0000-0000-0000-000000000001", "n": 3,
    }))
    .unwrap();
    assert!(matches!(
        frame,
        ServerFrame::Event(EventFrame {
            t: EventKind::Unknown,
            n: 3,
            ..
        })
    ));
    let frame: ServerFrame = serde_json::from_value(serde_json::json!({
        "op": "sig", "t": "p", "s": "00000000-0000-0000-0000-000000000001",
        "c": "00000000-0000-0000-0000-000000000002",
        "u": "00000000-0000-0000-0000-000000000003", "k": "hologram",
    }))
    .unwrap();
    assert!(matches!(frame, ServerFrame::Sig(SigFrame { k: None, .. })));
    let frame: ServerFrame = serde_json::from_value(serde_json::json!({
        "op": "sig", "t": "w", "s": "00000000-0000-0000-0000-000000000001",
    }))
    .unwrap();
    assert!(matches!(
        frame,
        ServerFrame::Sig(SigFrame {
            t: SigKind::Unknown,
            ..
        })
    ));
}
