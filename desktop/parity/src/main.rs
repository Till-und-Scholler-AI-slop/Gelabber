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
//! A scene's actions (click, link, rightclick, clicktext, hover, key, type,
//! say, wait) mean the same in shots.mjs; `before` runs say.mjs words ahead
//! of the page, posts and `after` are undone with say.mjs once the
//! screenshot is taken. A scene with `page` shows a bundled page of the
//! desktop app (desktop/app/setup) instead of a route: the Launcher here.

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
    #[serde(default)]
    path: String,
    /// A bundled page of the desktop app, e.g. "desktop/app/setup/index.html".
    page: Option<String>,
    /// The page's query, e.g. "?server=…&error=…".
    #[serde(default)]
    query: String,
    /// The system's colour scheme, "light" (Playwright's default) or "dark".
    scheme: Option<String>,
    user: Option<String>,
    /// `channel people` for call.mjs: people kept in a voice channel.
    call: Option<String>,
    /// Steps after the page settled, the same in both shot tools.
    #[serde(default)]
    actions: Vec<Action>,
    /// say.mjs words run before the page loads (a theme); reset.mjs undoes them.
    before: Option<String>,
    /// say.mjs words run after the screenshot (undo what dialogs made).
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    /// Click the button with this accessible name.
    Click(String),
    /// Click the link with this accessible name.
    Link(String),
    /// Right-click the first button or link with this accessible name.
    Rightclick(String),
    /// Click the first element with this label, whatever its role.
    Clicktext(String),
    /// Rest the pointer on the first button or link, else element, with this label;
    /// it stays there for the screenshot.
    Hover(String),
    /// Press a key, Playwright style: "Control+k", "ArrowDown", "Escape".
    Key(String),
    /// Type text into whatever has the focus.
    Type(String),
    /// "<user> <channel-key> <text>": someone else writes (say.mjs); the
    /// message is deleted after the screenshot.
    Say(String),
    /// Wait this many milliseconds.
    Wait(u64),
}

/// What a scene posted and what it undoes afterwards: run when dropped,
/// so a failing step leaves the fixture as seeded too.
#[derive(Default)]
struct Undo {
    said: Vec<(String, String)>,
    after: Option<String>,
}

impl Drop for Undo {
    fn drop(&mut self) {
        for (who, id) in &self.said {
            say(&["delete", who, id]);
        }
        if let Some(after) = &self.after {
            say(&after.split(' ').collect::<Vec<_>>());
        }
    }
}

/// Runs say.mjs with `args`; its output (a message id for "post").
fn say(args: &[&str]) -> String {
    let out = std::process::Command::new("node")
        .arg(parity_dir().join("say.mjs"))
        .args(args)
        .output()
        .expect("run say.mjs");
    assert!(
        out.status.success(),
        "say.mjs {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// "Control+Shift+k" as egui modifiers and key.
fn parse_key(spec: &str) -> (egui::Modifiers, egui::Key) {
    let mut modifiers = egui::Modifiers::NONE;
    let mut parts: Vec<&str> = spec.split('+').collect();
    // "Control++" would name the plus key; none of the scenes needs it.
    let name = parts.pop().expect("key name");
    for part in parts {
        match part {
            "Control" | "Meta" => modifiers |= egui::Modifiers::COMMAND,
            "Shift" => modifiers |= egui::Modifiers::SHIFT,
            "Alt" => modifiers |= egui::Modifiers::ALT,
            other => panic!("unknown modifier {other:?} in {spec:?}"),
        }
    }
    let key = if name.chars().count() == 1 {
        egui::Key::from_name(&name.to_uppercase())
    } else {
        egui::Key::from_name(name)
    };
    (
        modifiers,
        key.unwrap_or_else(|| panic!("unknown key {name:?}")),
    )
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
    // Device settings start empty in every scene, as in a fresh browser
    // context, and the user's own app config is never read or written.
    let config = std::env::temp_dir().join(format!("gelabber-parity-{}", std::process::id()));
    // SAFETY: set before the first client starts a thread.
    unsafe { std::env::set_var("GELABBER_CONFIG_DIR", &config) };
    for scene in &scenes.scenes {
        if only.as_ref().is_some_and(|o| !o.contains(&scene.name)) {
            continue;
        }
        let _ = std::fs::remove_dir_all(&config);
        // Opening a channel marks it read; every scene starts from the reference.
        let reset = std::process::Command::new("node")
            .arg(parity_dir().join("reset.mjs"))
            .status()
            .expect("run reset.mjs");
        assert!(reset.success(), "reset.mjs failed");
        if let Some(before) = &scene.before {
            say(&before.split(' ').collect::<Vec<_>>());
        }
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
        let system = match scene.scheme.as_deref() {
            Some("dark") => egui::Theme::Dark,
            _ => egui::Theme::Light,
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .with_max_steps(10_000)
            // Browser screenshots carry no pointer.
            .with_render_cursor(false)
            // Frames as in a browser, not 4 a second: timed things (toasts,
            // fades) are as far along as in the web shot.
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_eframe(|cc| {
                cc.egui_ctx.options_mut(|o| o.fallback_theme = system);
                match &scene.page {
                    Some(page) => page_app(cc, page, &scene.query),
                    None => {
                        gelabber_ui::Launcher::with_app(cc, client.clone(), Some(route.clone()))
                    }
                }
            });
        // The system theme as Chromium was told it (prefers-color-scheme).
        harness.input_mut().system_theme = Some(system);
        // Step until the app has nothing in flight for a while: data,
        // images and layout settle like the web page after networkidle.
        settle(&mut harness);
        // shots.mjs hides the caret (caret-color: transparent); so here.
        harness.ctx.all_styles_mut(|s| {
            s.visuals.text_cursor.stroke.color = egui::Color32::TRANSPARENT;
        });
        let mut undo = Undo {
            said: Vec::new(),
            after: scene.after.clone(),
        };
        let mut hovered = false;
        for action in &scene.actions {
            match action {
                Action::Say(spec) => {
                    let mut parts = spec.splitn(3, ' ');
                    let (who, at, text) = (
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default(),
                    );
                    undo.said
                        .push((who.to_owned(), say(&["post", who, at, text])));
                }
                Action::Click(name) => {
                    use egui::accesskit::Role;
                    reveal(&mut harness, size.y, name, &[Role::Button]);
                    find(&harness, name, &[Role::Button])
                        .unwrap_or_else(|| panic!("{}: no button {name:?}", scene.name))
                        .click();
                }
                Action::Link(name) => {
                    use egui::accesskit::Role;
                    reveal(&mut harness, size.y, name, &[Role::Link]);
                    find(&harness, name, &[Role::Link])
                        .unwrap_or_else(|| panic!("{}: no link {name:?}", scene.name))
                        .click();
                }
                Action::Rightclick(name) => {
                    use egui::accesskit::Role;
                    reveal(&mut harness, size.y, name, &[Role::Button, Role::Link]);
                    find(&harness, name, &[Role::Button, Role::Link])
                        .unwrap_or_else(|| panic!("{}: no button or link {name:?}", scene.name))
                        .click_secondary();
                }
                Action::Clicktext(label) => {
                    reveal(&mut harness, size.y, label, &[]);
                    find(&harness, label, &[])
                        .unwrap_or_else(|| panic!("{}: nothing labelled {label:?}", scene.name))
                        .click();
                }
                Action::Hover(label) => {
                    use egui::accesskit::Role;
                    reveal(&mut harness, size.y, label, &[Role::Button, Role::Link]);
                    find(&harness, label, &[Role::Button, Role::Link])
                        .or_else(|| find(&harness, label, &[]))
                        .unwrap_or_else(|| panic!("{}: nothing labelled {label:?}", scene.name))
                        .hover();
                    hovered = true;
                }
                Action::Key(spec) => {
                    let (modifiers, key) = parse_key(spec);
                    harness.key_press_modifiers(modifiers, key);
                }
                Action::Type(text) => {
                    harness.event(egui::Event::Text(text.clone()));
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
        // The pointer rests in the bottom-right corner, as shots.mjs leaves
        // it, unless a hover put it somewhere.
        if !hovered {
            harness.hover_at(egui::pos2(size.x - 1.0, size.y - 1.0));
        }
        // Long enough for the hover fades to end.
        harness.run_steps(15);
        let image = harness
            .render()
            .unwrap_or_else(|e| panic!("render {}: {e}", scene.name));
        let path = out.join(format!("{}.png", scene.name));
        image.save(&path).expect("save png");
        drop(undo);
        println!("{}: {route} -> {}", scene.name, path.display());
    }
    let _ = std::fs::remove_dir_all(&config);
}

type App<'a> = egui_kittest::Harness<'a, gelabber_ui::Launcher>;

/// The Launcher on a bundled page: the setup page with its `?server=` (the
/// way back) and `?error=` (why it shows).
fn page_app(cc: &eframe::CreationContext<'_>, page: &str, query: &str) -> gelabber_ui::Launcher {
    assert_eq!(
        page, "desktop/app/setup/index.html",
        "no native page for {page}"
    );
    let query = gelabber_ui::Location::parse(&format!("/{query}"));
    gelabber_ui::Launcher::setup(
        cc,
        query.query("server").map(str::to_owned),
        query.query("error").map(str::to_owned),
        Box::new(|_| {}),
    )
}

/// The first node labelled `label` with one of `roles` (any role if empty).
fn find<'t>(
    harness: &'t App<'_>,
    label: &'t str,
    roles: &[egui::accesskit::Role],
) -> Option<egui_kittest::Node<'t>> {
    use egui_kittest::kittest::{NodeT, Queryable};
    harness
        .query_all_by_label(label)
        .find(|n| roles.is_empty() || roles.contains(&n.accesskit_node().role()))
}

/// Playwright scrolls a target out of view to the middle before it
/// clicks; here in 100px scroll steps until it is there or cannot move.
fn reveal(harness: &mut App<'_>, height: f32, label: &str, roles: &[egui::accesskit::Role]) {
    let middle = height / 2.0;
    let mut scrolling = false;
    for _ in 0..60 {
        let Some(node) = find(harness, label, roles) else {
            return;
        };
        let rect = node.rect();
        let visible = rect.top() >= 0.0 && rect.bottom() <= height;
        if (!scrolling && visible) || (scrolling && (rect.center().y - middle).abs() <= 50.0) {
            return;
        }
        scrolling = true;
        if rect.center().y > middle {
            node.scroll_down();
        } else {
            node.scroll_up();
        }
        harness.run_steps(10);
        let moved = find(harness, label, roles).map(|n| n.rect().center().y);
        if moved == Some(rect.center().y) {
            return;
        }
    }
}

/// Steps until the app has had nothing in flight for ten frames.
fn settle(harness: &mut App<'_>) {
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
