//! Native parity check: renders every scene of web/scripts/parity/scenes.json
//! with the real app against the seeded stack (headless, egui_kittest on
//! wgpu) and compares it with the web screenshot of the same scene (SSIM).
//! A scene passes when SSIM, pixel match and edge match all reach 0.90
//! (see pixels.rs for the two stricter measures).
//!
//!   gelabber-parity render <out-dir> [scene,…]
//!   gelabber-parity compare <web-dir> <native-dir> <report-dir> [--only a,b] [--crop x,y,w,h]
//!
//! `--crop` scores one region (e.g. a page column) while the rest of the
//! window is not built yet; acceptance is always on the full scene.
//!
//! Reads web/scripts/parity/{scenes,fixture}.json from the repository.

mod pixels;
mod ssim;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

const THRESHOLD: f64 = 0.90;

#[derive(Deserialize)]
struct Scenes {
    size: [u32; 2],
    scenes: Vec<Scene>,
}

#[derive(Deserialize)]
struct Scene {
    name: String,
    path: String,
    user: Option<String>,
    /// `channel people` for call.mjs: people kept in a voice channel.
    call: Option<String>,
    /// Steps after the page settled, the same in both shot tools.
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    /// Click the button with this accessible name.
    Click(String),
    /// Wait this many milliseconds.
    Wait(u64),
}

/// call.mjs running for a scene; leaves the call when dropped.
struct Call(std::process::Child);

impl Call {
    fn start(spec: &str) -> Self {
        use std::io::{BufRead, BufReader};
        let mut child = std::process::Command::new("node")
            .arg(parity_dir().join("call.mjs"))
            .args(spec.split(' '))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("run call.mjs");
        let out = child.stdout.take().expect("call.mjs stdout");
        for line in BufReader::new(out).lines() {
            let line = line.expect("read call.mjs");
            if line.contains("in the call") {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
        Self(child)
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        // SIGTERM lets the browsers leave the call cleanly.
        let _ = std::process::Command::new("kill")
            .arg(self.0.id().to_string())
            .status();
        let _ = self.0.wait();
        std::thread::sleep(Duration::from_millis(1500));
    }
}

#[derive(Deserialize)]
struct Fixture {
    base: String,
    password: String,
    users: std::collections::BTreeMap<String, User>,
    servers: Servers,
    dm: String,
}

#[derive(Deserialize)]
struct User {
    email: String,
}

#[derive(Deserialize)]
struct Servers {
    home: Home,
    evening: Id,
}

#[derive(Deserialize)]
struct Home {
    id: String,
    channels: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/scripts/parity")
}

fn read<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = parity_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fill(path: &str, fixture: &Fixture) -> String {
    let mut ids = fixture.servers.home.channels.clone();
    ids.insert("home".into(), fixture.servers.home.id.clone());
    ids.insert("evening".into(), fixture.servers.evening.id.clone());
    ids.insert("dm".into(), fixture.dm.clone());
    let mut out = path.to_owned();
    for (key, value) in ids {
        out = out.replace(&format!("{{{key}}}"), &value);
    }
    out
}

fn render(out: &Path, only: Option<Vec<String>>) {
    let scenes: Scenes = read("scenes.json");
    let fixture: Fixture = read("fixture.json");
    std::fs::create_dir_all(out).expect("create output directory");
    for scene in &scenes.scenes {
        if only.as_ref().is_some_and(|o| !o.contains(&scene.name)) {
            continue;
        }
        // Opening a channel marks it read; every scene starts from the reference.
        let reset = std::process::Command::new("node")
            .arg(parity_dir().join("reset.mjs"))
            .status()
            .expect("run reset.mjs");
        assert!(reset.success(), "reset.mjs failed");
        let _call = scene.call.as_deref().map(Call::start);
        let client = gelabber_client::Client::new(&fixture.base).expect("client");
        if let Some(user) = &scene.user {
            let email = &fixture.users[user].email;
            client
                .login_blocking(email, &fixture.password)
                .unwrap_or_else(|e| panic!("login {user}: {e}"));
        }
        let route = fill(&scene.path, &fixture);
        let size = egui::vec2(scenes.size[0] as f32, scenes.size[1] as f32);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .with_max_steps(10_000)
            // The web shots have no pointer drawn over them.
            .with_render_cursor(false)
            .wgpu()
            .build_eframe(|cc| {
                gelabber_ui::GelabberApp::new(cc, client.clone(), Some(route.clone()))
            });
        // Step until the app has nothing in flight for a while: data,
        // images and layout settle like the web page after networkidle.
        settle(&mut harness);
        for action in &scene.actions {
            match action {
                Action::Click(name) => {
                    use egui_kittest::kittest::Queryable;
                    harness
                        .query_all_by_role_and_label(egui::accesskit::Role::Button, name)
                        .next()
                        .unwrap_or_else(|| panic!("{}: no button {name:?}", scene.name))
                        .click();
                }
                Action::Wait(ms) => {
                    let until = Instant::now() + Duration::from_millis(*ms);
                    while Instant::now() < until {
                        harness.step();
                        std::thread::sleep(Duration::from_millis(16));
                    }
                }
            }
            settle(&mut harness);
        }
        harness.run_steps(5);
        let image = harness
            .render()
            .unwrap_or_else(|e| panic!("render {}: {e}", scene.name));
        let path = out.join(format!("{}.png", scene.name));
        image.save(&path).expect("save png");
        println!("{}: {route} -> {}", scene.name, path.display());
    }
}

/// Steps until the app has had nothing in flight for ten frames.
fn settle(harness: &mut egui_kittest::Harness<'_, gelabber_ui::GelabberApp>) {
    let start = Instant::now();
    let mut quiet = 0;
    while quiet < 10 && start.elapsed() < Duration::from_secs(20) {
        harness.step();
        if harness.state().busy() {
            quiet = 0;
            std::thread::sleep(Duration::from_millis(20));
        } else {
            quiet += 1;
        }
    }
}

fn compare(
    web: &Path,
    native: &Path,
    report: &Path,
    only: Option<&[String]>,
    crop: Option<[u32; 4]>,
) -> bool {
    let scenes: Scenes = read("scenes.json");
    std::fs::create_dir_all(report).expect("create report directory");
    let mut all = true;
    let mut lines = vec![
        "| scene | SSIM | pixels | edges | result |".to_owned(),
        "|---|---|---|---|---|".to_owned(),
    ];
    for scene in &scenes.scenes {
        if only.is_some_and(|o| !o.contains(&scene.name)) {
            continue;
        }
        let (a, b) = (
            web.join(format!("{}.png", scene.name)),
            native.join(format!("{}.png", scene.name)),
        );
        let (Ok(a), Ok(b)) = (image::open(&a), image::open(&b)) else {
            lines.push(format!("| {} | – | – | – | missing |", scene.name));
            all = false;
            continue;
        };
        let (mut a, mut b) = (a.to_rgba8(), b.to_rgba8());
        if let Some([x, y, w, h]) = crop {
            a = image::imageops::crop_imm(&a, x, y, w, h).to_image();
            b = image::imageops::crop_imm(&b, x, y, w, h).to_image();
        }
        if a.dimensions() != b.dimensions() {
            lines.push(format!(
                "| {} | – | – | – | size {:?} vs {:?} |",
                scene.name,
                a.dimensions(),
                b.dimensions()
            ));
            all = false;
            continue;
        }
        let score = ssim::compare(&a, &b);
        let (pixel, map) = pixels::pixel_match(&a, &b);
        let edge = pixels::edge_match(&a, &b);
        let pass = score.ssim >= THRESHOLD && pixel >= THRESHOLD && edge >= THRESHOLD;
        all &= pass;
        score
            .diff
            .save(report.join(format!("{}-ssim.png", scene.name)))
            .expect("save diff");
        map.save(report.join(format!("{}-pixels.png", scene.name)))
            .expect("save map");
        lines.push(format!(
            "| {} | {:.4} | {:.4} | {:.4} | {} |",
            scene.name,
            score.ssim,
            pixel,
            edge,
            if pass { "pass" } else { "FAIL" }
        ));
    }
    let table = lines.join("\n");
    println!("{table}");
    std::fs::write(report.join("report.md"), table + "\n").expect("write report");
    all
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") if args.len() >= 2 => {
            // No microphone or camera here: calls run on the core's dummy
            // audio and a test pattern, like Chromium's fake devices.
            if std::env::var_os("GELABBER_FAKE_MEDIA").is_none() {
                // SAFETY: before any other thread of this process runs.
                unsafe { std::env::set_var("GELABBER_FAKE_MEDIA", "1") };
            }
            let only = args
                .get(2)
                .map(|s| s.split(',').map(str::to_owned).collect());
            render(Path::new(&args[1]), only);
        }
        Some("compare") if args.len() >= 4 => {
            let flag = |name: &str| {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|i| args.get(i + 1))
            };
            let only: Option<Vec<String>> =
                flag("--only").map(|s| s.split(',').map(str::to_owned).collect());
            let crop = flag("--crop").map(|s| {
                let v: Vec<u32> = s
                    .split(',')
                    .map(|n| n.parse().expect("--crop x,y,w,h"))
                    .collect();
                [v[0], v[1], v[2], v[3]]
            });
            if !compare(
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
                only.as_deref(),
                crop,
            ) {
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: gelabber-parity render <out-dir> [scene,…] | compare <web-dir> <native-dir> <report-dir> [--only a,b] [--crop x,y,w,h]"
            );
            std::process::exit(2);
        }
    }
}
