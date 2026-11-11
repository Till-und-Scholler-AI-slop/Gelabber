//! Native parity check: renders every scene of web/scripts/parity/scenes.json
//! with the real app against the seeded stack (headless, egui_kittest on
//! wgpu) and compares it with the web screenshot of the same scene (SSIM).
//! A scene passes when SSIM, pixel match and edge match all reach 0.90
//! (see pixels.rs for the two stricter measures). A render fails a scene
//! whose controls (buttons, links, fields, …) include one without a name.
//!
//!   gelabber-parity render <out-dir> [scene,…]
//!   gelabber-parity compare <web-dir> <native-dir> <report-dir> [--only a,b] [--crop x,y,w,h]
//!
//! `--crop` scores one region (e.g. a page column) while the rest of the
//! window is not built yet; acceptance is always on the full scene.
//!
//! Reads web/scripts/parity/{scenes,fixture}.json from the repository.
//! A scene's actions (click, link, menuitem, rightclick, clicktext, hover,
//! focus, key, type, say, wait, back, placeholder, call) mean the same in
//! shots.mjs; a name written `/…/` is a regular expression. `before` runs
//! say.mjs words ahead of the page, posts and `after` are undone with say.mjs
//! once the screenshot is taken. A scene with `page` shows the Tauri app's
//! setup page (kept in desktop/parity/reference/setup) instead of a route:
//! the Launcher here. A scene with `update` shows the update notice over the
//! page: the Tauri app's BANNER (reference/update-banner.js) there,
//! `platform::update::Notice` here. A scene with `locale: "en"` shows the
//! English interface: a signed-in scene sets the account's language
//! (`PUT /api/me/locale`) before the page loads and puts German back
//! afterwards, also when the scene fails; a signed-out one starts the app in
//! English (`gelabber_i18n::set`), as the web shot uses an English browser.

mod pixels;
mod ssim;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gelabber_ui::platform::update::{Click, Notice, Offer, Phase};
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
    /// A page of the desktop app, e.g. "desktop/parity/reference/setup/index.html".
    page: Option<String>,
    /// The page's query, e.g. "?server=…&error=…".
    #[serde(default)]
    query: String,
    /// The system's colour scheme, "light" (Playwright's default) or "dark".
    scheme: Option<String>,
    user: Option<String>,
    /// The web shot runs as the desktop app (shots.mjs); this one always is it.
    #[serde(default, rename = "desktop")]
    _desktop: bool,
    /// `channel people` for call.mjs: people kept in a voice channel.
    call: Option<String>,
    /// Steps after the page settled, the same in both shot tools.
    #[serde(default)]
    actions: Vec<Action>,
    /// say.mjs words run before the page loads (a theme); reset.mjs undoes them.
    before: Option<String>,
    /// say.mjs words run after the screenshot (undo what dialogs made).
    after: Option<String>,
    /// `[x, y, w, h]` areas left out of the comparison: picture content that
    /// comes from the fake devices (Chromium's versus the core's), not the app.
    #[serde(default)]
    mask: Vec<[u32; 4]>,
    /// The update the app offers, in the shape of the Tauri app's
    /// `update_state` (shots.mjs answers it with this).
    update: Option<UpdateState>,
    /// The setup page's update section, as the Tauri app's `update_state`
    /// answers that page where the app updates itself.
    updates: Option<SetupState>,
    /// `"en"`: the scene in English (see the module's comment).
    locale: Option<String>,
    /// Only the web has this page (the operator area): shots.mjs takes it
    /// as a reference, render and compare pass it by.
    #[serde(default, rename = "webOnly")]
    web_only: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupState {
    current: String,
    auto_check: bool,
    available: Option<String>,
    last_check: Option<u64>,
    /// "downloading" (`downloaded` of `total` bytes) or "installing": an
    /// install at work; otherwise the update is offered.
    #[serde(default)]
    phase: String,
    #[serde(default)]
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Deserialize)]
struct UpdateState {
    available: String,
    /// Where a click on "Neu starten" gets to: "downloading" with
    /// `downloaded` of `total` bytes, or "installing".
    #[serde(default)]
    phase: String,
    #[serde(default)]
    downloaded: u64,
    total: Option<u64>,
}

/// The Launcher with the update notice over it, as main.rs paints them.
struct Shell {
    launcher: gelabber_ui::Launcher,
    notice: Option<(Notice, Phase)>,
    /// What the Updater would publish for the setup screen.
    setup: Option<gelabber_ui::platform::update::SetupView>,
}

impl Shell {
    fn new(launcher: gelabber_ui::Launcher, update: Option<&UpdateState>) -> Self {
        // The Tauri app's notice is the Windows setup's.
        let notice = update.map(|u| {
            let after = match u.phase.as_str() {
                "installing" => Phase::Installing,
                _ => Phase::Downloading {
                    downloaded: u.downloaded,
                    total: u.total,
                },
            };
            (Notice::new(&u.available, Offer::Restart), after)
        });
        Self {
            launcher,
            notice,
            setup: None,
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        gelabber_ui::platform::update::show_setup(ui.ctx(), self.setup.clone());
        eframe::App::ui(&mut self.launcher, ui, frame);
        if let Some((notice, after)) = &mut self.notice {
            match notice.paint(ui.ctx()) {
                Some(Click::Restart) => notice.phase = after.clone(),
                Some(Click::Later) => self.notice = None,
                _ => {}
            }
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.launcher, visuals)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    /// Click the button with this accessible name.
    Click(String),
    /// Click the link with this accessible name.
    Link(String),
    /// Click the menu entry with this accessible name.
    Menuitem(String),
    /// Right-click the first button or link with this accessible name.
    Rightclick(String),
    /// Click the first element with this label, whatever its role.
    Clicktext(String),
    /// Rest the pointer on the first button or link, else element, with this label;
    /// it stays there for the screenshot.
    Hover(String),
    /// Select the whole text of the first text with exactly this label,
    /// with a drag across it.
    Select(String),
    /// Focus the text field with this accessible name.
    Focus(String),
    /// Press a key, Playwright style: "Control+k", "ArrowDown", "Escape".
    Key(String),
    /// Type text into whatever has the focus.
    Type(String),
    /// "<user> <channel-key> <text>": someone else writes (say.mjs); the
    /// message is deleted after the screenshot.
    Say(String),
    /// Wait this many milliseconds.
    Wait(u64),
    /// `true`: go back, as the browser does on Alt+←.
    Back(bool),
    /// Click the text field with this placeholder (natively its name).
    Placeholder(String),
    /// `channel people` for call.mjs, started now and kept until the
    /// screenshot is taken (a Live that begins while the page is open).
    Call(String),
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

/// Puts the fixture back to the reference state (reset.mjs).
fn reset() {
    let reset = std::process::Command::new("node")
        .arg(parity_dir().join("reset.mjs"))
        .status()
        .expect("run reset.mjs");
    assert!(reset.success(), "reset.mjs failed");
}

/// Runs reset.mjs when dropped: undoes a scene's `before`.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        reset();
    }
}

/// Puts the scene user's language back to German when dropped.
struct GermanAgain(gelabber_client::Client);

impl Drop for GermanAgain {
    fn drop(&mut self) {
        if let Err(e) = self.0.set_locale_blocking("de") {
            eprintln!("putting the language back to German: {e}");
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
    // The seeded invite's code, asked for when a scene needs it.
    if out.contains("{invite}") {
        out = out.replace("{invite}", &say(&["invite-code", "lena", "home"]));
    }
    out
}

/// Names in `only` that no scene has: a typo would otherwise run nothing
/// and pass.
fn unknown_scenes(scenes: &[Scene], only: Option<&[String]>) -> Vec<String> {
    only.unwrap_or_default()
        .iter()
        .filter(|name| !scenes.iter().any(|s| &s.name == *name))
        .cloned()
        .collect()
}

/// Whether render and compare take the scene: picked by `only` (or no
/// `only`), and not a page only the web has.
fn runs_natively(scene: &Scene, only: Option<&[String]>) -> bool {
    !scene.web_only && only.is_none_or(|o| o.contains(&scene.name))
}

fn check_only(scenes: &[Scene], only: Option<&[String]>) {
    let unknown = unknown_scenes(scenes, only);
    if !unknown.is_empty() {
        eprintln!("no such scene: {}", unknown.join(","));
        std::process::exit(2);
    }
}

/// A mask in window coordinates, moved into an image cropped at
/// `(x, y)` and cut to its `w × h`; `None` when it lies outside.
fn crop_mask(mask: [u32; 4], crop: [u32; 4]) -> Option<[u32; 4]> {
    let [mx, my, mw, mh] = mask;
    let [cx, cy, cw, ch] = crop;
    let left = mx.max(cx);
    let top = my.max(cy);
    let right = (mx + mw).min(cx + cw);
    let bottom = (my + mh).min(cy + ch);
    (left < right && top < bottom).then(|| [left - cx, top - cy, right - left, bottom - top])
}

fn render(out: &Path, only: Option<Vec<String>>) {
    let scenes: Scenes = read("scenes.json");
    check_only(&scenes.scenes, only.as_deref());
    let fixture: Fixture = read("fixture.json");
    std::fs::create_dir_all(out).expect("create output directory");
    // Device settings start empty in every scene, as in a fresh browser
    // context, and the user's own app config is never read or written.
    let config = std::env::temp_dir().join(format!("gelabber-parity-{}", std::process::id()));
    // SAFETY: set before the first client starts a thread.
    unsafe { std::env::set_var("GELABBER_CONFIG_DIR", &config) };
    let mut failed = Vec::new();
    for scene in &scenes.scenes {
        if !runs_natively(scene, only.as_deref()) {
            continue;
        }
        // A scene that fails is reported, and the others still render.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = std::fs::remove_dir_all(&config);
            // Opening a channel marks it read; every scene starts from the reference.
            reset();
            // `before` is undone once the scene ends, also when it fails, so the
            // last scene of a run does not leave its theme behind.
            let _restore = scene.before.as_ref().map(|before| {
                let restore = Restore;
                say(&before.split(' ').collect::<Vec<_>>());
                restore
            });
            let _call = scene.call.as_deref().map(Call::start);
            let client = gelabber_client::Client::new(&fixture.base).expect("client");
            // German unless the scene asks; the app follows a signed-in
            // account's language by itself.
            let locale = scene
                .locale
                .as_deref()
                .map(|l| gelabber_i18n::Locale::parse(l).expect("locale is de or en"))
                .unwrap_or_default();
            gelabber_i18n::set(locale);
            let mut _german_again = None;
            if let Some(user) = &scene.user {
                let email = &fixture.users[user].email;
                client
                    .login_blocking(email, &fixture.password)
                    .unwrap_or_else(|e| panic!("login {user}: {e}"));
                if let Some(locale) = &scene.locale {
                    client
                        .set_locale_blocking(locale)
                        .unwrap_or_else(|e| panic!("locale {user}: {e}"));
                    _german_again = Some(GermanAgain(client.clone()));
                }
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
                    let launcher = match &scene.page {
                        Some(page) => page_app(cc, page, &scene.query),
                        None => {
                            gelabber_ui::Launcher::with_app(cc, client.clone(), Some(route.clone()))
                        }
                    };
                    let mut shell = Shell::new(launcher, scene.update.as_ref());
                    shell.setup =
                        scene
                            .updates
                            .as_ref()
                            .map(|u| gelabber_ui::platform::update::SetupView {
                                current: u.current.clone(),
                                auto_check: u.auto_check,
                                checking: false,
                                available: u.available.clone(),
                                checked: u.last_check.is_some(),
                                error: None,
                                // The Windows setup's, as the Tauri app's.
                                installs: true,
                                notice: u.available.as_deref().map(|v| Notice {
                                    phase: match u.phase.as_str() {
                                        "downloading" => Phase::Downloading {
                                            downloaded: u.downloaded,
                                            total: u.total,
                                        },
                                        "installing" => Phase::Installing,
                                        _ => Phase::Offer,
                                    },
                                    ..Notice::new(v, Offer::Restart)
                                }),
                            });
                    shell
                });
            // The system theme as Chromium was told it (prefers-color-scheme).
            harness.input_mut().system_theme = Some(system);
            // Step until the app has nothing in flight for a while: data,
            // images and layout settle like the web page after networkidle.
            settle(&mut harness);
            // shots.mjs hides the caret (caret-color: transparent); so here.
            // A browser screenshot never shows a `title` tooltip, which egui's
            // hover texts stand for (the custom tooltips still show).
            harness.ctx.all_styles_mut(|s| {
                s.visuals.text_cursor.stroke.color = egui::Color32::TRANSPARENT;
                s.interaction.tooltip_delay = f32::MAX;
            });
            let mut undo = Undo {
                said: Vec::new(),
                after: scene.after.clone(),
            };
            let mut hovered = false;
            let mut calls = Vec::new();
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
                    Action::Menuitem(name) => {
                        use egui::accesskit::Role;
                        reveal(&mut harness, size.y, name, &[Role::MenuItem]);
                        find(&harness, name, &[Role::MenuItem])
                            .unwrap_or_else(|| panic!("{}: no menu entry {name:?}", scene.name))
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
                        if std::env::var_os("PARITY_DEBUG").is_some() {
                            use egui_kittest::kittest::{NodeT, Queryable};
                            for n in harness.query_all_by_label(label) {
                                eprintln!(
                                    "{label}: {:?} {:?}",
                                    n.accesskit_node().role(),
                                    n.rect()
                                );
                            }
                        }
                        // getByText skips aria-labels, such as a rail icon's: links
                        // only when nothing else carries the text.
                        find_text(&harness, label)
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
                    Action::Select(label) => {
                        use egui::accesskit::Role;
                        reveal(&mut harness, size.y, label, &[Role::Label]);
                        let rect = find(&harness, label, &[Role::Label])
                            .unwrap_or_else(|| panic!("{}: no text {label:?}", scene.name))
                            .rect();
                        // From before the first character to after the last.
                        let from = rect.left_top() + egui::vec2(1.0, 4.0);
                        let to = rect.right_bottom() - egui::vec2(1.0, 4.0);
                        harness.hover_at(from);
                        harness.drag_at(from);
                        harness.hover_at(rect.center());
                        harness.hover_at(to);
                        harness.event(egui::Event::PointerButton {
                            pos: to,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                    Action::Focus(label) => {
                        use egui::accesskit::Role;
                        let roles = [
                            Role::TextInput,
                            Role::MultilineTextInput,
                            Role::PasswordInput,
                            Role::SearchInput,
                        ];
                        find(&harness, label, &roles)
                            .unwrap_or_else(|| panic!("{}: no text field {label:?}", scene.name))
                            .focus();
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
                    Action::Call(spec) => calls.push(Call::start(spec)),
                    Action::Placeholder(label) => {
                        use egui::accesskit::Role;
                        let fields = [Role::TextInput, Role::SearchInput, Role::MultilineTextInput];
                        find(&harness, label, &fields)
                            .unwrap_or_else(|| panic!("{}: no field {label:?}", scene.name))
                            .click();
                    }
                    Action::Back(back) => {
                        if *back {
                            harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
                        }
                    }
                }
                settle(&mut harness);
            }
            // The pointer rests in the bottom-right corner, as shots.mjs leaves
            // it, unless a scene put it on something.
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
            // M5 of docs/v0.9-plan.md: every control a screen reader reaches
            // has a name.
            let (controls, nameless) = unnamed(&harness);
            assert!(
                nameless.is_empty(),
                "{}: controls without a name: {}",
                scene.name,
                nameless.join(", ")
            );
            println!("{}: {controls} controls, all named", scene.name);
            drop(undo);
            drop(calls);
            println!("{}: {route} -> {}", scene.name, path.display());
        }));
        if outcome.is_err() {
            failed.push(scene.name.clone());
        }
    }
    let _ = std::fs::remove_dir_all(&config);
    if !failed.is_empty() {
        eprintln!("failed scenes: {}", failed.join(","));
        std::process::exit(1);
    }
}

type App<'a> = egui_kittest::Harness<'a, Shell>;

/// The controls a screen reader reaches, and those without a name (role
/// and place).
fn unnamed(harness: &App<'_>) -> (usize, Vec<String>) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    const CONTROLS: &[Role] = &[
        Role::Button,
        Role::CheckBox,
        Role::RadioButton,
        Role::Switch,
        Role::Link,
        Role::MenuItem,
        Role::MenuItemCheckBox,
        Role::MenuItemRadio,
        Role::Tab,
        Role::TextInput,
        Role::MultilineTextInput,
        Role::SearchInput,
        Role::Slider,
        Role::SpinButton,
        Role::ComboBox,
        Role::DisclosureTriangle,
        Role::ListBoxOption,
        Role::TreeItem,
    ];
    let controls = harness
        .query_all_by(|n| CONTROLS.contains(&n.role()) && !n.is_hidden())
        .count();
    let nameless = harness
        .query_all_by(|n| {
            CONTROLS.contains(&n.role())
                && !n.is_hidden()
                && n.label().is_none_or(|l| l.trim().is_empty())
        })
        .map(|n| {
            let node = n.accesskit_node();
            let at = node
                .bounding_box()
                .map(|b| format!("{:.0},{:.0}", b.x0, b.y0))
                .unwrap_or_default();
            format!("{:?}@{at}", node.role())
        })
        .collect();
    (controls, nameless)
}

/// The Launcher on a bundled page: the setup page with its `?server=` (the
/// way back), `?error=` (why it shows) and `?detail=` (what the system said).
fn page_app(cc: &eframe::CreationContext<'_>, page: &str, query: &str) -> gelabber_ui::Launcher {
    assert_eq!(
        page, "desktop/parity/reference/setup/index.html",
        "no native page for {page}"
    );
    let query = gelabber_ui::Location::parse(&format!("/{query}"));
    gelabber_ui::Launcher::setup(
        cc,
        query.query("server").map(str::to_owned),
        query.query("error").map(|message| {
            gelabber_ui::setup::Failure::new(message, query.query("detail").map(str::to_owned))
        }),
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
    if let Some(pattern) = label.strip_prefix('/').and_then(|l| l.strip_suffix('/')) {
        let re = regex::Regex::new(pattern).unwrap_or_else(|e| panic!("{label}: {e}"));
        return harness
            .query_all_by(move |n| n.label().is_some_and(|l| re.is_match(&l)))
            .find(|n| roles.is_empty() || roles.contains(&n.accesskit_node().role()));
    }
    harness
        .query_all_by_label(label)
        .find(|n| roles.is_empty() || roles.contains(&n.accesskit_node().role()))
}

/// The first node with this label that is not a link, else the first link.
fn find_text<'t>(harness: &'t App<'_>, label: &'t str) -> Option<egui_kittest::Node<'t>> {
    use egui::accesskit::Role;
    let others = [
        Role::Button,
        Role::DisclosureTriangle,
        Role::MenuItem,
        Role::Tab,
        Role::RadioButton,
        Role::CheckBox,
        Role::Switch,
        Role::Label,
        Role::GenericContainer,
        Role::Unknown,
    ];
    find(harness, label, &others).or_else(|| find(harness, label, &[]))
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

/// Signed in on a server's page whose presence has not arrived yet.
fn presence_pending(harness: &mut App<'_>) -> bool {
    use gelabber_ui::route::Route;
    let Some(app) = harness.state_mut().launcher.app() else {
        return false;
    };
    let server = match app.nav().route() {
        Route::Server(s) | Route::Channel(s, _) | Route::ServerSettings(s) => *s,
        _ => return false,
    };
    app.client()
        .with_state(|s| s.me().is_some() && !s.presence.contains_key(&server))
}

/// Steps until the app has had nothing in flight for ten frames.
fn settle(harness: &mut App<'_>) {
    let start = Instant::now();
    let mut quiet = 0;
    while quiet < 10 && start.elapsed() < Duration::from_secs(20) {
        harness.step();
        // The gateway's presence snapshot is no request the client counts;
        // without it the member list shows everyone offline. A server that
        // never sends one (not a member) waits 5 s at most.
        let presence = start.elapsed() < Duration::from_secs(5) && presence_pending(harness);
        if harness.state().launcher.busy() || presence {
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
    check_only(&scenes.scenes, only);
    std::fs::create_dir_all(report).expect("create report directory");
    let mut all = true;
    let mut compared = 0;
    let mut lines = vec![
        "| scene | SSIM | pixels | edges | result |".to_owned(),
        "|---|---|---|---|---|".to_owned(),
    ];
    for scene in &scenes.scenes {
        if !runs_natively(scene, only) {
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
        compared += 1;
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
        // Masks are in window coordinates; a crop moves the image's origin.
        let masks = scene
            .mask
            .iter()
            .filter_map(|&m| match crop {
                Some(c) => crop_mask(m, c),
                None => Some(m),
            })
            .collect::<Vec<_>>();
        for &[x, y, w, h] in &masks {
            for img in [&mut a, &mut b] {
                for py in y..(y + h).min(img.height()) {
                    for px in x..(x + w).min(img.width()) {
                        img.put_pixel(px, py, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
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
    if compared == 0 {
        eprintln!("no scene was compared");
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mask_follows_the_crop() {
        let crop = [100, 50, 200, 100];
        // Inside: shifted by the crop's origin.
        assert_eq!(crop_mask([120, 60, 10, 10], crop), Some([20, 10, 10, 10]));
        // Across the crop's edge: cut to the part inside.
        assert_eq!(crop_mask([90, 40, 20, 20], crop), Some([0, 0, 10, 10]));
        assert_eq!(crop_mask([290, 140, 50, 50], crop), Some([190, 90, 10, 10]));
        // Outside: nothing to paint.
        assert_eq!(crop_mask([0, 0, 50, 50], crop), None);
    }

    #[test]
    fn an_unknown_scene_name_is_named() {
        let scenes: Vec<Scene> = serde_json::from_value(serde_json::json!([
            {"name": "login"}, {"name": "dm"}
        ]))
        .unwrap();
        let only = ["dm".to_owned(), "lgoin".to_owned()];
        assert_eq!(
            unknown_scenes(&scenes, Some(&only)),
            vec!["lgoin".to_owned()]
        );
        assert!(unknown_scenes(&scenes, None).is_empty());
    }

    /// The web shot's `userAgent` (the Security scenes) is the browser's
    /// business; the app sends its own, so the scene reads without it.
    #[test]
    fn a_scene_with_a_user_agent_reads() {
        let scenes: Vec<Scene> = serde_json::from_value(serde_json::json!([
            {"name": "settings-konto", "path": "/settings/konto", "user": "lena",
             "userAgent": "Gelabber-Desktop/0.10.0 (linux)"}
        ]))
        .unwrap();
        assert_eq!(scenes[0].name, "settings-konto");
    }

    /// The operator pages are web-only: the flag reads, and render and
    /// compare pass those scenes by, also when `only` names them.
    #[test]
    fn a_web_only_scene_is_skipped() {
        let scenes: Vec<Scene> = serde_json::from_value(serde_json::json!([
            {"name": "register-invitation", "path": "/register?invitation=k7m2p9qrs4tx", "user": null},
            {"name": "operator-server", "path": "/betrieb/server", "user": "lena", "webOnly": true}
        ]))
        .unwrap();
        assert!(!scenes[0].web_only);
        assert!(scenes[1].web_only);
        assert!(runs_natively(&scenes[0], None));
        assert!(!runs_natively(&scenes[1], None));
        let only = ["operator-server".to_owned()];
        assert!(!runs_natively(&scenes[1], Some(&only)));
        assert!(!runs_natively(&scenes[0], Some(&only)));
        assert!(unknown_scenes(&scenes, Some(&only)).is_empty());
    }
}
