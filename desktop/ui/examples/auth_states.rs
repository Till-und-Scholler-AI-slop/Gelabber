//! Renders the sign-in forms in their error states against the seeded
//! parity stack, to compare with the web by eye (the parity scenes only
//! cover the pristine pages):
//!
//!   cargo run --release -p gelabber-ui --example auth_states -- <out-dir>
//!
//! States: login-empty (submit with nothing typed), login-wrong (a wrong
//! password), register-invalid (bad address, short password).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gelabber_ui::GelabberApp;

fn base() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/scripts/parity/fixture.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&text).expect("fixture");
    fixture["base"].as_str().expect("base").to_owned()
}

fn settle(harness: &mut Harness<'_, GelabberApp>) {
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
}

fn open<'a>(route: &str) -> Harness<'a, GelabberApp> {
    let client = gelabber_client::Client::new(&base()).expect("client");
    let route = route.to_owned();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_max_steps(10_000)
        .wgpu()
        .build_eframe(move |cc| GelabberApp::new(cc, client.clone(), Some(route.clone())));
    settle(&mut harness);
    harness
}

fn type_into(harness: &mut Harness<'_, GelabberApp>, label: &str, text: &str) {
    harness.get_by_label(label).focus();
    harness.step();
    harness.get_by_label(label).type_text(text);
    harness.step();
}

fn shot(harness: &mut Harness<'_, GelabberApp>, out: &Path, name: &str) {
    // Like the web shots: the pointer rests in the corner.
    harness.hover_at(egui::pos2(1279.0, 799.0));
    settle(harness);
    let image = harness.render().expect("render");
    let path = out.join(format!("{name}.png"));
    image.save(&path).expect("save");
    println!("{name}: {}", path.display());
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: auth_states <out-dir>"),
    );
    std::fs::create_dir_all(&out).expect("out dir");

    let mut login = open("/login");
    login.get_by_label("Anmelden").click();
    settle(&mut login);
    shot(&mut login, &out, "login-empty");

    let mut login = open("/login");
    type_into(&mut login, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut login, "Passwort", "falsch12");
    login.get_by_label("Anmelden").click();
    settle(&mut login);
    shot(&mut login, &out, "login-wrong");

    let mut register = open("/register");
    type_into(&mut register, "Name", "Lena");
    type_into(&mut register, "E-Mail-Adresse", "lena@x");
    type_into(&mut register, "Passwort", "kurz");
    register.get_by_label("Registrieren").click();
    settle(&mut register);
    shot(&mut register, &out, "register-invalid");

    // The other front-door pages as they open.
    for (name, route) in [
        ("forgot-password", "/forgot-password"),
        ("reset-dead", "/reset-password"),
        ("verify-dead", "/verify-email"),
        ("not-found", "/gibt-es-nicht"),
        ("invite-signed-out", "/invite/abcdefghjkmn"),
    ] {
        let mut page = open(route);
        shot(&mut page, &out, name);
        println!("  at {}", page.state().nav().location().href());
    }

    // A successful sign-in lands in the workspace.
    let mut login = open("/login?redirect=/d");
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/scripts/parity/fixture.json"),
    )
    .expect("fixture");
    let fixture: serde_json::Value = serde_json::from_str(&fixture).expect("fixture");
    let email = fixture["users"]["lena"]["email"].as_str().expect("email");
    let password = fixture["password"].as_str().expect("password");
    type_into(&mut login, "E-Mail-Adresse", email);
    type_into(&mut login, "Passwort", password);
    login.key_press(egui::Key::Enter);
    settle(&mut login);
    println!("login-ok: at {}", login.state().nav().location().href());
}
