//! The gateway test client. A socket answers the server's heartbeats while
//! the test waits, as a real client does, and every wait has a deadline: a
//! frame that never comes fails the test with what came instead, it does
//! not hang it.

use std::future::Future;
use std::net::SocketAddr;
use std::panic::Location;
use std::time::Duration;

use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{COOKIE, ORIGIN};

use super::{Client, SESSION};

pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// How long one wait may take: for one frame, for the frame a predicate
/// wants, or for the socket to close.
pub const WAIT: Duration = Duration::from_secs(5);

/// Frames kept for the message of a wait that fails.
const SEEN: usize = 20;

/// `client`'s socket on `/ws`.
pub async fn connect(addr: SocketAddr, client: &Client) -> Ws {
    connect_to(addr, "/ws", Some(&client.jar[SESSION]), None).await
}

/// A socket on `path` (`/ws?caps=dnd`, say) with the session cookie and
/// `Origin` header given, if any. Panics unless the upgrade succeeds.
pub async fn connect_to(
    addr: SocketAddr,
    path: &str,
    session: Option<&str>,
    origin: Option<&str>,
) -> Ws {
    let mut request = format!("ws://{addr}{path}")
        .into_client_request()
        .expect("ws request");
    if let Some(session) = session {
        request.headers_mut().insert(
            COOKIE,
            format!("{SESSION}={session}")
                .parse()
                .expect("cookie header"),
        );
    }
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(ORIGIN, origin.parse().expect("origin header"));
    }
    let (ws, response) = tokio::time::timeout(WAIT, tokio_tungstenite::connect_async(request))
        .await
        .expect("ws upgrade within the wait")
        .unwrap_or_else(|err| panic!("ws connect failed: {err}"));
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws
}

pub async fn send(ws: &mut Ws, frame: Value) {
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .expect("ws send");
}

/// The next text frame before `deadline`, or `None` once it has passed.
/// A server heartbeat is answered, and returned like any frame. Panics if
/// the socket closes or sends something other than JSON text.
async fn next_before(ws: &mut Ws, deadline: Instant) -> Option<Value> {
    loop {
        let message = match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_) => return None,
            Ok(None) => panic!("ws closed"),
            Ok(Some(message)) => message.expect("ws error"),
        };
        match message {
            Message::Text(text) => {
                let frame: Value = serde_json::from_str(text.as_str()).expect("json frame");
                if frame["op"] == "h" {
                    send(ws, json!({ "op": "h" })).await;
                }
                return Some(frame);
            }
            // Pings are answered by the library.
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// The next text frame within [`WAIT`].
#[track_caller]
pub fn recv(ws: &mut Ws) -> impl Future<Output = Value> + '_ {
    let at = Location::caller();
    async move {
        next_before(ws, Instant::now() + WAIT)
            .await
            .unwrap_or_else(|| panic!("no frame within {WAIT:?} (waited at {at})"))
    }
}

/// The first frame `want` accepts, within [`WAIT`] in all.
#[track_caller]
pub fn until<'a>(
    ws: &'a mut Ws,
    want: impl Fn(&Value) -> bool + 'a,
) -> impl Future<Output = Value> + 'a {
    let at = Location::caller();
    async move { skipping(ws, want, at).await.0 }
}

/// [`until`], with the frames other than heartbeats skipped on the way, to
/// check what came first.
#[track_caller]
pub fn until_skipping<'a>(
    ws: &'a mut Ws,
    want: impl Fn(&Value) -> bool + 'a,
) -> impl Future<Output = (Value, Vec<Value>)> + 'a {
    let at = Location::caller();
    skipping(ws, want, at)
}

async fn skipping(
    ws: &mut Ws,
    want: impl Fn(&Value) -> bool,
    at: &'static Location<'static>,
) -> (Value, Vec<Value>) {
    let deadline = Instant::now() + WAIT;
    let mut skipped = Vec::new();
    let mut heartbeats = 0;
    loop {
        match next_before(ws, deadline).await {
            Some(frame) if want(&frame) => return (frame, skipped),
            Some(frame) if frame["op"] == "h" => heartbeats += 1,
            Some(frame) => skipped.push(frame),
            None => {
                let start = skipped.len().saturating_sub(SEEN);
                panic!(
                    "no matching frame within {WAIT:?} (waited at {at}); {heartbeats} \
                     heartbeats and {} other frames came, the last ones {:?}",
                    skipped.len(),
                    &skipped[start..]
                )
            }
        }
    }
}

/// The first frame `want` accepts within `wait`, or `None`. For a check
/// that something does not arrive, `wait` should follow an event that
/// proves the work is done, so the check cannot be early.
pub async fn within(ws: &mut Ws, wait: Duration, want: impl Fn(&Value) -> bool) -> Option<Value> {
    let deadline = Instant::now() + wait;
    loop {
        match next_before(ws, deadline).await {
            Some(frame) if want(&frame) => return Some(frame),
            Some(_) => {}
            None => return None,
        }
    }
}

/// Waits within [`WAIT`] until the server closes the socket (a close frame,
/// an error or the end of the stream) and returns the text frames that came
/// before. Heartbeats are answered, so a socket that only went quiet does
/// not count as closed.
#[track_caller]
pub fn closed(ws: &mut Ws) -> impl Future<Output = Vec<Value>> + '_ {
    let at = Location::caller();
    closed_at(ws, at)
}

async fn closed_at(ws: &mut Ws, at: &'static Location<'static>) -> Vec<Value> {
    let deadline = Instant::now() + WAIT;
    let mut before = Vec::new();
    loop {
        let message = match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_) => panic!(
                "the socket did not close within {WAIT:?} (waited at {at}); it sent {before:?}"
            ),
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return before,
            Ok(Some(Ok(message))) => message,
        };
        if let Message::Text(text) = message {
            let frame: Value = serde_json::from_str(text.as_str()).expect("json frame");
            if frame["op"] == "h" {
                // The socket may be closing already; then the next read ends.
                let _ = ws
                    .send(Message::Text(json!({ "op": "h" }).to_string().into()))
                    .await;
            } else {
                before.push(frame);
            }
        }
    }
}

/// Answers a heartbeat and waits for the server's next one. A socket that
/// has done this is registered with the gateway: frames published after it
/// reach the socket.
#[track_caller]
pub fn registered(ws: &mut Ws) -> impl Future<Output = ()> + '_ {
    let at = Location::caller();
    async move {
        send(ws, json!({ "op": "h" })).await;
        skipping(ws, |f| f["op"] == "h", at).await;
    }
}
