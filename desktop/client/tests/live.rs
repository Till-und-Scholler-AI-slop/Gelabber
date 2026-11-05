//! Against the seeded local stack (web/scripts/parity/seed.mjs), ignored by
//! default. Run with
//! `GELABBER_TEST_SERVER=http://127.0.0.1:5180 cargo test -p gelabber-client --test live -- --ignored`.
//! The seeded screens stay as they are: what these tests write happens in
//! a scratch server that is deleted at the end.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gelabber_client::*;
use parking_lot::Mutex;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Fixture {
    password: String,
    users: HashMap<String, FixtureUser>,
    servers: FixtureServers,
    dm: Uuid,
}

#[derive(Deserialize)]
struct FixtureUser {
    id: Uuid,
    email: String,
}

#[derive(Deserialize)]
struct FixtureServers {
    home: FixtureHome,
    evening: FixtureServer,
}

#[derive(Deserialize)]
struct FixtureHome {
    id: Uuid,
    channels: HashMap<String, Uuid>,
}

#[derive(Deserialize)]
struct FixtureServer {
    id: Uuid,
}

fn server() -> String {
    std::env::var("GELABBER_TEST_SERVER").unwrap_or_else(|_| "http://127.0.0.1:5180".into())
}

fn fixture() -> Fixture {
    let path = std::env::var("GELABBER_FIXTURE").unwrap_or_else(|_| {
        format!(
            "{}/../../web/scripts/parity/fixture.json",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// Polls the state until `done` holds.
fn wait(client: &Client, what: &str, done: impl Fn(&State) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if client.with_state(&done) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let ops = client.with_state(|s| format!("{:?}", s.ops));
    panic!("timed out waiting for {what}; ops: {ops}");
}

/// A signed-in client with its workspace loaded.
fn signed_in(fixture: &Fixture, who: &str) -> Client {
    let client = Client::new(&server()).unwrap();
    client.login(&fixture.users[who].email, &fixture.password);
    wait(&client, "bootstrap", |s| s.bootstrapped);
    wait(&client, "gateway open", |s| {
        s.gateway == GatewayStatus::Open
    });
    client
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER)"]
fn seeded_workspace_loads_like_the_web() {
    let fx = fixture();
    let lena = signed_in(&fx, "lena");
    let home = fx.servers.home.id;
    let allgemein = fx.servers.home.channels["allgemein"];
    lena.with_state(|s| {
        assert_eq!(s.me().unwrap().id, fx.users["lena"].id);
        let names: Vec<&str> = s.servers.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(&names[..2], ["Wohnzimmer", "Spieleabend"]);
        assert_eq!(s.servers[1].id, fx.servers.evening.id);
        assert_eq!(s.dms.len(), 1);
        assert_eq!(s.dms[0].peer.id, fx.users["jonas"].id);
        assert!(s.themes.is_some() && s.notification_privacy.is_some());
        // musik has one unread message from Jonas: a dot, no number.
        let musik = fx.servers.home.channels["musik"];
        assert!(s.read[&musik].unread_count >= 1);
        assert_eq!(
            s.channel_badge(musik, chrono::Utc::now()),
            Badge {
                count: 0,
                dot: true
            }
        );
    });

    lena.open_server_channel(home, allgemein);
    wait(&lena, "allgemein history", |s| {
        s.messages(allgemein).is_some_and(|h| h.loaded)
    });
    lena.with_state(|s| {
        let detail = s.detail(home).expect("server detail");
        assert_eq!(detail.members.len(), 5);
        let history = s.messages(allgemein).unwrap();
        let texts: Vec<&str> = history
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(texts.len(), 10);
        assert_eq!(texts[0], "Hallo zusammen! 👋");
        assert_eq!(texts[1], "Wer ist heute Abend dabei?");
        assert_eq!(texts[9], "Bis später! 🎲");
        assert!(!history.has_more);
        let plan = &history.messages[2];
        let chips = &history.messages[5];
        assert_eq!(chips.author.name, "Jonas");
        let reply = chips.reply.as_ref().expect("reply");
        assert_eq!(reply.id, plan.id);
        assert_eq!(reply.author.as_ref().unwrap().name, "Mira");
        let code = &history.messages[7];
        assert_eq!(code.author.name, "Tom");
        let thumbs = code.reactions.iter().find(|r| r.emoji == "👍").unwrap();
        let mut expected = vec![fx.users["jonas"].id, fx.users["mira"].id];
        expected.sort();
        assert_eq!(thumbs.user_ids, expected);
        assert!(code.reacted("🎉", fx.users["lena"].id));
        // The view's topics: server, every text channel, the DM.
        let topics = s.desired_topics();
        assert!(topics.contains(&Topic::server(home)));
        assert!(topics.contains(&Topic::channel(home, allgemein)));
        assert!(topics.contains(&Topic::dm(fx.dm)));
    });
    // The subscribe answers with a presence snapshot of the server: me.
    wait(&lena, "presence snapshot", |s| {
        s.presence_of(home, fx.users["lena"].id) == PresenceStatus::Online
    });

    lena.open_dm(fx.dm);
    wait(&lena, "DM history", |s| {
        s.messages(fx.dm).is_some_and(|h| h.loaded)
    });
    lena.with_state(|s| {
        let texts: Vec<&str> = s
            .messages(fx.dm)
            .unwrap()
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(
            texts,
            [
                "Hey Lena, hast du die Fotos vom Wochenende noch?",
                "Ja, ich schicke sie dir heute Abend."
            ]
        );
        assert_eq!(s.view.server, None);
        assert_eq!(s.view.channel, Some(fx.dm));
    });
    while lena.busy() {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Deletes the scratch server even when an assertion fails.
struct Scratch {
    client: Client,
    server: Uuid,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let http = self.client.http().clone();
        let server = self.server;
        let _ = self
            .client
            .block_on(async move { http.delete_server(server).await });
    }
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER)"]
fn messages_typing_and_presence_arrive_over_the_gateway() {
    let fx = fixture();
    let lena = signed_in(&fx, "lena");
    let jonas = signed_in(&fx, "jonas");
    let lena_id = fx.users["lena"].id;
    let jonas_id = fx.users["jonas"].id;

    // A scratch server with Jonas in it.
    let http = lena.http().clone();
    let detail = lena
        .block_on(async move { http.create_server("Native-Kern-Test").await })
        .unwrap();
    let scratch = Scratch {
        client: lena.clone(),
        server: detail.server.id,
    };
    let server = scratch.server;
    let text = detail
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Text)
        .unwrap()
        .id;
    let voice = detail
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .unwrap()
        .id;
    let http = lena.http().clone();
    let invite = lena
        .block_on(async move { http.create_invite(server, None, Some(1)).await })
        .unwrap();
    jonas.join_invite(&invite.code);
    wait(&jonas, "joined", |s| s.server(server).is_some());

    // A raw gateway on Lena's cookies records what arrives.
    let seen: Arc<Mutex<Vec<GatewayEvent>>> = Arc::default();
    let sink = seen.clone();
    let raw_runtime = tokio::runtime::Runtime::new().unwrap();
    let raw = Gateway::start(
        raw_runtime.handle(),
        lena.http().clone(),
        GatewayConfig::default(),
        Arc::new(move |event| sink.lock().push(event)),
    );
    raw.set_topics(vec![Topic::server(server), Topic::channel(server, text)]);

    lena.refresh_servers();
    wait(&lena, "scratch server listed", |s| {
        s.server(server).is_some()
    });
    lena.open_server_channel(server, text);
    wait(&lena, "scratch history", |s| {
        s.messages(text).is_some_and(|h| h.loaded) && s.detail(server).is_some()
    });
    jonas.open_server_channel(server, text);
    wait(&jonas, "jonas history", |s| {
        s.messages(text).is_some_and(|h| h.loaded)
    });

    // Presence: Jonas' subscribe announces him in the server.
    wait(&lena, "jonas online", |s| {
        s.presence_of(server, jonas_id) == PresenceStatus::Online
    });
    lena.with_state(|s| {
        let groups = s.member_groups(server);
        assert_eq!(groups[0].0, PresenceStatus::Online);
        assert_eq!(groups[0].1.len(), 2);
    });

    // Typing, with the German bar text.
    jonas.typing_input(text, "H");
    wait(&lena, "jonas typing", |s| {
        s.typers(text, Instant::now()) == vec![jonas_id]
    });
    lena.with_state(|s| assert_eq!(s.typing_text(text, Instant::now()), "Jonas tippt…"));

    // Jonas sends; Lena gets it over the socket, Jonas' pending row is
    // swapped for the server's message.
    jonas.send_message(text, "Hallo aus dem nativen Client", None);
    wait(&lena, "message arrives", |s| {
        s.messages(text).is_some_and(|h| {
            h.messages
                .iter()
                .any(|m| m.content == "Hallo aus dem nativen Client")
        })
    });
    wait(&lena, "typing stops", |s| {
        s.typers(text, Instant::now()).is_empty()
    });
    wait(&jonas, "pending confirmed", |s| {
        s.messages(text)
            .is_some_and(|h| h.pending.is_empty() && h.messages.len() == 1)
    });
    let message = jonas.with_state(|s| s.messages(text).unwrap().messages[0].clone());
    assert_eq!(message.author.id, jonas_id);
    assert!(message.nonce.is_some());

    // A reply from Lena, a reaction, an edit and a delete, all live.
    lena.send_message(text, "Antwort", Some(message.id));
    wait(&jonas, "reply arrives", |s| {
        s.messages(text).is_some_and(|h| {
            h.messages
                .iter()
                .any(|m| m.reply.as_ref().is_some_and(|r| r.id == message.id))
        })
    });
    lena.toggle_reaction(message.id, "👍");
    wait(&jonas, "reaction arrives", |s| {
        s.find_message(message.id)
            .is_some_and(|m| m.reacted("👍", lena_id))
    });
    jonas.edit_message(message.id, "Hallo, geändert");
    wait(&lena, "edit arrives", |s| {
        s.find_message(message.id)
            .is_some_and(|m| m.content == "Hallo, geändert" && m.edited_at.is_some())
    });
    wait(&lena, "reply excerpt follows the edit", |s| {
        s.messages(text).is_some_and(|h| {
            h.messages.iter().any(|m| {
                m.reply
                    .as_ref()
                    .is_some_and(|r| r.excerpt.as_deref() == Some("Hallo, geändert"))
            })
        })
    });
    jonas.delete_message(message.id);
    wait(&lena, "delete arrives", |s| {
        s.find_message(message.id).is_none()
    });
    lena.with_state(|s| {
        let reply = s.messages(text).unwrap().messages[0].reply.clone().unwrap();
        assert!(reply.deleted);
    });

    // Voice presence: Jonas takes a seat muted; Lena sees it.
    jonas.set_voice_muted(true);
    jonas.voice_join(server, voice);
    wait(&lena, "jonas in voice", |s| {
        s.voice_occupants(server, voice)
            .iter()
            .any(|(u, o)| *u == jonas_id && o.muted)
    });
    jonas.voice_leave();
    wait(&lena, "jonas left voice", |s| {
        s.voice_occupants(server, voice).is_empty()
    });

    // The raw socket saw the sequenced events once each, in order.
    let start = Instant::now();
    loop {
        let kinds: Vec<EventKind> = seen
            .lock()
            .iter()
            .filter_map(|e| match e {
                GatewayEvent::Event(ev) if ev.c == Some(text) => Some(ev.t),
                _ => None,
            })
            .collect();
        // create, create (reply), edit (reaction), edit, edit (reply quote), delete
        if kinds.contains(&EventKind::D) || start.elapsed() > Duration::from_secs(5) {
            assert_eq!(kinds.first(), Some(&EventKind::C));
            assert_eq!(kinds.last(), Some(&EventKind::D));
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let n: Vec<u64> = seen
        .lock()
        .iter()
        .filter_map(|e| match e {
            GatewayEvent::Event(ev) if ev.c == Some(text) => Some(ev.n),
            _ => None,
        })
        .collect();
    assert!(n.windows(2).all(|w| w[0] < w[1]), "sequence {n:?}");
    assert!(
        seen.lock()
            .iter()
            .any(|e| matches!(e, GatewayEvent::Status(GatewayStatus::Open)))
    );
    raw.stop();

    // Kicking Jonas reaches him as a notice and drops the server.
    lena.kick(server, jonas_id);
    let start = Instant::now();
    let mut notices = Vec::new();
    while notices.is_empty() && start.elapsed() < Duration::from_secs(10) {
        notices = jonas.take_notices();
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(matches!(
        notices.first(),
        Some(Notice::Kicked { text, .. }) if text == "Du wurdest vom Server entfernt."
    ));
    wait(&jonas, "server dropped", |s| s.server(server).is_none());

    drop(scratch);
    jonas.logout();
    wait(&jonas, "signed out", |s| s.session == Session::Anonymous);
}

/// A scratch server owned by `client` with its text channel.
fn scratch_server(client: &Client, name: &str) -> (Scratch, Uuid) {
    let http = client.http().clone();
    let name = name.to_owned();
    let detail = client
        .block_on(async move { http.create_server(&name).await })
        .unwrap();
    let text = detail
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Text)
        .unwrap()
        .id;
    (
        Scratch {
            client: client.clone(),
            server: detail.server.id,
        },
        text,
    )
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER)"]
fn attachments_upload_bind_and_download() {
    let fx = fixture();
    let lena = signed_in(&fx, "lena");
    let (scratch, text) = scratch_server(&lena, "Native-Anhang-Test");
    lena.refresh_servers();
    lena.open_server_channel(scratch.server, text);
    wait(&lena, "history", |s| {
        s.messages(text).is_some_and(|h| h.loaded)
    });
    let file = OutgoingFile {
        filename: "notiz.txt".into(),
        content_type: "text/plain".into(),
        data: Arc::new(b"Hallo Anhang".to_vec()),
    };
    lena.send_files(text, "Mit Datei", vec![file], None);
    wait(&lena, "message with attachment", |s| {
        s.messages(text).is_some_and(|h| {
            h.pending.is_empty() && h.messages.iter().any(|m| m.attachments.len() == 1)
        })
    });
    let attachment =
        lena.with_state(|s| s.messages(text).unwrap().messages[0].attachments[0].clone());
    assert_eq!(attachment.filename, "notiz.txt");
    assert_eq!(attachment.size, 12);
    // Follows the redirect to the object store without our cookies.
    let http = lena.http().clone();
    let url = http.attachment_url(attachment.id, false);
    let bytes = lena
        .block_on(async move { http.bytes(&url).await })
        .unwrap();
    assert_eq!(bytes, b"Hallo Anhang");
    while lena.busy() {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER)"]
fn reconnect_resumes_without_losing_or_repeating_events() {
    let fx = fixture();
    let lena = signed_in(&fx, "lena");
    let (scratch, text) = scratch_server(&lena, "Native-Resume-Test");
    let server = scratch.server;
    // A watchdog shorter than the server's 15 s heartbeat drops the socket
    // after 0.6 s of silence, so the sends below straddle reconnects.
    let seen: Arc<Mutex<Vec<GatewayEvent>>> = Arc::default();
    let sink = seen.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let config = GatewayConfig {
        dead: Duration::from_millis(600),
        ..GatewayConfig::default()
    };
    let raw = Gateway::start(
        runtime.handle(),
        lena.http().clone(),
        config,
        Arc::new(move |event| sink.lock().push(event)),
    );
    raw.set_topics(vec![Topic::channel(server, text)]);
    let opens = || {
        seen.lock()
            .iter()
            .filter(|e| matches!(e, GatewayEvent::Ready))
            .count()
    };
    let start = Instant::now();
    while opens() < 1 && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let http = lena.http().clone();
    for i in 0..4 {
        let http = http.clone();
        let body = NewMessage {
            content: format!("Nachricht {i}"),
            attachment_ids: Some(vec![]),
            ..NewMessage::default()
        };
        lena.block_on(async move { http.create_message(text, &body).await })
            .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
    }
    let start = Instant::now();
    let creates = || -> Vec<u64> {
        seen.lock()
            .iter()
            .filter_map(|e| match e {
                GatewayEvent::Event(ev) if ev.t == EventKind::C => Some(ev.n),
                _ => None,
            })
            .collect()
    };
    while creates().len() < 4 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(opens() >= 2, "the socket never reconnected");
    assert_eq!(creates(), vec![1, 2, 3, 4]);
    assert!(
        raw.cursor(&Topic::channel(server, text))
            .is_some_and(|c| c.n == 4)
    );
    raw.stop();
    drop(scratch);
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER)"]
fn switching_accounts_keeps_nothing_of_the_first() {
    let fx = fixture();
    let client = signed_in(&fx, "lena");
    let home = fx.servers.home.id;
    let allgemein = fx.servers.home.channels["allgemein"];
    client.open_server_channel(home, allgemein);
    // Sign out while the history and its read mark are still in flight.
    client.logout();
    wait(&client, "signed out", |s| {
        s.session == Session::Anonymous && s.op("logout") == Some(&OpState::Done)
    });
    client.login(&fx.users["jonas"].email, &fx.password);
    wait(&client, "jonas bootstrapped", |s| {
        s.me().is_some_and(|m| m.id == fx.users["jonas"].id) && s.bootstrapped
    });
    wait(&client, "gateway open", |s| {
        s.gateway == GatewayStatus::Open
    });
    // Lena's stopped socket must not report its end over Jonas' one.
    std::thread::sleep(Duration::from_millis(500));
    let http = client.http().clone();
    let servers = client
        .block_on(async move { http.servers().await })
        .unwrap();
    client.with_state(|s| {
        assert_eq!(s.gateway, GatewayStatus::Open);
        assert_eq!(s.view, View::default());
        assert!(s.messages(allgemein).is_none());
        let ids: Vec<Uuid> = s.servers.iter().map(|v| v.id).collect();
        assert_eq!(ids, servers.iter().map(|v| v.id).collect::<Vec<_>>());
    });
}
