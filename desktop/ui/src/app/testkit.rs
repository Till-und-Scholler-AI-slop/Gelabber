//! For kittest tests of whole screens: a stand-in Gelabber API on a local
//! port (enough of it for the app to start signed in or out), and a
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
        "banner_url": null, "avatar_decoration": null
    })
}

/// The stand-in API. Signed in as Lena (no servers, no DMs) or signed out.
pub struct FakeApi {
    pub origin: String,
    /// `METHOD /path` of every request, in order.
    pub seen: Arc<Mutex<Vec<String>>>,
}

impl FakeApi {
    pub fn start(
        signed_in: bool,
        route: impl Fn(&str, &str, &str) -> Option<Reply> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let state = Arc::new(Mutex::new(signed_in));
        let route: Arc<Route> = Arc::new(route);
        let (log, flag) = (seen.clone(), state.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, flag, route) = (log.clone(), flag.clone(), route.clone());
                std::thread::spawn(move || serve(stream, &log, &flag, &*route));
            }
        });
        Self { origin, seen }
    }

    pub fn saw(&self, request: &str) -> bool {
        self.seen.lock().iter().any(|r| r == request)
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<String>>, signed_in: &Mutex<bool>, route: &Route) {
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
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            let header = header.trim();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap_or(0);
            }
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
        ("GET", "/auth/session") => Reply::json(json!({"user": user, "csrf_token": "csrf"})),
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

pub const HOME: &str = "00000000-0000-4000-8000-0000000000a0";
pub const ALLGEMEIN: &str = "00000000-0000-4000-8000-0000000000c1";

/// Wohnzimmer as the parity fixture has it: Lena owns it; Textkanäle
/// (allgemein), Sprachkanäle (Wohnzimmer), Plaudern (spiele, musik).
pub fn workspace(method: &str, path: &str) -> Option<Reply> {
    let server = json!({
        "id": HOME, "name": "Wohnzimmer", "owner_id": LENA, "icon_url": null,
        "created_at": "2026-01-01T00:00:00Z", "role": "owner",
        "permissions": ["manage_server", "manage_channels", "join_voice"],
        "member_permissions": []
    });
    let category = |n: u8, name: &str| {
        json!({"id": format!("00000000-0000-4000-8000-0000000000b{n}"), "server_id": HOME,
               "name": name, "created_at": "2026-01-01T00:00:00Z"})
    };
    let channel = |n: u8, category: u8, name: &str, kind: &str| {
        json!({"id": format!("00000000-0000-4000-8000-0000000000c{n}"), "server_id": HOME,
               "category_id": format!("00000000-0000-4000-8000-0000000000b{category}"),
               "name": name, "kind": kind, "created_at": "2026-01-01T00:00:00Z"})
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
                "accent_color": null, "avatar_decoration": null, "role": "owner"
            }]);
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

/// Steps until a node labelled `label` is in the tree.
pub fn wait_for_label(harness: &mut Harness<'_, Launcher>, label: &str) {
    wait_for(harness, label, |h| h.query_by_label(label).is_some());
}

/// Steps until a node whose label contains `text` is in the tree.
pub fn wait_for_text(harness: &mut Harness<'_, Launcher>, text: &str) {
    wait_for(harness, text, |h| h.query_by_label_contains(text).is_some());
}
