//! For kittest tests of whole screens: a stand-in Gelabber API on a local
//! port (enough of it for the app to start signed in or out) with its
//! gateway (`/ws`: subscriptions answered, frames pushed by the test), and a
//! harness around the [`Launcher`] that steps until a condition holds.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::setup::Launcher;

/// One answer of the stand-in API.
pub struct Reply {
    pub status: u16,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
}

impl Reply {
    pub fn json(body: Value) -> Self {
        Self::status(200, body)
    }

    pub fn status(status: u16, body: Value) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    /// The API's error envelope.
    pub fn error(status: u16, code: &str) -> Self {
        Self::status(status, json!({"error": code, "message": code}))
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// Answers `method path body` before the defaults do; `None` falls through.
pub type Route = dyn Fn(&str, &str, &str) -> Option<Reply> + Send + Sync;

pub const LENA: &str = "00000000-0000-4000-8000-000000000037";

pub fn lena() -> Value {
    json!({
        "id": LENA, "email": "lena@example.test", "name": "Lena", "avatar_url": null,
        "created_at": "2026-01-01T00:00:00Z", "bio": null, "pronouns": null,
        "status_text": null, "accent_color": null, "banner_color": null,
        "banner_url": null, "avatar_decoration": null, "locale": "de"
    })
}

/// The stand-in API. Signed in as Lena (no servers, no DMs) or signed out.
pub struct FakeApi {
    pub origin: String,
    /// `METHOD /path` of every request, in order.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// Its `/ws`.
    pub gateway: Arc<Gateway>,
}

/// The stand-in gateway: answers each subscription with an `ok` at the
/// topic's last pushed sequence number of [`EPOCH`], keeps every frame the
/// app sends, and sends the app the frames a test pushes, on every open
/// socket.
#[derive(Default)]
pub struct Gateway {
    received: Mutex<Vec<Value>>,
    sockets: Mutex<Vec<std::sync::mpsc::Sender<Value>>>,
    /// The last `n` pushed per topic (`s`, `c`).
    sequence: Mutex<std::collections::HashMap<(String, Option<String>), u64>>,
}

/// The `ep` of every topic of the stand-in gateway.
pub const EPOCH: &str = "00000000-0000-4000-8000-0000000000e0";

impl Gateway {
    /// Sends `frame` to the app on every open socket.
    pub fn push(&self, frame: Value) {
        if let (Some(s), Some(n)) = (frame["s"].as_str(), frame["n"].as_u64()) {
            let topic = (s.to_owned(), frame["c"].as_str().map(str::to_owned));
            self.sequence.lock().insert(topic, n);
        }
        self.sockets
            .lock()
            .retain(|socket| socket.send(frame.clone()).is_ok());
    }

    /// Whether the app subscribed to server `s` (and channel `c`).
    pub fn subscribed(&self, s: &str, c: Option<&str>) -> bool {
        self.received
            .lock()
            .iter()
            .any(|f| f["op"] == "s" && f["s"] == s && f["c"].as_str() == c)
    }

    fn serve(&self, stream: TcpStream, key: &str) {
        use tungstenite::protocol::Role;
        let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
        let mut head = stream.try_clone().expect("clone");
        let switching = format!(
            "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\n\
             sec-websocket-accept: {accept}\r\n\r\n"
        );
        if head.write_all(switching.as_bytes()).is_err() {
            return;
        }
        // Reads wait briefly, so frames pushed meanwhile go out.
        let _ = stream.set_read_timeout(Some(Duration::from_millis(10)));
        let mut socket = tungstenite::WebSocket::from_raw_socket(stream, Role::Server, None);
        let (tx, rx) = std::sync::mpsc::channel();
        self.sockets.lock().push(tx);
        loop {
            while let Ok(frame) = rx.try_recv() {
                if socket.send(frame.to_string().into()).is_err() {
                    return;
                }
            }
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    if frame["op"] == "s" {
                        let topic = (
                            frame["s"].as_str().unwrap_or_default().to_owned(),
                            frame["c"].as_str().map(str::to_owned),
                        );
                        let n = self.sequence.lock().get(&topic).copied().unwrap_or(0);
                        let mut ok = json!({"op": "ok", "s": frame["s"], "n": n, "ep": EPOCH});
                        if !frame["c"].is_null() {
                            ok["c"] = frame["c"].clone();
                        }
                        if socket.send(ok.to_string().into()).is_err() {
                            return;
                        }
                    }
                    self.received.lock().push(frame);
                }
                Ok(tungstenite::Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
        }
    }
}

impl FakeApi {
    pub fn start(
        signed_in: bool,
        route: impl Fn(&str, &str, &str) -> Option<Reply> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let gateway: Arc<Gateway> = Arc::default();
        let state = Arc::new(Mutex::new(signed_in));
        let route: Arc<Route> = Arc::new(route);
        let (log, flag, ws) = (seen.clone(), state.clone(), gateway.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, flag, route, ws) = (log.clone(), flag.clone(), route.clone(), ws.clone());
                std::thread::spawn(move || serve(stream, &log, &flag, &*route, &ws));
            }
        });
        Self {
            origin,
            seen,
            gateway,
        }
    }

    pub fn saw(&self, request: &str) -> bool {
        self.seen.lock().iter().any(|r| r == request)
    }
}

fn serve(
    stream: TcpStream,
    log: &Mutex<Vec<String>>,
    signed_in: &Mutex<bool>,
    route: &Route,
    gateway: &Gateway,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut out = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (
            parts.next().unwrap_or("").to_owned(),
            parts.next().unwrap_or("").to_owned(),
        );
        let mut length = 0;
        let mut websocket_key = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            let header = header.trim();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                } else if name.eq_ignore_ascii_case("sec-websocket-key") {
                    websocket_key = Some(value.trim().to_owned());
                }
            }
        }
        if let Some(key) = websocket_key
            && target.split('?').next() == Some("/ws")
        {
            log.lock().push(format!("{method} /ws"));
            return gateway.serve(out, &key);
        }
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let body = String::from_utf8_lossy(&body).into_owned();
        let path = target.split('?').next().unwrap_or("").to_owned();
        let path = path.strip_prefix("/api").unwrap_or(&path).to_owned();
        log.lock().push(format!("{method} {path}"));
        let reply =
            route(&method, &path, &body).unwrap_or_else(|| default(&method, &path, signed_in));
        let text = reply.body.to_string();
        let mut head = format!(
            "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n",
            reply.status,
            text.len()
        );
        for (name, value) in &reply.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        if out.write_all(head.as_bytes()).is_err() || out.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}

fn default(method: &str, path: &str, signed_in: &Mutex<bool>) -> Reply {
    let user = if *signed_in.lock() {
        lena()
    } else {
        Value::Null
    };
    match (method, path) {
        ("GET", "/auth/session") => {
            Reply::json(json!({"user": user, "csrf_token": "csrf", "operator": false}))
        }
        ("GET", "/auth/mail") => Reply::json(json!({"enabled": true})),
        ("POST", "/auth/logout") => {
            *signed_in.lock() = false;
            Reply::json(json!({"csrf_token": "csrf"}))
        }
        ("GET", "/me") if !user.is_null() => Reply::json(user),
        ("GET", "/servers" | "/dms" | "/messages/unread" | "/notification-settings") => {
            Reply::json(json!([]))
        }
        ("GET", "/notification-privacy") => Reply::json(json!({"hide_content": false})),
        ("GET", "/me/themes") => {
            Reply::json(json!({"version": 1, "revision": 0, "active": "dark", "customThemes": []}))
        }
        _ => Reply::error(404, "not_found"),
    }
}

/// `/auth/session` and `/me` of Lena with English as her account's
/// language, for tests of the English interface.
pub fn english_account(method: &str, path: &str) -> Option<Reply> {
    let mut user = lena();
    user["locale"] = "en".into();
    match (method, path) {
        ("GET", "/auth/session") => Some(Reply::json(
            json!({"user": user, "csrf_token": "csrf", "operator": false}),
        )),
        ("GET", "/me") => Some(Reply::json(user)),
        _ => None,
    }
}

pub const HOME: &str = "00000000-0000-4000-8000-0000000000a0";
pub const ALLGEMEIN: &str = "00000000-0000-4000-8000-0000000000c1";
pub const SPIELE: &str = "00000000-0000-4000-8000-0000000000c3";
pub const MODERATION: &str = "00000000-0000-4000-8000-0000000000d1";
pub const STAMMGAESTE: &str = "00000000-0000-4000-8000-0000000000d2";

/// The custom roles of the parity seed: Moderation (blue) and Stammgäste
/// (green), by position.
pub fn roles() -> Value {
    json!([
        {"id": MODERATION, "server_id": HOME, "name": "Moderation", "color": "blue",
         "permissions": ["moderate_members", "manage_messages", "mention_everyone"],
         "position": 0, "created_at": "2026-01-01T00:00:00Z"},
        {"id": STAMMGAESTE, "server_id": HOME, "name": "Stammgäste", "color": "green",
         "permissions": [], "position": 1, "created_at": "2026-01-01T00:00:00Z"}
    ])
}

/// Wohnzimmer as the parity fixture has it: Lena owns it; Textkanäle
/// (allgemein), Sprachkanäle (Wohnzimmer), Plaudern (spiele, musik).
/// spiele is limited to Moderation and Stammgäste.
pub fn workspace(method: &str, path: &str) -> Option<Reply> {
    let server = json!({
        "id": HOME, "name": "Wohnzimmer", "owner_id": LENA, "icon_url": null,
        "created_at": "2026-01-01T00:00:00Z", "role": "owner",
        "permissions": ["manage_server", "manage_channels", "join_voice"],
        "blocked": false, "member_permissions": []
    });
    let category = |n: u8, name: &str| {
        json!({"id": format!("00000000-0000-4000-8000-0000000000b{n}"), "server_id": HOME,
               "name": name, "created_at": "2026-01-01T00:00:00Z"})
    };
    let channel = |n: u8, category: u8, name: &str, kind: &str| {
        let id = format!("00000000-0000-4000-8000-0000000000c{n}");
        let role_ids = if id == SPIELE {
            json!([MODERATION, STAMMGAESTE])
        } else {
            json!([])
        };
        json!({"id": id, "server_id": HOME,
               "category_id": format!("00000000-0000-4000-8000-0000000000b{category}"),
               "name": name, "kind": kind, "created_at": "2026-01-01T00:00:00Z",
               "limited": id == SPIELE, "role_ids": role_ids})
    };
    match (method, path) {
        ("GET", "/servers") => Some(Reply::json(json!([server]))),
        ("GET", p) if p == format!("/servers/{HOME}") => {
            let mut detail = server.clone();
            detail["categories"] = json!([
                category(1, "Textkanäle"),
                category(2, "Sprachkanäle"),
                category(3, "Plaudern")
            ]);
            detail["channels"] = json!([
                channel(1, 1, "allgemein", "text"),
                channel(2, 2, "Wohnzimmer", "voice"),
                channel(3, 3, "spiele", "text"),
                channel(4, 3, "musik", "text")
            ]);
            detail["members"] = json!([{
                "user_id": LENA, "name": "Lena", "avatar_url": null,
                "joined_at": "2026-01-01T00:00:00Z", "status_text": null,
                "accent_color": null, "avatar_decoration": null, "role": "owner",
                "role_ids": []
            }]);
            detail["roles"] = roles();
            Some(Reply::json(detail))
        }
        ("GET", p) if p.starts_with("/channels/") && p.ends_with("/messages") => {
            Some(Reply::json(json!({"messages": [], "has_more": false})))
        }
        _ => None,
    }
}

/// The label of the node that has focus, if any.
pub fn focused_label(harness: &Harness<'_, Launcher>) -> Option<String> {
    use egui_kittest::kittest::NodeT;
    harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.is_focused()))
        .next()
        .map(|n| n.accesskit_node().label().unwrap_or_default())
}

/// The app on `api` at `route`, headless (accessibility tree only).
pub fn launcher(api: &FakeApi, route: &str) -> Harness<'static, Launcher> {
    let client = gelabber_client::Client::new(&api.origin).expect("client");
    let route = route.to_owned();
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_max_steps(10_000)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| Launcher::with_app(cc, client.clone(), Some(route.clone())))
}

/// Steps (in real time: requests go to another thread) until `done`.
pub fn wait_for(
    harness: &mut Harness<'_, Launcher>,
    what: &str,
    done: impl Fn(&Harness<'_, Launcher>) -> bool,
) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for {what}"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Steps until a node labelled `label` is in the tree and lies where it
/// lay a frame before. A click goes to the place a node had when the test
/// asked for it, and a node that only just showed may still move: the auth
/// pages centre their column by the height of the pass before, so a frame
/// that got an answer between its two passes is drawn once with the column
/// off by half of what came in (GEL-78).
pub fn wait_for_label(harness: &mut Harness<'_, Launcher>, label: &str) {
    use egui_kittest::kittest::NodeT;
    let last = std::cell::Cell::new(None);
    wait_for(harness, label, |h| {
        let place = h
            .query_by_label(label)
            .map(|node| node.accesskit_node().bounding_box());
        place.is_some() && last.replace(place) == place
    });
}

/// Steps until a node whose label contains `text` is in the tree.
pub fn wait_for_text(harness: &mut Harness<'_, Launcher>, text: &str) {
    wait_for(harness, text, |h| h.query_by_label_contains(text).is_some());
}

/// The user panel's avatar button, which opens the account popover: the
/// lowest button named after `name` (an unread feedback reply adds to its
/// name).
pub fn account_button<'a>(
    harness: &'a Harness<'_, Launcher>,
    name: &str,
) -> Option<egui_kittest::Node<'a>> {
    use egui_kittest::kittest::NodeT;
    harness
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l == name || l.starts_with(&format!("{name}, ")))
        })
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
}

/// Opens the account popover from `name`'s avatar, once it is drawn.
pub fn open_account(harness: &mut Harness<'_, Launcher>, name: &str) {
    wait_for(harness, "the user panel", |h| {
        account_button(h, name).is_some()
    });
    account_button(harness, name).expect("the avatar").click();
    wait_for_label(harness, "Abmelden");
}
