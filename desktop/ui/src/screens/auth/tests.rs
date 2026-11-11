//! The signed-out forms against the stand-in API (app/testkit.rs): rate
//! limits (useFormErrors, useCooldown) and the caps-lock hint
//! (PasswordField).

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::testkit::{FakeApi, Reply, launcher, wait_for, wait_for_label, wait_for_text};
use crate::platform::paths::testing::TempConfig;

/// A 429 with `Retry-After: 30` for `path`.
fn limited(path: &'static str) -> FakeApi {
    FakeApi::start(false, move |method, at, _| {
        (method == "POST" && at == path)
            .then(|| Reply::error(429, "rate_limited").header("Retry-After", "30"))
    })
}

/// The text or password input labelled `label`.
pub(crate) fn input<'t>(
    harness: &'t egui_kittest::Harness<'_, crate::Launcher>,
    label: &'t str,
) -> egui_kittest::Node<'t> {
    harness
        .query_all_by_label(label)
        .find(|n| {
            matches!(
                n.accesskit_node().role(),
                Role::TextInput | Role::PasswordInput
            )
        })
        .unwrap_or_else(|| panic!("no input {label:?}"))
}

pub(crate) fn type_into(
    harness: &mut egui_kittest::Harness<'_, crate::Launcher>,
    label: &str,
    text: &str,
) {
    input(harness, label).focus();
    harness.step();
    input(harness, label).type_text(text);
    harness.step();
}

/// The message, and the submit button waiting the seconds out.
fn assert_cooldown(harness: &mut egui_kittest::Harness<'_, crate::Launcher>) {
    wait_for_text(harness, "Zu viele Versuche. Bitte 30 Sekunden warten.");
    let button = harness.get_by_label_contains("Erneut versuchen in");
    assert_eq!(button.accesskit_node().role(), Role::Button);
    assert!(button.accesskit_node().is_disabled());
    let label = button.accesskit_node().label().unwrap_or_default();
    assert!(
        label == "Erneut versuchen in 30 s" || label == "Erneut versuchen in 29 s",
        "{label}"
    );
}

#[test]
fn login_waits_out_a_rate_limit() {
    let _config = TempConfig::new("auth-login-limit");
    let api = limited("/auth/login");
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Anmelden")
        .click();
    assert_cooldown(&mut harness);
    // Enter does not send while it waits.
    let sent = api.seen.lock().len();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(5);
    assert!(
        !api.seen.lock()[sent..]
            .iter()
            .any(|r| r == "POST /auth/login")
    );
}

#[test]
fn register_waits_out_a_rate_limit() {
    let _config = TempConfig::new("auth-register-limit");
    let api = limited("/auth/register");
    let mut harness = launcher(&api, "/register");
    wait_for_label(&mut harness, "Name");
    type_into(&mut harness, "Name", "Lena");
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    assert_cooldown(&mut harness);
}

#[test]
fn forgot_password_waits_out_a_rate_limit() {
    let _config = TempConfig::new("auth-forgot-limit");
    let api = limited("/auth/password-reset");
    let mut harness = launcher(&api, "/forgot-password");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    harness
        .get_by_role_and_label(Role::Button, "Link schicken")
        .click();
    assert_cooldown(&mut harness);
}

/// Without a wait the generic text, and the button stays.
#[test]
fn a_rate_limit_without_a_wait_says_so_generically() {
    let _config = TempConfig::new("auth-login-generic");
    let api = FakeApi::start(false, |method, at, _| {
        (method == "POST" && at == "/auth/login").then(|| Reply::error(429, "rate_limited"))
    });
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Anmelden")
        .click();
    wait_for_text(
        &mut harness,
        "Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.",
    );
    assert!(
        harness
            .query_by_label_contains("Erneut versuchen in")
            .is_none()
    );
}

/// PasswordField: the Caps Lock warning while the field has focus.
#[test]
fn caps_lock_shows_under_the_password() {
    let _config = TempConfig::new("auth-caps");
    let api = FakeApi::start(false, |_, _, _| None);
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Passwort");
    input(&harness, "Passwort").focus();
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_none());
    harness.key_press(egui::Key::CapsLock);
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_some());
    // Typing with it on keeps it; a lowercase letter says it went off.
    input(&harness, "Passwort").type_text("ABC");
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_some());
    input(&harness, "Passwort").type_text("d");
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_none());
    // Uppercase without Shift turns it on; leaving the field hides it.
    input(&harness, "Passwort").type_text("E");
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_some());
    input(&harness, "E-Mail-Adresse").focus();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_none());
    // Not on the e-mail field.
    harness.key_press(egui::Key::CapsLock);
    harness.step();
    assert!(harness.query_by_label("Feststelltaste ist an").is_none());
}

/// A mailed link's 64 hex digits.
const LINK: &str = "#token=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn reset_password_waits_out_a_rate_limit() {
    let _config = TempConfig::new("auth-reset-limit");
    let api = limited("/auth/password-reset/confirm");
    let mut harness = launcher(&api, &format!("/reset-password{LINK}"));
    wait_for_label(&mut harness, "Neues Passwort");
    type_into(&mut harness, "Neues Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Passwort speichern")
        .click();
    assert_cooldown(&mut harness);
}

#[test]
fn verify_email_waits_out_a_rate_limit() {
    let _config = TempConfig::new("auth-verify-limit");
    let api = limited("/auth/verify-email");
    let mut harness = launcher(&api, &format!("/verify-email{LINK}"));
    wait_for_label(&mut harness, "Passwort");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Bestätigen")
        .click();
    assert_cooldown(&mut harness);
}

/// A failed `/auth/mail` at launch (server restarting, offline) is not
/// "no mail": it is asked again, and "Passwort vergessen?" comes.
#[test]
fn a_failed_mail_check_is_retried() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let _config = TempConfig::new("auth-mail-retry");
    let down = Arc::new(AtomicBool::new(true));
    let flag = down.clone();
    let api = FakeApi::start(false, move |method, at, _| {
        (method == "GET" && at == "/auth/mail" && flag.swap(false, Ordering::SeqCst))
            .then(|| Reply::error(503, "unavailable"))
    });
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    wait_for_label(&mut harness, "Passwort vergessen?");
    assert!(!down.load(Ordering::SeqCst));
}

/// The same form in English: labels, the button and the form's errors
/// come from the catalogs.
#[test]
fn login_speaks_english() {
    let _locale = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
    let _config = TempConfig::new("auth-login-english");
    let api = limited("/auth/login");
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Email address");
    assert!(harness.query_all_by_label("Register").next().is_some());
    type_into(&mut harness, "Email address", "lena@example.test");
    type_into(&mut harness, "Password", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Sign in")
        .click();
    wait_for_text(&mut harness, "Too many attempts. Please wait 30 seconds.");
    harness.get_by_label_contains("Try again in");
}

/// An instance that published all three legal pages (issue #522).
fn published(method: &str, at: &str, _: &str) -> Option<Reply> {
    (method == "GET" && at == "/public-pages.json").then(|| {
        Reply::json(serde_json::json!({
            "version": 1,
            "landing": true,
            "legal": {
                "imprint": {"de": "/de/impressum", "en": "/en/imprint"},
                "privacy": {"de": "/de/datenschutz", "en": "/en/privacy"},
                "terms": {"de": "/de/nutzungsbedingungen", "en": "/en/terms"}
            }
        }))
    })
}

/// The links under the card, by their names, in the order they show.
fn legal_links(harness: &egui_kittest::Harness<'_, crate::Launcher>) -> Vec<String> {
    let mut links: Vec<_> = harness
        .query_all_by_role(Role::Link)
        .filter_map(|n| {
            let label = n.accesskit_node().label()?;
            let rect = n.accesskit_node().bounding_box()?;
            Some((rect.y0, rect.x0, label))
        })
        .filter(|(_, _, label)| {
            [
                "Impressum",
                "Datenschutz",
                "Nutzungsbedingungen",
                "Imprint",
                "Privacy",
                "Terms",
            ]
            .contains(&label.as_str())
        })
        .collect();
    links.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    links.into_iter().map(|(_, _, label)| label).collect()
}

/// AuthLayout `LegalLinks`: the published pages under the card, named,
/// in the reference's order, in a navigation called "Rechtliches"; a
/// click opens the page in the browser.
#[test]
fn login_links_the_published_legal_pages() {
    let _config = TempConfig::new("auth-legal");
    let opened = crate::platform::open_testing::Recording::start();
    let api = FakeApi::start(false, published);
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Nutzungsbedingungen");
    assert_eq!(
        legal_links(&harness),
        ["Impressum", "Datenschutz", "Nutzungsbedingungen"]
    );
    let nav = harness.get_by_label("Rechtliches");
    assert_eq!(nav.accesskit_node().role(), Role::Navigation);
    // Under the card: below the sign-in button.
    let below = |label: &str| {
        harness
            .get_by_role_and_label(Role::Link, label)
            .accesskit_node()
            .bounding_box()
            .expect("placed")
            .y0
    };
    let submit = harness
        .get_by_role_and_label(Role::Button, "Anmelden")
        .accesskit_node()
        .bounding_box()
        .expect("placed")
        .y1;
    assert!(below("Impressum") > submit);
    harness
        .get_by_role_and_label(Role::Link, "Datenschutz")
        .click();
    harness.run_steps(2);
    assert_eq!(opened.opened(), [format!("{}/de/datenschutz", api.origin)]);
    // The app stays where it was.
    assert!(harness.query_by_label("Passwort").is_some());
}

/// In English the English names and pages.
#[test]
fn legal_links_follow_the_language() {
    let _locale = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
    let _config = TempConfig::new("auth-legal-english");
    let opened = crate::platform::open_testing::Recording::start();
    let api = FakeApi::start(false, published);
    let mut harness = launcher(&api, "/register");
    wait_for_label(&mut harness, "Terms");
    assert_eq!(legal_links(&harness), ["Imprint", "Privacy", "Terms"]);
    assert!(harness.query_by_label("Legal").is_some());
    harness.get_by_role_and_label(Role::Link, "Terms").click();
    harness.run_steps(2);
    assert_eq!(opened.opened(), [format!("{}/en/terms", api.origin)]);
}

/// A page published in one language only is linked there in both.
#[test]
fn a_page_in_one_language_is_linked_in_the_other() {
    let _locale = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
    let _config = TempConfig::new("auth-legal-one-language");
    let opened = crate::platform::open_testing::Recording::start();
    let api = FakeApi::start(false, |method, at, _| {
        (method == "GET" && at == "/public-pages.json").then(|| {
            Reply::json(serde_json::json!({
                "version": 1, "landing": false,
                "legal": {"privacy": {"de": "/de/datenschutz"}}
            }))
        })
    });
    let mut harness = launcher(&api, "/forgot-password");
    wait_for_label(&mut harness, "Privacy");
    assert_eq!(legal_links(&harness), ["Privacy"]);
    harness.get_by_role_and_label(Role::Link, "Privacy").click();
    harness.run_steps(2);
    assert_eq!(opened.opened(), [format!("{}/de/datenschutz", api.origin)]);
}

/// No file (404): no row, as on a self-hosted instance by default.
#[test]
fn no_published_pages_show_no_row() {
    let _config = TempConfig::new("auth-legal-none");
    let api = FakeApi::start(false, |_, _, _| None);
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    crate::app::testkit::wait_for(&mut harness, "the public pages", |_| {
        api.saw("GET /public-pages.json")
    });
    harness.run_steps(10);
    assert!(legal_links(&harness).is_empty());
    assert!(harness.query_by_label("Rechtliches").is_none());
}

/// The setup screen has no instance yet: never a row, even for an address
/// whose instance publishes pages.
#[test]
fn the_setup_screen_shows_no_legal_links() {
    let _config = TempConfig::new("auth-legal-setup");
    let api = FakeApi::start(false, published);
    let origin = api.origin.clone();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_max_steps(10_000)
        .build_eframe(move |cc| {
            crate::Launcher::setup(cc, Some(origin.clone()), None, Box::new(|_| {}))
        });
    harness.run();
    assert!(harness.query_by_label("Verbinden").is_some());
    assert!(legal_links(&harness).is_empty());
    assert!(harness.query_by_label("Rechtliches").is_none());
}

/// The column centres itself with the link row and the Google button
/// counted in, as the web's `.auth-shell` with both: as much room above
/// the brand as below the links, and it stays put.
#[test]
fn the_column_is_centred_with_the_legal_links() {
    let _config = TempConfig::new("auth-legal-centred");
    let api = FakeApi::start(false, |method, at, body| {
        published(method, at, body).or_else(|| google(method, at))
    });
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Nutzungsbedingungen");
    wait_for_label(&mut harness, "Über Google anmelden");
    harness.run_steps(4);
    let place = |harness: &egui_kittest::Harness<'_, crate::Launcher>| {
        let brand = harness
            .get_by_role_and_label(Role::Link, "Gelabber")
            .accesskit_node()
            .bounding_box()
            .expect("placed");
        let links = harness
            .get_by_role_and_label(Role::Link, "Impressum")
            .accesskit_node()
            .bounding_box()
            .expect("placed");
        (brand.y0, links.y1)
    };
    let (top, bottom) = place(&harness);
    assert!(
        (top - (800.0 - bottom)).abs() <= 1.0,
        "brand at {top}, links end at {bottom}"
    );
    harness.run_steps(4);
    assert_eq!(place(&harness), (top, bottom));
}

// ------------------------------------------------- sign-in with Google

/// `GET /auth/providers` listing Google.
fn google(method: &str, at: &str) -> Option<Reply> {
    (method == "GET" && at == "/auth/providers").then(|| {
        Reply::json(serde_json::json!({"providers": [{"id": "google", "name": "Google"}]}))
    })
}

/// A stand-in for the system browser: it notes the provider's page and,
/// when `follow`, goes to it on a thread of its own (the app's loopback
/// receiver accepts only once `open` has returned).
fn browser(
    harness: &egui_kittest::Harness<'_, crate::Launcher>,
    follow: bool,
) -> std::sync::Arc<parking_lot::Mutex<Vec<String>>> {
    let opened: std::sync::Arc<parking_lot::Mutex<Vec<String>>> = std::sync::Arc::default();
    let seen = opened.clone();
    super::oidc::set_opener(
        &harness.ctx,
        std::sync::Arc::new(move |url: &str| {
            seen.lock().push(url.to_owned());
            if follow {
                let url = url.to_owned();
                std::thread::spawn(move || get(&url));
            }
            Ok(())
        }),
    );
    opened
}

/// A plain HTTP GET of `url` (`http://127.0.0.1:port/path?query`).
fn get(url: &str) {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").expect("http");
    let (host, path) = rest.split_once('/').expect("path");
    let mut stream = std::net::TcpStream::connect(host).expect("connect");
    let request = format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).expect("write");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
}

/// `start`'s answer: the "provider" sends the browser straight back to the
/// app's loopback port with `code`, as the API's callback would.
fn back_to_the_app(body: &str, code: &str) -> Reply {
    let start: serde_json::Value = serde_json::from_str(body).expect("start body");
    let desktop = &start["desktop"];
    let url = format!(
        "http://127.0.0.1:{}/callback?state={}&code={code}",
        desktop["port"],
        desktop["state"].as_str().expect("state")
    );
    Reply::json(serde_json::json!({ "url": url }))
}

/// The provider button and the divider show only once the server lists a
/// provider, in both languages.
#[test]
fn the_google_button_shows_when_a_provider_is_listed() {
    let _config = TempConfig::new("auth-oidc-button");
    let none = FakeApi::start(false, |_, _, _| None);
    let mut harness = launcher(&none, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    harness.run_steps(10);
    assert!(harness.query_by_label("Über Google anmelden").is_none());
    assert!(harness.query_by_label("oder").is_none());

    let api = FakeApi::start(false, |method, at, _| google(method, at));
    for route in ["/login", "/register"] {
        let mut harness = launcher(&api, route);
        wait_for_label(&mut harness, "Über Google anmelden");
        let button = harness.get_by_label("Über Google anmelden");
        assert_eq!(button.accesskit_node().role(), Role::Button);
        assert!(api.saw("GET /auth/providers"));
    }
    let _locale = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Sign in with Google");
}

/// A click opens the provider's page in the browser; the card waits with
/// "Weiter im Browser …" and "Abbrechen", which ends the wait.
#[test]
fn the_card_waits_for_the_browser_and_can_give_up() {
    let _config = TempConfig::new("auth-oidc-wait");
    let api = FakeApi::start(false, |method, at, _| {
        google(method, at).or_else(|| {
            (method == "POST" && at == "/auth/oidc/google/start").then(|| {
                Reply::json(serde_json::json!({"url": "https://accounts.example/o/auth?x=1"}))
            })
        })
    });
    let mut harness = launcher(&api, "/login?redirect=%2Finvite%2Fabc");
    let opened = browser(&harness, false);
    wait_for_label(&mut harness, "Über Google anmelden");
    harness.get_by_label("Über Google anmelden").click();
    wait_for_label(&mut harness, "Weiter im Browser …");
    assert_eq!(
        opened.lock().as_slice(),
        ["https://accounts.example/o/auth?x=1"]
    );
    assert!(
        harness
            .get_by_label("Weiter im Browser …")
            .accesskit_node()
            .is_disabled()
    );
    harness
        .get_by_role_and_label(Role::Button, "Abbrechen")
        .click();
    wait_for_label(&mut harness, "Über Google anmelden");
    assert!(harness.query_by_label("Abbrechen").is_none());
}

/// The browser comes back with a code: the exchange signs in, and the app
/// goes where the sign-in was headed.
#[test]
fn a_provider_sign_in_signs_in() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _config = TempConfig::new("auth-oidc-signed-in");
    let signed_in = std::sync::Arc::new(AtomicBool::new(false));
    let flag = signed_in.clone();
    let api = FakeApi::start(false, move |method, at, body| match (method, at) {
        ("POST", "/auth/oidc/google/start") => {
            let start: serde_json::Value = serde_json::from_str(body).expect("body");
            assert_eq!(start["purpose"], "sign_in");
            assert_eq!(start["redirect"], "/settings/profil");
            assert_eq!(
                start["desktop"]["challenge"].as_str().map(str::len),
                Some(43)
            );
            Some(back_to_the_app(body, &"c".repeat(64)))
        }
        ("POST", "/auth/oidc/desktop/exchange") => {
            let exchange: serde_json::Value = serde_json::from_str(body).expect("body");
            assert_eq!(exchange["code"], "c".repeat(64));
            flag.store(true, Ordering::SeqCst);
            Some(Reply::json(serde_json::json!({
                "result": "signed_in", "user": crate::app::testkit::lena(), "csrf_token": "c2"
            })))
        }
        ("GET", "/auth/session") if flag.load(Ordering::SeqCst) => Some(Reply::json(
            serde_json::json!({"user": crate::app::testkit::lena(), "csrf_token": "c2"}),
        )),
        ("GET", "/me") if flag.load(Ordering::SeqCst) => {
            Some(Reply::json(crate::app::testkit::lena()))
        }
        _ => google(method, at),
    });
    let mut harness = launcher(&api, "/login?redirect=%2Fsettings%2Fprofil");
    browser(&harness, true);
    wait_for_label(&mut harness, "Über Google anmelden");
    harness.get_by_label("Über Google anmelden").click();
    wait_for(&mut harness, "signed in", |h| {
        h.query_by_label("E-Mail-Adresse").is_none() && signed_in.load(Ordering::SeqCst)
    });
    let path = harness
        .state_mut()
        .app()
        .expect("app")
        .nav()
        .location()
        .path
        .clone();
    assert_eq!(path, "/settings/profil");
}

/// A refusal of the exchange is the form's error, in the web's words; an
/// `oidc_error` in the address shows in the same place.
#[test]
fn a_refused_provider_sign_in_says_why() {
    let _config = TempConfig::new("auth-oidc-refused");
    let api = FakeApi::start(false, |method, at, body| match (method, at) {
        ("POST", "/auth/oidc/google/start") => Some(back_to_the_app(body, &"d".repeat(64))),
        ("POST", "/auth/oidc/desktop/exchange") => {
            Some(Reply::error(409, "oidc_account_unverified"))
        }
        _ => google(method, at),
    });
    let mut harness = launcher(&api, "/login");
    browser(&harness, true);
    wait_for_label(&mut harness, "Über Google anmelden");
    harness.get_by_label("Über Google anmelden").click();
    wait_for_text(
        &mut harness,
        "Zu dieser E-Mail-Adresse gibt es schon ein Konto, dessen Adresse nie bestätigt wurde.",
    );
    // The button is back for another try.
    harness.get_by_label("Über Google anmelden");

    let mut harness = launcher(&api, "/login?oidc_error=cancelled");
    wait_for_text(&mut harness, "Anmeldung abgebrochen.");
    let mut harness = launcher(&api, "/register?oidc_error=invitation_required");
    wait_for_text(&mut harness, "Zum Registrieren brauchst du eine Einladung.");
    // An unknown code is not read.
    let mut harness = launcher(&api, "/login?oidc_error=registration_closed");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    harness.run_steps(5);
    assert!(harness.query_by_role(Role::Alert).is_none());
}

/// A new account: the name step with the provider's address and name, then
/// `complete` with the flow's verifier signs in.
#[test]
fn a_first_provider_sign_in_picks_its_name() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _config = TempConfig::new("auth-oidc-name");
    let code = "e".repeat(64);
    let created = std::sync::Arc::new(AtomicBool::new(false));
    let flag = created.clone();
    let challenge = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
    let kept = challenge.clone();
    let token = code.clone();
    let api = FakeApi::start(false, move |method, at, body| match (method, at) {
        ("POST", "/auth/oidc/google/start") => {
            let start: serde_json::Value = serde_json::from_str(body).expect("body");
            *kept.lock() = start["desktop"]["challenge"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Some(back_to_the_app(body, &token))
        }
        ("POST", "/auth/oidc/desktop/exchange") => Some(Reply::json(serde_json::json!({
            "result": "needs_name",
            "provider": {"id": "google", "name": "Google"},
            "email": "neu@example.test", "name": "Neu Ling"
        }))),
        ("POST", "/auth/oidc/pending") => Some(Reply::json(serde_json::json!({
            "provider": {"id": "google", "name": "Google"},
            "email": "neu@example.test", "name": "Neu Ling"
        }))),
        ("POST", "/auth/oidc/complete") => {
            let complete: serde_json::Value = serde_json::from_str(body).expect("body");
            assert_eq!(complete["token"], token);
            assert_eq!(complete["name"], "Neu Ling");
            let verifier = complete["verifier"].as_str().unwrap_or_default();
            assert_eq!(
                gelabber_client::oidc::challenge(verifier),
                *kept.lock(),
                "the flow's verifier"
            );
            flag.store(true, Ordering::SeqCst);
            Some(Reply::status(
                201,
                serde_json::json!({
                    "user": crate::app::testkit::lena(), "csrf_token": "c3", "redirect": null
                }),
            ))
        }
        ("GET", "/auth/session") if flag.load(Ordering::SeqCst) => Some(Reply::json(
            serde_json::json!({"user": crate::app::testkit::lena(), "csrf_token": "c3"}),
        )),
        ("GET", "/me") if flag.load(Ordering::SeqCst) => {
            Some(Reply::json(crate::app::testkit::lena()))
        }
        _ => google(method, at),
    });
    let mut harness = launcher(&api, "/register");
    browser(&harness, true);
    wait_for_label(&mut harness, "Über Google anmelden");
    harness.get_by_label("Über Google anmelden").click();
    // The provider's name for the person is the suggestion.
    wait_for_label(&mut harness, "Anzeigename");
    assert_eq!(
        input(&harness, "Anzeigename")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("Neu Ling")
    );
    let path = harness
        .state_mut()
        .app()
        .expect("app")
        .nav()
        .location()
        .href();
    assert_eq!(path, "/sign-in/name");
    harness
        .get_by_role_and_label(Role::Button, "Konto anlegen")
        .click();
    wait_for(&mut harness, "account created", |h| {
        created.load(Ordering::SeqCst) && h.query_by_label("Anzeigename").is_none()
    });
}

/// The name step's token is gone: the page says the sign-in expired.
#[test]
fn a_dead_name_step_says_the_sign_in_expired() {
    let _config = TempConfig::new("auth-oidc-dead");
    let api = FakeApi::start(false, |method, at, _| {
        (method == "POST" && at == "/auth/oidc/pending").then(|| Reply::error(410, "token_invalid"))
    });
    let mut harness = launcher(&api, &format!("/sign-in/name{LINK}"));
    wait_for_label(&mut harness, "Zur Anmeldung");
    assert!(api.saw("POST /auth/oidc/pending"));
    let api = FakeApi::start(false, |_, _, _| None);
    let mut harness = launcher(&api, "/sign-in/name");
    wait_for_label(&mut harness, "Zur Anmeldung");
    assert!(!api.saw("POST /auth/oidc/pending"));
}

// ------------------------------------------------------------ invitations

/// The stand-in API with `mode` for `GET /auth/registration`; the bodies of
/// `POST /auth/register` land in the returned list, and `answer` replies.
fn registration(
    mode: &'static str,
    answer: fn() -> Reply,
) -> (
    FakeApi,
    std::sync::Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
) {
    let bodies: std::sync::Arc<parking_lot::Mutex<Vec<serde_json::Value>>> = Default::default();
    let seen = bodies.clone();
    let api = FakeApi::start(false, move |method, at, body| match (method, at) {
        ("GET", "/auth/registration") => Some(Reply::json(serde_json::json!({"mode": mode}))),
        ("POST", "/auth/register") => {
            seen.lock()
                .push(serde_json::from_str(body).unwrap_or_default());
            Some(answer())
        }
        _ => None,
    });
    (api, bodies)
}

fn mailed() -> Reply {
    Reply::status(
        202,
        serde_json::json!({"user": null, "csrf_token": "csrf", "verification_sent": true}),
    )
}

fn fill_account(harness: &mut egui_kittest::Harness<'_, crate::Launcher>) {
    wait_for_label(harness, "Name");
    type_into(harness, "Name", "Mira");
    type_into(harness, "E-Mail-Adresse", "mira@example.test");
    type_into(harness, "Passwort", "parity-pass-123");
}

/// `/register?invitation=`: the field shows, pre-filled, and the code goes
/// with the registration, next to the language.
#[test]
fn an_invitation_link_fills_the_code_and_sends_it() {
    let _config = TempConfig::new("auth-invitation-link");
    let (api, bodies) = registration("open", mailed);
    let mut harness = launcher(&api, "/register?invitation=k7m2p9qrs4tx");
    fill_account(&mut harness);
    assert_eq!(
        input(&harness, "Einladungscode")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("k7m2p9qrs4tx")
    );
    assert!(
        harness
            .query_by_label("Gelabber ist gerade nur mit Einladung offen.")
            .is_none()
    );
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    wait_for_label(&mut harness, "Andere Adresse verwenden");
    let body = bodies.lock()[0].clone();
    assert_eq!(body["invitation"], "k7m2p9qrs4tx");
    assert_eq!(body["locale"], "de");
    assert!(body.get("server_invite").is_none(), "{body}");
}

/// Invitation-only without a code: the line above the heading, the field,
/// and no request while it is empty.
#[test]
fn an_invitation_only_instance_asks_for_a_code() {
    let _config = TempConfig::new("auth-invitation-only");
    let (api, bodies) = registration("invitation", mailed);
    let mut harness = launcher(&api, "/register");
    wait_for_label(&mut harness, "Gelabber ist gerade nur mit Einladung offen.");
    fill_account(&mut harness);
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    // The field's own error takes the focus (its copy: rules.rs tests).
    wait_for(&mut harness, "the code field's error", |h| {
        crate::app::testkit::focused_label(h).as_deref() == Some("Einladungscode")
    });
    assert!(bodies.lock().is_empty());
    type_into(&mut harness, "Einladungscode", " abc123 ");
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    wait_for_label(&mut harness, "Andere Adresse verwenden");
    assert_eq!(bodies.lock()[0]["invitation"], "abc123");
}

/// Following a server invite on an invitation-only instance: no field, no
/// line, the invite's code goes along, and a refusal is the form's line.
#[test]
fn a_server_invite_admits_without_the_field() {
    let _config = TempConfig::new("auth-invitation-server");
    let (api, bodies) = registration("invitation", || {
        Reply::status(
            410,
            serde_json::json!({"error": "invitation_invalid", "message": "x",
                               "fields": {"invitation": "invalid"}}),
        )
    });
    let mut harness = launcher(&api, "/register?redirect=%2Finvite%2Fabc123");
    fill_account(&mut harness);
    harness.run_steps(5);
    assert!(
        harness
            .query_all_by_label("Einladungscode")
            .next()
            .is_none()
    );
    assert!(
        harness
            .query_by_label("Gelabber ist gerade nur mit Einladung offen.")
            .is_none()
    );
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    wait_for_text(
        &mut harness,
        "Diese Einladung ist unbekannt, abgelaufen oder schon benutzt.",
    );
    let body = bodies.lock()[0].clone();
    assert_eq!(body["server_invite"], "abc123");
    assert!(body.get("invitation").is_none(), "{body}");
    assert!(
        harness
            .query_all_by_label("Einladungscode")
            .next()
            .is_none()
    );
}

/// An open instance that asks for a code after all brings the field up,
/// with the API's message under it.
#[test]
fn a_refusal_for_a_code_brings_the_field_up() {
    let _config = TempConfig::new("auth-invitation-asked");
    let (api, _) = registration("open", || {
        Reply::status(
            403,
            serde_json::json!({"error": "invitation_required", "message": "x",
                               "fields": {"invitation": "required"}}),
        )
    });
    let mut harness = launcher(&api, "/register");
    fill_account(&mut harness);
    assert!(
        harness
            .query_all_by_label("Einladungscode")
            .next()
            .is_none()
    );
    harness
        .get_by_role_and_label(Role::Button, "Registrieren")
        .click();
    wait_for_label(&mut harness, "Einladungscode");
    wait_for(&mut harness, "the code field's error", |h| {
        crate::app::testkit::focused_label(h).as_deref() == Some("Einladungscode")
    });
}

/// A disabled account's sign-in says so (`account_disabled`).
#[test]
fn a_disabled_account_is_told_at_sign_in() {
    let _config = TempConfig::new("auth-account-disabled");
    let api = FakeApi::start(false, |method, at, _| {
        (method == "POST" && at == "/auth/login").then(|| Reply::error(403, "account_disabled"))
    });
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "E-Mail-Adresse");
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Anmelden")
        .click();
    wait_for_text(&mut harness, "Dieses Konto ist gesperrt.");
}
