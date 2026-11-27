//! For kittest tests of whole screens: a stand-in Gelabber API on a local
//! port (enough of it for the app to start signed in or out), and a
//! harness around the [`Launcher`] that steps until a condition holds.
//!
//! Requests go to the stand-in API on other threads, in real time, and its
//! answers change the layout. So a click waits until the app is idle and
//! checks where it lands ([`click`], [`click_node`]). With
//! `GELABBER_TEST_API_DELAY_MS` set, every answer comes that much later: a
//! test that clicks too early then fails every time instead of once in a
//! while. Run the whole-app tests so:
//!
//! `GELABBER_TEST_API_DELAY_MS=400 cargo test --manifest-path desktop/Cargo.toml -p gelabber-ui --lib`
//!
//! Idle ([`wait_until_idle`]) knows the stand-in's requests, the client's
//! requests and its timers for later work (`Client::scheduled`: retries,
//! the unread debounce), and the images loading. It does not know the
//! client's 400 ms ticker (typing, idle) or a page's own delay before it
//! asks (`request_repaint_after`: the chat search, the GIF picker, the
//! directory's search). A test of those waits for what it expects to see.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
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
    /// Requests read and not yet answered.
    answering: Arc<AtomicUsize>,
}

/// How long every answer of the stand-in API waits: `GELABBER_TEST_API_DELAY_MS`.
fn answer_delay() -> Option<Duration> {
    let ms: u64 = std::env::var("GELABBER_TEST_API_DELAY_MS")
        .ok()?
        .parse()
        .ok()?;
    (ms > 0).then(|| Duration::from_millis(ms))
}

/// The stand-in API of the app this test thread launched last
/// ([`launcher`], [`FakeApi::watch`]): its requests in flight and its log.
struct Watched {
    answering: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
}

thread_local! {
    static WATCHED: RefCell<Option<Watched>> = const { RefCell::new(None) };
}

fn answering() -> usize {
    WATCHED.with(|w| {
        w.borrow()
            .as_ref()
            .map_or(0, |w| w.answering.load(Ordering::SeqCst))
    })
}

/// The last requests the stand-in saw, for a failure's message.
fn last_seen() -> String {
    WATCHED.with(|w| {
        w.borrow().as_ref().map_or_else(String::new, |w| {
            let seen = w.seen.lock();
            let from = seen.len().saturating_sub(12);
            format!("; the stand-in saw last: {:?}", &seen[from..])
        })
    })
}

/// Work on its way that the next frames depend on: requests, the client's
/// timers and images.
fn pending(harness: &mut Harness<'_, Launcher>) -> bool {
    let scheduled = harness
        .state_mut()
        .app()
        .is_some_and(|app| app.client().scheduled() > 0);
    answering() > 0 || scheduled || harness.state().busy()
}

/// Counts a request as in flight until it is answered or the connection ends.
struct Answering<'a>(&'a AtomicUsize);

impl<'a> Answering<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Answering<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
        let state = Arc::new(Mutex::new(signed_in));
        let route: Arc<Route> = Arc::new(route);
        let answering: Arc<AtomicUsize> = Arc::default();
        let (log, flag, count) = (seen.clone(), state.clone(), answering.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, flag, route, count) =
                    (log.clone(), flag.clone(), route.clone(), count.clone());
                std::thread::spawn(move || serve(stream, &log, &flag, &*route, &count));
            }
        });
        Self {
            origin,
            seen,
            answering,
        }
    }

    /// Makes [`wait_until_idle`] on this thread wait for this API's answers;
    /// [`launcher`] does it, a harness built by hand calls it.
    pub fn watch(&self) {
        WATCHED.with(|w| {
            *w.borrow_mut() = Some(Watched {
                answering: self.answering.clone(),
                seen: self.seen.clone(),
            })
        });
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
    answering: &AtomicUsize,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut out = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let _answering = Answering::new(answering);
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
        // The route ran (a test may read what it kept); only the answer is late.
        if let Some(delay) = answer_delay() {
            std::thread::sleep(delay);
        }
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
    harness
        .query_all(By::new().predicate(|n| n.is_focused()))
        .next()
        .map(|n| n.accesskit_node().label().unwrap_or_default())
}

/// The app on `api` at `route`, headless (accessibility tree only).
pub fn launcher(api: &FakeApi, route: &str) -> Harness<'static, Launcher> {
    let client = gelabber_client::Client::new(&api.origin).expect("client");
    launcher_with(api, client, route)
}

/// The app with `client` (on `api`) at `route`.
pub fn launcher_with(
    api: &FakeApi,
    client: gelabber_client::Client,
    route: &str,
) -> Harness<'static, Launcher> {
    api.watch();
    let route = route.to_owned();
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_max_steps(10_000)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| Launcher::with_app(cc, client.clone(), Some(route.clone())))
}

/// How long [`wait_for`] and the helpers on it give the app.
const PATIENCE: Duration = Duration::from_secs(15);
/// Frames in a row the tree must stay the same for the app to count as idle.
const STILL_FRAMES: usize = 3;

/// Steps (in real time: requests go to another thread) until `check`
/// passes, and fails with its last word when it never does.
fn step_until<'h>(
    harness: &mut Harness<'h, Launcher>,
    mut check: impl FnMut(&mut Harness<'h, Launcher>) -> Result<(), String>,
) {
    let start = Instant::now();
    loop {
        let why = match check(harness) {
            Ok(()) => return,
            Err(why) => why,
        };
        assert!(
            start.elapsed() < PATIENCE,
            "timed out waiting for {why}{}",
            last_seen()
        );
        harness.step();
        // Real time only for work on other threads; frames alone need none.
        if pending(harness) {
            std::thread::sleep(Duration::from_millis(10));
        } else {
            std::thread::yield_now();
        }
    }
}

/// Polls `done` until it holds, for work on other threads that no harness
/// steps (a worker, a download, the client's state); fails as [`wait_for`].
pub fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Steps until `done`.
pub fn wait_for(
    harness: &mut Harness<'_, Launcher>,
    what: &str,
    done: impl Fn(&Harness<'_, Launcher>) -> bool,
) {
    wait_until(harness, what, |h| done(h));
}

/// Steps until `done`, which may reach into the app ([`Launcher::app`]).
pub fn wait_until<'h>(
    harness: &mut Harness<'h, Launcher>,
    what: &str,
    mut done: impl FnMut(&mut Harness<'h, Launcher>) -> bool,
) {
    step_until(harness, |h| {
        if done(h) {
            Ok(())
        } else {
            Err(what.to_owned())
        }
    });
}

/// Steps until the app shows `path`.
pub fn wait_for_path(harness: &mut Harness<'_, Launcher>, path: &str) {
    wait_until(harness, path, |h| {
        h.state_mut()
            .app()
            .is_some_and(|app| app.nav().route().path() == path)
    });
}

/// Every node with what it says and where it lies, as one number.
fn fingerprint(harness: &Harness<'_, Launcher>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for node in harness.query_all(By::new()) {
        let node = node.accesskit_node();
        node.locate().0.hash(&mut hasher);
        format!("{:?}", node.data()).hash(&mut hasher);
    }
    hasher.finish()
}

/// Steps until the app has nothing on its way: no request at the stand-in
/// API, none in the client, no image loading, and the accessibility tree
/// the same for a few frames. A click aimed before then can land where a
/// node lay before an answer moved it.
pub fn wait_until_idle(harness: &mut Harness<'_, Launcher>) {
    let mut last = None;
    let mut still = 0;
    step_until(harness, |h| {
        let print = fingerprint(h);
        let answering = answering();
        let busy = pending(h);
        // Frames count as still only once nothing is on its way: an answer
        // that just came in changes the tree on a later frame.
        still = if answering == 0 && !busy && last.replace(print) == Some(print) {
            still + 1
        } else {
            last = Some(print);
            0
        };
        if answering > 0 {
            Err(format!(
                "the app to settle: {answering} requests at the stand-in API"
            ))
        } else if busy {
            Err("the app to settle: the client is busy or has a timer set".into())
        } else if still < STILL_FRAMES {
            Err("the app to settle: the tree keeps changing".into())
        } else {
            Ok(())
        }
    });
}

/// The node with accesskit id `id`.
fn node_by_id<'a>(
    harness: &'a Harness<'_, Launcher>,
    id: egui::accesskit::NodeId,
) -> Option<egui_kittest::Node<'a>> {
    harness
        .query_all(By::new())
        .find(|n| n.accesskit_node().locate().0 == id)
}

/// The node and the nodes it lies in, innermost first.
fn with_ancestors(node: &egui_kittest::Node<'_>) -> Vec<egui::accesskit::NodeId> {
    let mut ids = vec![node.accesskit_node().locate().0];
    let mut up = node.accesskit_node().parent();
    while let Some(parent) = up {
        ids.push(parent.locate().0);
        up = parent.parent();
    }
    ids
}

/// Clicks the node `pick` names, once the app is idle ([`wait_until_idle`]),
/// as [`click_now`] does.
pub fn click_node(
    harness: &mut Harness<'_, Launcher>,
    what: &str,
    pick: impl Fn(&Harness<'_, Launcher>) -> Option<egui::accesskit::NodeId>,
) {
    wait_until_idle(harness);
    click_now(harness, what, pick);
}

/// Clicks the node `pick` names without waiting for the app, once it lies
/// still: for a test whose point is a click while a request is on its way,
/// or before the app's own retry. The pointer goes
/// there first, and egui's own hit test must name that node, one it lies
/// in or one in it (for a disabled node: one around it) before the button
/// goes down; a click that would land elsewhere fails at once, with the
/// node's rectangle.
pub fn click_now(
    harness: &mut Harness<'_, Launcher>,
    what: &str,
    pick: impl Fn(&Harness<'_, Launcher>) -> Option<egui::accesskit::NodeId>,
) {
    // Where it lay a frame before: an answer that just came in may still
    // move it.
    let mut last = None;
    step_until(harness, |h| {
        let Some(place) = pick(h).and_then(|id| node_by_id(h, id)).map(|n| n.rect()) else {
            let labels: Vec<String> = h
                .query_all(By::new())
                .filter_map(|n| n.accesskit_node().label())
                .collect();
            return Err(format!("{what} to click; the labels: {labels:?}"));
        };
        if last.replace(place) == Some(place) {
            Ok(())
        } else {
            Err(format!("{what} to lie still"))
        }
    });
    let id = pick(harness).expect("the picked node");
    let rect = node_by_id(harness, id).expect("the picked node").rect();
    let window = harness.ctx.content_rect();
    assert!(
        window.contains(rect.center()),
        "{what} at {rect:?} lies outside the window {window:?}"
    );
    harness.hover_at(rect.center());
    harness.step();
    let node = node_by_id(harness, id)
        .unwrap_or_else(|| panic!("{what} at {rect:?} went away under the pointer"));
    assert_eq!(node.rect(), rect, "{what} moved under the pointer");
    let around = with_ancestors(&node);
    let disabled = node.accesskit_node().is_disabled();
    let inside: Vec<egui::accesskit::NodeId> = harness
        .query_all(By::new())
        .filter(|n| with_ancestors(n)[1..].contains(&id))
        .map(id_of)
        .collect();
    // Whether the node takes a click itself, in the frame the press is
    // checked against.
    let clickable = harness.ctx.viewport(|v| {
        v.prev_pass.widgets.layers().any(|(_, widgets)| {
            widgets
                .iter()
                .any(|w| w.id.accesskit_id() == id && w.sense.senses_click())
        })
    });
    node.click();
    harness.step();
    // What egui gave the press: the node or one in it; one it lies in only
    // when the node takes no click itself (a label in a row, a disabled
    // button).
    let hit = harness
        .ctx
        .viewport(|v| v.hits.click.map(|w| w.id.accesskit_id()));
    let lands = hit.is_some_and(|hit| {
        hit == id || inside.contains(&hit) || ((!clickable || disabled) && around.contains(&hit))
    });
    let hit_node = hit.and_then(|hit| node_by_id(harness, hit));
    if !lands {
        let other = hit_node.map(|n| format!("{:?} at {:?}", n.accesskit_node().label(), n.rect()));
        panic!(
            "the click on {what} at {rect:?} landed on {}",
            other.unwrap_or_else(|| format!("{hit:?}"))
        );
    }
}

/// The accesskit id of `node`, for a [`click_node`] pick.
pub fn id_of(node: egui_kittest::Node<'_>) -> egui::accesskit::NodeId {
    node.accesskit_node().locate().0
}

/// Clicks the one node labelled `label` ([`click_node`]).
pub fn click(harness: &mut Harness<'_, Launcher>, label: &str) {
    click_node(harness, label, |h| {
        let mut all = h.query_all_by_label(label);
        let one = all.next()?;
        assert!(all.next().is_none(), "more than one node labelled {label}");
        Some(one.accesskit_node().locate().0)
    });
}

/// Clicks the one node with `role` and `label` ([`click_node`]).
pub fn click_role(harness: &mut Harness<'_, Launcher>, role: egui::accesskit::Role, label: &str) {
    click_node(harness, label, |h| {
        h.query_by_role_and_label(role, label)
            .map(|n| n.accesskit_node().locate().0)
    });
}

/// Steps until a node labelled `label` is in the tree and lies where it
/// lay a frame before: a node that only just showed may still move (the
/// auth pages centre their column by the height of the pass before,
/// GEL-78). A click needs more than that and waits for itself ([`click`]).
pub fn wait_for_label(harness: &mut Harness<'_, Launcher>, label: &str) {
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

/// [`click_role`] without waiting for the app ([`click_now`]).
pub fn click_role_now(
    harness: &mut Harness<'_, Launcher>,
    role: egui::accesskit::Role,
    label: &str,
) {
    click_now(harness, label, |h| {
        h.query_by_role_and_label(role, label).map(id_of)
    });
}

/// Clicks the last node with `role` and `label`: a dialog's button over
/// the page's of the same name ([`click_node`]).
pub fn click_last(harness: &mut Harness<'_, Launcher>, role: egui::accesskit::Role, label: &str) {
    click_node(harness, label, |h| {
        h.query_all_by_label(label)
            .rfind(|n| n.accesskit_node().role() == role)
            .map(id_of)
    });
}

/// The user panel's avatar button, which opens the account popover: the
/// lowest button named after `name` (an unread feedback reply adds to its
/// name).
pub fn account_button<'a>(
    harness: &'a Harness<'_, Launcher>,
    name: &str,
) -> Option<egui_kittest::Node<'a>> {
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
    click_node(harness, "the avatar", |h| {
        account_button(h, name).map(|n| n.accesskit_node().locate().0)
    });
    wait_for_label(harness, "Abmelden");
}
