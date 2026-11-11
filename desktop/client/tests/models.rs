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

/// The sign-in-through-a-provider shapes (#518), as
/// backend/api/src/auth/oidc/routes.rs writes them and
/// backend/api/tests/oidc.rs checks them.
#[test]
fn sign_in_through_a_provider() {
    use serde_json::json;
    let providers: AuthProviders =
        serde_json::from_value(json!({"providers": [{"id": "google", "name": "Google"}]})).unwrap();
    assert_eq!(providers.providers[0].name, "Google");
    let off: AuthProviders = serde_json::from_value(json!({"providers": []})).unwrap();
    assert!(off.providers.is_empty());

    let methods = json!({
        "password": false,
        "providers": [
            {"id": "google", "name": "Google", "configured": true,
             "connected": {"email": "lena@example.test", "since": "2026-11-10T10:00:00Z"}},
            {"id": "old", "name": "old", "configured": false, "connected": null},
        ],
    });
    let read: SignInMethods = serde_json::from_value(methods.clone()).unwrap();
    assert!(!read.password);
    let google = read.providers[0].connected.as_ref().unwrap();
    assert_eq!(google.email, "lena@example.test");
    assert!(!read.providers[1].configured && read.providers[1].connected.is_none());
    assert_eq!(serde_json::to_value(&read).unwrap(), methods);

    let pending = json!({
        "provider": {"id": "google", "name": "Google"},
        "email": "neu@example.test",
        "name": null,
    });
    let read: OidcPending = serde_json::from_value(pending.clone()).unwrap();
    assert_eq!(read.name, None);
    assert_eq!(serde_json::to_value(&read).unwrap(), pending);

    let mut session = fixture("session");
    session["redirect"] = "/invite/abc123".into();
    let complete: OidcCompleteResponse = serde_json::from_value(session.clone()).unwrap();
    assert_eq!(complete.user.name, "Lena");
    assert_eq!(complete.redirect.as_deref(), Some("/invite/abc123"));
    assert_eq!(serde_json::to_value(&complete).unwrap(), session);

    // The exchange, by `result`.
    let mut signed_in = fixture("session");
    signed_in["result"] = "signed_in".into();
    let read: OidcExchange = serde_json::from_value(signed_in.clone()).unwrap();
    assert!(matches!(&read, OidcExchange::SignedIn(s) if s.user.is_some()));
    assert_eq!(serde_json::to_value(&read).unwrap(), signed_in);
    let mut needs_name = pending.clone();
    needs_name["result"] = "needs_name".into();
    needs_name["name"] = "Lena".into();
    let read: OidcExchange = serde_json::from_value(needs_name.clone()).unwrap();
    assert!(matches!(&read, OidcExchange::NeedsName(p) if p.name.as_deref() == Some("Lena")));
    assert_eq!(serde_json::to_value(&read).unwrap(), needs_name);
    let mut linked = methods;
    linked["result"] = "linked".into();
    let read: OidcExchange = serde_json::from_value(linked.clone()).unwrap();
    assert!(matches!(&read, OidcExchange::Linked(m) if !m.password));
    assert_eq!(serde_json::to_value(&read).unwrap(), linked);

    // The start body: only what is set, with the app's loopback fields.
    let mut start = OidcStart::new(OidcPurpose::Link);
    start.current_password = Some("geheim".into());
    assert_eq!(
        serde_json::to_value(&start).unwrap(),
        json!({"purpose": "link", "current_password": "geheim"})
    );
    assert!(!format!("{start:?}").contains("geheim"));
    assert_eq!(
        serde_json::to_value(OidcStart::new(OidcPurpose::SignIn)).unwrap(),
        json!({"purpose": "sign_in"})
    );
}

/// The operator package's shapes (#517), as backend/api/src/auth/mod.rs
/// and servers/mod.rs write them; older servers send none of the new keys.
#[test]
fn operator_flags_registration_and_admission_codes() {
    use serde_json::json;
    let old: SessionResponse = round_trip("session");
    assert!(!old.operator);
    let mut session = fixture("session");
    session["operator"] = true.into();
    let read: SessionResponse = serde_json::from_value(session.clone()).unwrap();
    assert!(read.operator);
    assert_eq!(serde_json::to_value(&read).unwrap(), session);
    let registered: RegisterResponse = serde_json::from_value(session).unwrap();
    assert!(registered.operator && !registered.verification_sent);

    let servers: Vec<Server> = round_trip("servers");
    assert!(servers.iter().all(|s| !s.blocked));
    let mut blocked = fixture("join");
    blocked["blocked"] = true.into();
    let read: Server = serde_json::from_value(blocked.clone()).unwrap();
    assert!(read.blocked);
    assert_eq!(serde_json::to_value(&read).unwrap(), blocked);

    for (mode, expected) in [
        ("open", RegistrationMode::Open),
        ("invitation", RegistrationMode::Invitation),
    ] {
        let read: RegistrationStatus = serde_json::from_value(json!({"mode": mode})).unwrap();
        assert_eq!(read.mode, expected);
    }

    // Only the codes that are there go out, and the log shows none.
    let mut body = RegisterBody::new("neu@example.test", "geheim123", "Neu");
    assert_eq!(
        serde_json::to_value(&body).unwrap(),
        json!({"email": "neu@example.test", "password": "geheim123", "name": "Neu", "locale": "de"})
    );
    body.invitation = Some("k7m2p9qrs4tx".into());
    body.server_invite = Some("abcdefghjkmn".into());
    let sent = serde_json::to_value(&body).unwrap();
    assert_eq!(sent["invitation"], "k7m2p9qrs4tx");
    assert_eq!(sent["server_invite"], "abcdefghjkmn");
    let logged = format!("{body:?}");
    for secret in ["geheim123", "k7m2p9qrs4tx", "abcdefghjkmn"] {
        assert!(!logged.contains(secret), "{logged}");
    }
}

/// The directory's shapes (#520), as backend/api/src/directory and
/// backend/api/tests/directory.rs write them.
#[test]
fn directory_listings_settings_requests_and_reports() {
    use serde_json::json;
    let round = |value: &Value| assert_eq!(serde_json::to_value(value).unwrap(), *value);

    let status = json!({"enabled": true, "languages": [
        {"code": "de", "name": "Deutsch", "listings": 3},
        {"code": "uk", "name": "Українська", "listings": 0},
    ]});
    let read: DirectoryStatus = serde_json::from_value(status.clone()).unwrap();
    assert!(read.enabled);
    assert_eq!(read.languages[1].name, "Українська");
    assert_eq!(serde_json::to_value(&read).unwrap(), status);
    let off: DirectoryStatus =
        serde_json::from_value(json!({"enabled": false, "languages": []})).unwrap();
    assert!(!off.enabled && off.languages.is_empty());

    let filmclub = json!({
        "server_id": "00000000-0000-0000-0000-000000000001",
        "name": "Filmclub",
        "icon_url": null,
        "description": "Jeden Freitag ein Film.",
        "language": "de",
        "member_count": 2,
        "join_mode": "open",
        "member": false,
        "request": null,
        "retry_at": null,
    });
    let declined = json!({
        "server_id": "00000000-0000-0000-0000-000000000002",
        "name": "Modellbau-Keller",
        "icon_url": "/api/server-icons/00000000-0000-0000-0000-000000000002/a.png",
        "description": "Kleine Dinge, große Geduld.",
        "language": "de",
        "member_count": 48,
        "join_mode": "request",
        "member": false,
        "request": "declined",
        "retry_at": "2026-11-18T10:00:00Z",
    });
    let page = json!({"listings": [filmclub, declined], "next_offset": 30});
    let read: ListingPage = serde_json::from_value(page.clone()).unwrap();
    assert_eq!(read.next_offset, Some(30));
    assert_eq!(read.listings[0].join_mode, JoinMode::Open);
    assert_eq!(read.listings[0].request, None);
    assert_eq!(read.listings[1].join_mode, JoinMode::Request);
    assert_eq!(read.listings[1].request, Some(RequestState::Declined));
    assert!(read.listings[1].retry_at.is_some());
    assert_eq!(serde_json::to_value(&read).unwrap(), page);
    // A row this client cannot read costs only that row; the last page
    // has no next offset.
    let mut odd = filmclub.clone();
    odd["join_mode"] = "invite_only".into();
    let read: ListingPage =
        serde_json::from_value(json!({"listings": [odd, filmclub], "next_offset": null})).unwrap();
    assert_eq!(read.listings.len(), 1);
    assert_eq!(read.next_offset, None);
    for (value, expected) in [
        ("pending", RequestState::Pending),
        ("declined", RequestState::Declined),
    ] {
        let mut row = declined.clone();
        row["request"] = value.into();
        let listing: Listing = serde_json::from_value(row).unwrap();
        assert_eq!(listing.request, Some(expected));
    }

    // The form: the defaults of a server without a row, and a saved one.
    let fresh = json!({"listed": false, "description": "", "language": null,
        "join_mode": "request", "delisted": false, "listed_at": null});
    let read: ListingSettings = serde_json::from_value(fresh.clone()).unwrap();
    assert_eq!((read.language, read.join_mode), (None, JoinMode::Request));
    round(&fresh);
    let saved = json!({"listed": true, "description": "Gemütliche Abende mit Freunden.",
        "language": "de", "join_mode": "open", "delisted": true,
        "listed_at": "2026-11-10T10:00:00Z"});
    let read: ListingSettings = serde_json::from_value(saved.clone()).unwrap();
    assert!(read.listed && read.delisted);
    assert_eq!(serde_json::to_value(&read).unwrap(), saved);
    let body = ListingSettingsBody {
        listed: true,
        description: "Gemütliche Abende mit Freunden.".into(),
        language: "de".into(),
        join_mode: JoinMode::Request,
    };
    assert_eq!(
        serde_json::to_value(&body).unwrap(),
        json!({"listed": true, "description": "Gemütliche Abende mit Freunden.",
            "language": "de", "join_mode": "request"})
    );

    let answer: RequestAnswer = serde_json::from_value(json!({"request": "pending"})).unwrap();
    assert_eq!(answer.request, RequestState::Pending);

    let queue = json!([{"user_id": "00000000-0000-0000-0000-000000000003", "name": "Kai",
        "avatar_url": null, "message": "Ich baue Schiffe.",
        "created_at": "2026-11-10T10:00:00Z"}]);
    let read: Vec<JoinRequest> = serde_json::from_value(queue.clone()).unwrap();
    assert_eq!(read[0].name, "Kai");
    assert_eq!(serde_json::to_value(&read).unwrap(), queue);

    // Reports: `details` only when given.
    assert_eq!(
        serde_json::to_value(ReportBody {
            reason: ReportReason::Spam,
            details: None
        })
        .unwrap(),
        json!({"reason": "spam"})
    );
    assert_eq!(
        serde_json::to_value(ReportBody {
            reason: ReportReason::Other,
            details: Some("Werbung für Fremdes".into())
        })
        .unwrap(),
        json!({"reason": "other", "details": "Werbung für Fremdes"})
    );
    for (reason, text) in [
        (ReportReason::Harassment, "harassment"),
        (ReportReason::Sexual, "sexual"),
        (ReportReason::Illegal, "illegal"),
    ] {
        assert_eq!(serde_json::to_value(reason).unwrap(), text);
    }
    let filed: ReportFiled = serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-000000000004", "created_at": "2026-11-10T10:00:00Z"}))
    .unwrap();
    assert_eq!(filed.id.as_u128(), 4);
}

/// The account's devices, export and deletion (v0.10, #519).
#[test]
fn devices_export_and_deletion() {
    use serde_json::json;
    let devices = json!({"sessions": [
        {"id": "00000000-0000-0000-0000-000000000001", "client": "desktop", "os": "linux",
         "created_at": "2026-11-01T18:00:00Z", "last_used_at": "2026-11-10T19:02:00Z",
         "current": true},
        {"id": "00000000-0000-0000-0000-000000000002", "client": "firefox", "os": "windows",
         "created_at": "2026-11-01T18:00:00Z", "last_used_at": "2026-11-09T19:02:00Z",
         "current": false}]});
    let row: DeviceSession = serde_json::from_value(devices["sessions"][0].clone()).unwrap();
    assert_eq!(
        (row.client, row.os, row.current),
        (DeviceClient::Desktop, DeviceOs::Linux, true)
    );
    assert_eq!(serde_json::to_value(&row).unwrap(), devices["sessions"][0]);
    // A device a newer server knows reads as unknown, not as an error.
    let newer: DeviceSession = serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-000000000003", "client": "opera", "os": "haiku",
        "created_at": "2026-11-01T18:00:00Z", "last_used_at": "2026-11-01T18:00:00Z",
        "current": false}))
    .unwrap();
    assert_eq!(
        (newer.client, newer.os),
        (DeviceClient::Unknown, DeviceOs::Other)
    );

    let ready = json!({"export": {"state": "ready", "requested_at": "2026-11-10T22:00:00Z",
        "finished_at": "2026-11-11T00:10:00Z", "expires_at": "2026-11-18T00:10:00Z",
        "size_bytes": 123456}, "mail": true});
    let status: ExportStatus = serde_json::from_value(ready.clone()).unwrap();
    assert_eq!(serde_json::to_value(&status).unwrap(), ready);
    assert!(!status.building());
    let view = status.export.unwrap();
    assert_eq!(view.size_bytes, Some(123_456));
    assert_eq!(view.file_name(), "gelabber-export-2026-11-11.zip");
    let pending: ExportStatus = serde_json::from_value(json!({"export": {"state": "pending",
        "requested_at": "2026-11-10T22:00:00Z", "finished_at": null, "expires_at": null,
        "size_bytes": null}, "mail": false}))
    .unwrap();
    assert!(pending.building());
    let none: ExportStatus =
        serde_json::from_value(json!({"export": null, "mail": false})).unwrap();
    assert!(none.export.is_none() && !none.building());

    let password = DeleteConfirmation::Password("geheim 1".into());
    assert_eq!(
        serde_json::to_value(&password).unwrap(),
        json!({"password": "geheim 1"})
    );
    assert!(!format!("{password:?}").contains("geheim"));
    assert_eq!(
        serde_json::to_value(DeleteConfirmation::Email("lena@example.test".into())).unwrap(),
        json!({"email": "lena@example.test"})
    );
    let owned: OwnedServer = serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-000000000007", "name": "Wohnzimmer", "member_count": 5}))
    .unwrap();
    assert_eq!(owned.member_count, Some(5));

    // A ban outlives the account that issued it.
    let bans: Vec<Ban> = serde_json::from_value(json!([{
        "user_id": "00000000-0000-0000-0000-000000000003", "name": "Kai", "avatar_url": null,
        "banned_by": null, "banned_at": "2026-11-10T10:00:00Z"}]))
    .unwrap();
    assert_eq!(bans[0].banned_by, None);
}
