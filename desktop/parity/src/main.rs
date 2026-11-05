//! Native parity check: renders every scene of web/scripts/parity/scenes.json
//! with the real app against the seeded stack (headless, egui_kittest on
//! wgpu) and compares it with the web screenshot of the same scene (SSIM).
//! A scene passes when SSIM, pixel match and edge match all reach 0.90
//! (see pixels.rs for the two stricter measures).
//!
//!   gelabber-parity render <out-dir> [scene,…]
//!   gelabber-parity compare <web-dir> <native-dir> <report-dir>
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
            .wgpu()
            .build_eframe(|cc| {
                gelabber_ui::GelabberApp::new(cc, client.clone(), Some(route.clone()))
            });
        // Step until the app has nothing in flight for a while: data,
        // images and layout settle like the web page after networkidle.
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
        harness.run_steps(5);
        let image = harness
            .render()
            .unwrap_or_else(|e| panic!("render {}: {e}", scene.name));
        let path = out.join(format!("{}.png", scene.name));
        image.save(&path).expect("save png");
        println!("{}: {route} -> {}", scene.name, path.display());
    }
}

fn compare(web: &Path, native: &Path, report: &Path) -> bool {
    let scenes: Scenes = read("scenes.json");
    std::fs::create_dir_all(report).expect("create report directory");
    let mut all = true;
    let mut lines = vec![
        "| scene | SSIM | pixels | edges | result |".to_owned(),
        "|---|---|---|---|---|".to_owned(),
    ];
    for scene in &scenes.scenes {
        let (a, b) = (
            web.join(format!("{}.png", scene.name)),
            native.join(format!("{}.png", scene.name)),
        );
        let (Ok(a), Ok(b)) = (image::open(&a), image::open(&b)) else {
            lines.push(format!("| {} | – | – | – | missing |", scene.name));
            all = false;
            continue;
        };
        let (a, b) = (a.to_rgba8(), b.to_rgba8());
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
            let only = args
                .get(2)
                .map(|s| s.split(',').map(str::to_owned).collect());
            render(Path::new(&args[1]), only);
        }
        Some("compare") if args.len() == 4 => {
            if !compare(
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
            ) {
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: gelabber-parity render <out-dir> [scene,…] | compare <web-dir> <native-dir> <report-dir>"
            );
            std::process::exit(2);
        }
    }
}
