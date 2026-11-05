//! A call against the seeded stack (web/scripts/parity), with fake media:
//! Lena joins Sofa where Jonas (camera) and Mira (muted) wait in headless
//! Chromium (call.mjs, which this test starts and stops).
//!
//!   GELABBER_TEST_SERVER=http://127.0.0.1:5180 \
//!   cargo test -p gelabber-call --features media --test live -- --ignored --nocapture
#![cfg(feature = "media")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gelabber_call::{Call, CallStatus, VideoKey};
use gelabber_client::{Client, GatewayEvent, GatewayStatus, ServerFrame, SigKind, TrackKind};
use parking_lot::Mutex;
use serde_json::Value;
use uuid::Uuid;

/// The `sig` frames about Lena that her observer saw.
type Seen = Arc<Mutex<Vec<(SigKind, Option<TrackKind>)>>>;

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/scripts/parity")
}

/// call.mjs, ended with SIGTERM so the browsers leave cleanly.
struct People(std::process::Child);

impl People {
    fn start(spec: &[&str]) -> Self {
        let mut child = std::process::Command::new("node")
            .current_dir(parity_dir().join("../.."))
            .arg(parity_dir().join("call.mjs"))
            .args(spec)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("node call.mjs");
        let out = child.stdout.take().expect("stdout");
        for line in BufReader::new(out).lines() {
            let line = line.expect("call.mjs output");
            eprintln!("call.mjs: {line}");
            if line.contains("in the call") {
                break;
            }
        }
        Self(child)
    }
}

impl Drop for People {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kill")
            .arg(self.0.id().to_string())
            .status();
        let _ = self.0.wait();
    }
}

fn wait(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < limit, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("ok after {:?}: {what}", start.elapsed());
}

fn signed_in(base: &str, email: &str, password: &str, server: Uuid) -> Client {
    let client = Client::new(base).expect("client");
    client.login_blocking(email, password).expect("login");
    client.bootstrap();
    wait("gateway open", Duration::from_secs(20), || {
        client.with_state(|s| s.gateway == GatewayStatus::Open && s.servers_loaded)
    });
    client.open_server(server);
    wait("server detail", Duration::from_secs(20), || {
        client.with_state(|s| s.server_details.contains_key(&server))
    });
    client
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER) and call.mjs"]
fn lena_joins_sofa_with_jonas_and_mira() {
    let Ok(base) = std::env::var("GELABBER_TEST_SERVER") else {
        eprintln!("GELABBER_TEST_SERVER is not set");
        return;
    };
    // SAFETY: set before any thread of this test reads the environment.
    unsafe { std::env::set_var("GELABBER_FAKE_MEDIA", "1") };
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(parity_dir().join("fixture.json")).unwrap())
            .unwrap();
    let id = |v: &Value| v.as_str().unwrap().parse::<Uuid>().unwrap();
    let password = fixture["password"].as_str().unwrap();
    let home = id(&fixture["servers"]["home"]["id"]);
    let sofa = id(&fixture["servers"]["home"]["channels"]["Sofa"]);
    let lena = id(&fixture["users"]["lena"]["id"]);
    let jonas = id(&fixture["users"]["jonas"]["id"]);
    let mira = id(&fixture["users"]["mira"]["id"]);

    let _people = People::start(&["Sofa", "jonas:camera,mira:muted"]);

    // Jonas' second session, outside the call: what the others see of Lena.
    let observer = signed_in(
        &base,
        fixture["users"]["jonas"]["email"].as_str().unwrap(),
        password,
        home,
    );
    let published: Seen = Arc::default();
    {
        let published = published.clone();
        observer.set_voice_hook(Some(Arc::new(move |event: &GatewayEvent| {
            if let GatewayEvent::Frame(ServerFrame::Sig(sig)) = event
                && sig.u == Some(lena)
            {
                published.lock().push((sig.t, sig.k));
            }
        })));
    }
    let seat_of_lena =
        || observer.with_state(|s| s.voice.get(&home).and_then(|v| v.get(&lena)).copied());

    let client = signed_in(
        &base,
        fixture["users"]["lena"]["email"].as_str().unwrap(),
        password,
        home,
    );
    let call = Call::new(client.clone());
    call.join(home, sofa);
    wait("connected", Duration::from_secs(30), || {
        call.with_state(|s| s.status == CallStatus::Connected)
    });
    wait(
        "Jonas and Mira in the room",
        Duration::from_secs(10),
        || {
            call.with_state(|s| {
                let ids: Vec<Uuid> = s.participants.iter().map(|p| p.user).collect();
                ids.contains(&lena) && ids.contains(&jonas) && ids.contains(&mira)
            })
        },
    );
    assert!(
        call.with_state(|s| s.participants.iter().any(|p| p.user == mira && p.muted)),
        "Mira is muted"
    );
    wait("the others see Lena", Duration::from_secs(10), || {
        seat_of_lena().is_some_and(|o| o.channel_id == sofa)
    });
    wait(
        "Lena announces her microphone",
        Duration::from_secs(10),
        || published.lock().contains(&(SigKind::P, Some(TrackKind::A))),
    );

    // Jonas' fake microphone beeps: his voice is consumed and plays.
    wait("Jonas' tone is heard", Duration::from_secs(15), || {
        call.levels().get(&jonas).is_some_and(|l| *l > 0.0)
    });

    call.set_muted(true);
    wait("muted for the others", Duration::from_secs(10), || {
        seat_of_lena().is_some_and(|o| o.muted)
    });
    call.set_muted(false);
    wait("unmuted for the others", Duration::from_secs(10), || {
        seat_of_lena().is_some_and(|o| !o.muted)
    });

    // Jonas' camera arrives as frames.
    wait("Jonas' camera", Duration::from_secs(20), || {
        call.frame(VideoKey::camera(jonas))
            .is_some_and(|f| f.serial > 5 && f.width > 0)
    });
    let frame = call.frame(VideoKey::camera(jonas)).unwrap();
    eprintln!("Jonas' camera: {}x{}", frame.width, frame.height);

    // Lena's camera: the test pattern, its preview, and the others see it.
    call.set_camera(true);
    wait("own camera preview", Duration::from_secs(10), || {
        call.frame(VideoKey::camera(lena))
            .is_some_and(|f| f.serial > 5)
    });
    wait("the others see the camera", Duration::from_secs(10), || {
        published.lock().contains(&(SigKind::P, Some(TrackKind::V)))
    });
    assert!(call.with_state(|s| {
        s.participants
            .iter()
            .any(|p| p.me && p.camera == Some(VideoKey::camera(lena)))
    }));
    call.set_camera(false);
    wait("camera off for the others", Duration::from_secs(10), || {
        published.lock().contains(&(SigKind::U, Some(TrackKind::V)))
    });

    call.leave();
    assert_eq!(call.state().status, CallStatus::Idle);
    wait("no stray seat", Duration::from_secs(10), || {
        seat_of_lena().is_none()
    });
    // The seat stays gone (no rebuild after leaving).
    std::thread::sleep(Duration::from_secs(3));
    assert!(seat_of_lena().is_none(), "Lena came back");
}
