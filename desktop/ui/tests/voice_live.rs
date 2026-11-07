//! The voice feature end to end in the real app (egui_kittest on wgpu)
//! against the seeded stack, with fake media standing in for devices:
//! Lena joins Sofa where Jonas (camera) and Mira (muted) wait in headless
//! Chromium (voice_observer.mjs, which this test starts, reads and stops).
//!
//!   GELABBER_TEST_SERVER=http://127.0.0.1:5180 \
//!   cargo test -p gelabber-ui --features media --test voice_live -- --ignored --nocapture
#![cfg(feature = "media")]

use std::io::{BufRead, BufReader, Lines, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use gelabber_call::{CallStatus, VideoKey};
use gelabber_ui::GelabberApp;
use serde_json::{Value, json};
use uuid::Uuid;

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/scripts/parity")
}

/// The people in the channel and their pages; they leave when dropped.
struct Observer {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
}

impl Observer {
    fn start(channel: &str, people: &str) -> Self {
        let mut child = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/voice_observer.mjs"))
            .args([channel, people])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("node voice_observer.mjs");
        let input = child.stdin.take().expect("stdin");
        let mut output = BufReader::new(child.stdout.take().expect("stdout")).lines();
        loop {
            let line = output
                .next()
                .expect("observer ended before everyone joined")
                .expect("observer output");
            eprintln!("observer: {line}");
            if line == "ready" {
                break;
            }
        }
        Self {
            child,
            input,
            output,
        }
    }

    fn ask(&mut self, command: Value) -> Value {
        writeln!(self.input, "{command}").expect("write to observer");
        loop {
            let line = self.output.next().expect("observer ended").expect("read");
            if let Some(answer) = line.strip_prefix("= ") {
                let answer: Value = serde_json::from_str(answer).expect("observer JSON");
                assert!(answer.get("error").is_none(), "{command}: {answer}");
                return answer;
            }
        }
    }

    /// The card of `name` in the room as `who`'s page shows it.
    fn card(&mut self, who: &str, name: &str) -> Option<Value> {
        let state = self.ask(json!({ "op": "state", "who": who }));
        state["room"]
            .as_array()?
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
    }

    fn click(&mut self, who: &str, name: &str, role: &str) {
        self.ask(json!({ "op": "click", "who": who, "name": name, "role": role }));
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        // SIGTERM lets the browsers leave the call cleanly.
        let _ = Command::new("kill")
            .arg(self.child.id().to_string())
            .status();
        let _ = self.child.wait();
    }
}

type App = Harness<'static, GelabberApp>;

/// Frames for `ms` at about 60 per second.
fn pump(h: &mut App, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        h.step();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn until(h: &mut App, what: &str, limit: Duration, mut done: impl FnMut(&mut App) -> bool) {
    let start = Instant::now();
    while !done(h) {
        assert!(start.elapsed() < limit, "timed out: {what}");
        pump(h, 50);
    }
    eprintln!("ok after {:?}: {what}", start.elapsed());
}

/// Waits until `who`'s page shows `name`'s card passing `check`.
fn web_until(
    h: &mut App,
    obs: &mut Observer,
    what: &str,
    who: &str,
    name: &str,
    mut check: impl FnMut(Option<&Value>) -> bool,
) -> Option<Value> {
    let start = Instant::now();
    loop {
        let card = obs.card(who, name);
        if check(card.as_ref()) {
            eprintln!("ok after {:?}: {what}: {card:?}", start.elapsed());
            return card;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out: {what}: {card:?}"
        );
        pump(h, 250);
    }
}

fn button(h: &App, label: &str) -> egui::Rect {
    h.query_all_by_role_and_label(Role::Button, label)
        .next()
        .unwrap_or_else(|| panic!("no button {label:?}"))
        .rect()
}

fn has_button(h: &App, label: &str) -> bool {
    h.query_all_by_role_and_label(Role::Button, label)
        .next()
        .is_some()
}

fn click(h: &mut App, label: &str) {
    h.query_all_by_role_and_label(Role::Button, label)
        .next()
        .unwrap_or_else(|| panic!("no button {label:?}"))
        .click();
    pump(h, 100);
}

fn status(h: &App, name: &str) -> String {
    let status = h.state().call().state().status;
    format!("{name}: {status:?}")
}

/// How many pixels differ between two renders inside `rect`, and how many
/// colours the second has there.
fn picture_change(a: &image::RgbaImage, b: &image::RgbaImage, rect: egui::Rect) -> (f64, usize) {
    let mut changed = 0usize;
    let mut total = 0usize;
    let mut colours = std::collections::HashSet::new();
    for y in rect.top() as u32..rect.bottom() as u32 {
        for x in rect.left() as u32..rect.right() as u32 {
            let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
            total += 1;
            if p.0.iter().zip(q.0).any(|(u, v)| u.abs_diff(v) > 8) {
                changed += 1;
            }
            colours.insert([q[0] >> 3, q[1] >> 3, q[2] >> 3]);
        }
    }
    (changed as f64 / total.max(1) as f64, colours.len())
}

#[test]
#[ignore = "needs the seeded stack (GELABBER_TEST_SERVER) and Chromium"]
fn voice_end_to_end() {
    let server = std::env::var("GELABBER_TEST_SERVER").expect("GELABBER_TEST_SERVER");
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(parity_dir().join("fixture.json")).expect("fixture.json"),
    )
    .expect("fixture JSON");
    let config = std::env::temp_dir().join(format!("gelabber-voice-live-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    // The web shots' reduced motion keeps rings and glows still between renders.
    std::fs::write(config.join("settings.json"), r#"{"reducedMotion": true}"#).unwrap();
    // SAFETY: set before the app or any thread of this test reads them.
    unsafe {
        std::env::set_var("GELABBER_FAKE_MEDIA", "1");
        std::env::set_var("GELABBER_FAKE_SCREEN", "0");
        std::env::set_var("GELABBER_CONFIG_DIR", &config);
    }
    let home = fixture["servers"]["home"]["id"].as_str().unwrap();
    let sofa = fixture["servers"]["home"]["channels"]["Sofa"]
        .as_str()
        .unwrap();
    let jonas: Uuid = fixture["users"]["jonas"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mira: Uuid = fixture["users"]["mira"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let lena: Uuid = fixture["users"]["lena"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let mut obs = Observer::start("Sofa", "jonas:camera,mira:muted");
    let client = gelabber_client::Client::new(&server).expect("client");
    client
        .login_blocking(
            fixture["users"]["lena"]["email"].as_str().unwrap(),
            fixture["password"].as_str().unwrap(),
        )
        .expect("login lena");
    let route = format!("/s/{home}/c/{sofa}");
    let mut h: App = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_max_steps(10_000)
        .with_render_cursor(false)
        .wgpu()
        .build_eframe(move |cc| GelabberApp::new(cc, client.clone(), Some(route.clone())));

    // ---- join
    until(&mut h, "Beitreten shown", Duration::from_secs(20), |h| {
        has_button(h, "Beitreten")
    });
    click(&mut h, "Beitreten");
    until(
        &mut h,
        "connected with Jonas and Mira",
        Duration::from_secs(20),
        |h| {
            let s = h.state().call().state();
            s.status == CallStatus::Connected
                && [lena, jonas, mira]
                    .iter()
                    .all(|u| s.participants.iter().any(|p| p.user == *u))
        },
    );
    assert!(has_button(&h, "Verlassen"), "call bar after joining");
    web_until(&mut h, &mut obs, "Jonas sees Lena", "jonas", "Lena", |c| {
        c.is_some_and(|c| {
            c["status"].as_str().unwrap_or("").starts_with("Im Raum")
                || c["status"].as_str().unwrap_or("").starts_with("Spricht")
        })
    });

    // ---- speaking: no flicker over six seconds, as the web's hold keeps it
    pump(&mut h, 1500);
    let mut flips = [0u32; 2];
    let mut last = [None::<bool>; 2];
    let mut on = [0u32; 2];
    let mut samples = 0u32;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(6) {
        h.step();
        let s = h.state().call().state();
        for (i, user) in [lena, jonas].iter().enumerate() {
            let now = s.participants.iter().any(|p| p.user == *user && p.speaking);
            if last[i].is_some_and(|l| l != now) {
                flips[i] += 1;
            }
            last[i] = Some(now);
            on[i] += u32::from(now);
        }
        samples += 1;
        std::thread::sleep(Duration::from_millis(16));
    }
    eprintln!(
        "speaking over 6 s: Lena on {}/{samples} flips {}, Jonas on {}/{samples} flips {}",
        on[0], flips[0], on[1], flips[1]
    );
    assert_eq!(flips, [0, 0], "speaking rings flicker");
    assert_eq!(on, [samples, samples], "Lena and Jonas speak throughout");

    // ---- Jonas' camera in a native tile, frames advancing
    let key = VideoKey::camera(jonas);
    until(
        &mut h,
        "Jonas' camera frames",
        Duration::from_secs(15),
        |h| h.state().call().frame(key).is_some(),
    );
    let first = h.state().call().frame(key).unwrap();
    pump(&mut h, 1000);
    let second = h.state().call().frame(key).unwrap();
    eprintln!(
        "Jonas' camera: {}x{}, serial {} -> {}",
        second.width, second.height, first.serial, second.serial
    );
    assert!(second.serial > first.serial + 10, "camera frames advance");
    let more = button(&h, "Optionen für Jonas");
    // The picture above and left of its "…" button, in its bottom corner.
    let region = egui::Rect::from_min_max(
        egui::pos2(
            (more.right() - 600.0).max(0.0),
            (more.top() - 400.0).max(0.0),
        ),
        egui::pos2(more.right(), more.top()),
    );
    let a = h.render().expect("render");
    pump(&mut h, 700);
    let b = h.render().expect("render");
    if let Some(dir) = std::env::var_os("GELABBER_TEST_SHOTS") {
        let _ = b.save(Path::new(&dir).join("voice-camera.png"));
    }
    let (change, colours) = picture_change(&a, &b, region);
    eprintln!(
        "Jonas' tile {region:?}: {:.1} % of pixels changed, {colours} colours",
        change * 100.0
    );
    assert!(
        change > 0.01 && colours > 20,
        "the tile shows a moving picture"
    );

    // ---- mute and deafen, as Jonas' browser shows them
    click(&mut h, "Stummschalten");
    assert!(h.state().call().state().muted, "{}", status(&h, "muted"));
    web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena muted",
        "jonas",
        "Lena",
        |c| c.is_some_and(|c| c["status"].as_str().unwrap_or("").starts_with("Stumm")),
    );
    click(&mut h, "Stummschalten");
    web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena unmuted",
        "jonas",
        "Lena",
        |c| c.is_some_and(|c| !c["status"].as_str().unwrap_or("").starts_with("Stumm")),
    );
    click(&mut h, "Taubschalten");
    assert!(h.state().call().state().deafened);
    web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena deafened",
        "jonas",
        "Lena",
        |c| c.is_some_and(|c| c["status"].as_str().unwrap_or("").starts_with("Taub")),
    );
    click(&mut h, "Taubschalten");
    web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena hear again",
        "jonas",
        "Lena",
        |c| {
            c.is_some_and(|c| {
                let s = c["status"].as_str().unwrap_or("");
                !s.starts_with("Taub") && !s.starts_with("Stumm")
            })
        },
    );

    // ---- own camera (test pattern), shown in Jonas' browser
    click(&mut h, "Kamera");
    until(&mut h, "camera on", Duration::from_secs(5), |h| {
        h.state().call().state().camera
    });
    let seen = web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena's camera",
        "jonas",
        "Lena",
        |c| {
            c.is_some_and(|c| {
                c["video"]["width"].as_u64().unwrap_or(0) > 0
                    && c["video"]["frames"].as_u64().unwrap_or(0) > 0
            })
        },
    )
    .unwrap();
    pump(&mut h, 1000);
    let later = obs.card("jonas", "Lena").unwrap();
    eprintln!(
        "Lena's camera in Jonas' browser: {} -> {}",
        seen["video"], later["video"]
    );
    assert!(
        later["video"]["frames"].as_u64() > seen["video"]["frames"].as_u64(),
        "Lena's camera frames advance in Jonas' browser"
    );
    click(&mut h, "Kamera");
    web_until(
        &mut h,
        &mut obs,
        "Lena's camera off for Jonas",
        "jonas",
        "Lena",
        |c| c.is_some_and(|c| c["video"].is_null()),
    );

    // ---- Raster, Fokus, Kino
    let selected = |h: &App, label: &str| {
        h.query_all_by_role_and_label(Role::RadioButton, label)
            .next()
            .unwrap_or_else(|| panic!("no view {label:?}"))
            .accesskit_node()
            .toggled()
            == Some(egui::accesskit::Toggled::True)
    };
    let radio = |h: &mut App, label: &str| {
        h.query_all_by_role_and_label(Role::RadioButton, label)
            .next()
            .unwrap_or_else(|| panic!("no view {label:?}"))
            .click();
        pump(h, 150);
    };
    radio(&mut h, "Fokus");
    assert!(selected(&h, "Fokus"), "Fokus chosen");
    radio(&mut h, "Raster");
    assert!(selected(&h, "Raster"), "Raster chosen");
    click(&mut h, "Kino");
    assert!(has_button(&h, "Kino verlassen"), "Kino shows its exit");
    h.key_press(egui::Key::Escape);
    pump(&mut h, 150);
    assert!(!has_button(&h, "Kino verlassen"), "Esc leaves Kino");
    assert!(has_button(&h, "Kino"), "back on the room page");

    // ---- Go Live without a screen picker (no portal here): the web's message
    click(&mut h, "Live gehen");
    let bar_top = button(&h, "Verlassen").top();
    h.query_all_by_role_and_label(Role::Button, "Live gehen")
        .find(|n| n.rect().bottom() < bar_top)
        .expect("the Live menu's entry")
        .click();
    pump(&mut h, 300);
    if let Some(dir) = std::env::var_os("GELABBER_TEST_SHOTS") {
        let _ = h
            .render()
            .unwrap()
            .save(Path::new(&dir).join("voice-live.png"));
    }
    eprintln!("after Live gehen: live {}", h.state().call().state().live);
    let mut message = String::new();
    until(&mut h, "Live failure toast", Duration::from_secs(10), |h| {
        // egui gives a label's text as its value.
        if let Some(text) = h
            .query_all_by_role(Role::Label)
            .filter_map(|n| n.value())
            .find(|v| v.contains("Bildschirmaufnahme"))
        {
            message = text;
        }
        !message.is_empty()
    });
    eprintln!("Live failure: {message}");
    assert!(message.starts_with("Die Bildschirmaufnahme konnte nicht gestartet werden"));
    assert!(!h.state().call().state().live, "no Live after the failure");

    // ---- watching Jonas' Live, on, off and on again
    obs.click("jonas", "Live gehen", "button");
    pump(&mut h, 500);
    obs.click("jonas", "Live gehen", "menuitem");
    let live = VideoKey::live(jonas);
    for round in 0..2 {
        until(&mut h, "Zuschauen offered", Duration::from_secs(15), |h| {
            has_button(h, "Zuschauen")
        });
        assert!(
            h.state().call().frame(live).is_none(),
            "no picture before watching"
        );
        click(&mut h, "Zuschauen");
        until(&mut h, "Live frames", Duration::from_secs(15), |h| {
            h.state().call().frame(live).is_some()
        });
        let a = h.state().call().frame(live).unwrap();
        pump(&mut h, 1000);
        let b = h.state().call().frame(live).unwrap();
        eprintln!(
            "round {round}: Jonas' Live {}x{}, serial {} -> {}",
            b.width, b.height, a.serial, b.serial
        );
        assert!(b.serial > a.serial, "Live frames advance");
        assert!(
            h.state()
                .call()
                .state()
                .participants
                .iter()
                .any(|p| p.user == jonas && p.watching)
        );
        click(&mut h, "Nicht mehr zuschauen");
        until(
            &mut h,
            "Live stops after watching",
            Duration::from_secs(5),
            |h| h.state().call().frame(live).is_none(),
        );
    }
    obs.click("jonas", "Live gehen", "button");
    pump(&mut h, 500);
    obs.click("jonas", "Live beenden", "menuitem");

    // ---- per-person volume
    until(&mut h, "Jonas' Live gone", Duration::from_secs(10), |h| {
        !has_button(h, "Zuschauen")
    });
    pump(&mut h, 300);
    click(&mut h, "Optionen für Jonas");
    pump(&mut h, 200);
    if let Some(dir) = std::env::var_os("GELABBER_TEST_SHOTS") {
        let _ = h
            .render()
            .unwrap()
            .save(Path::new(&dir).join("voice-volume.png"));
    }
    let slider = h
        .query_all_by_role_and_label(Role::Slider, "Lautstärke")
        .next()
        .expect("the volume slider")
        .rect();
    // 50 of 0..200 on the slider is 50 %; the thumb's centre runs 7px in.
    let x = slider.left() + 7.0 + (slider.width() - 14.0) * 0.25;
    let at = egui::pos2(x, slider.center().y);
    // A click there: press and release without the pointer leaving, which
    // `drop_at` would do in the same frame.
    h.hover_at(at);
    pump(&mut h, 50);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
    }
    pump(&mut h, 200);
    let shown = h
        .query_all_by_role_and_label(Role::Slider, "Lautstärke")
        .next()
        .and_then(|n| n.accesskit_node().numeric_value());
    eprintln!("slider at {slider:?}, clicked at x {x}: {shown:?}");
    let volume = h
        .state()
        .call()
        .volumes()
        .get(&jonas)
        .copied()
        .unwrap_or_default();
    let played = h
        .state()
        .call()
        .state()
        .participants
        .iter()
        .find(|p| p.user == jonas)
        .map(|p| p.volume);
    eprintln!("Jonas' volume: {volume:?}, participant {played:?}");
    assert!((volume.volume - 0.5).abs() < 0.011, "slider sets 50 %");
    assert_eq!(played, Some(volume.volume as f32));
    h.query_all_by_role_and_label(Role::CheckBox, "Lokal stummschalten")
        .next()
        .expect("Lokal stummschalten")
        .click();
    pump(&mut h, 150);
    assert!(h.state().call().volumes()[&jonas].muted, "locally muted");
    click(&mut h, "Zurücksetzen");
    let reset = h
        .state()
        .call()
        .volumes()
        .get(&jonas)
        .copied()
        .unwrap_or_default();
    assert_eq!((reset.volume, reset.muted), (1.0, false), "reset");
    h.key_press(egui::Key::Escape);
    pump(&mut h, 150);

    // ---- leave
    click(&mut h, "Verlassen");
    until(&mut h, "left", Duration::from_secs(10), |h| {
        let s = h.state().call().state();
        s.channel.is_none() && s.status == CallStatus::Idle
    });
    web_until(
        &mut h,
        &mut obs,
        "Jonas sees Lena gone",
        "jonas",
        "Lena",
        |c| c.is_none(),
    );
    assert!(has_button(&h, "Beitreten"), "Beitreten again after leaving");
    drop(obs);
    let _ = std::fs::remove_dir_all(&config);
}
