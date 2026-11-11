//! The signed-out forms against the stand-in API (app/testkit.rs): rate
//! limits (useFormErrors, useCooldown) and the caps-lock hint
//! (PasswordField).

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::testkit::{FakeApi, Reply, launcher, wait_for_label, wait_for_text};
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

/// The column centres itself with the link row counted in: as much room
/// above the brand as below the links, and it stays put.
#[test]
fn the_column_is_centred_with_the_legal_links() {
    let _config = TempConfig::new("auth-legal-centred");
    let api = FakeApi::start(false, published);
    let mut harness = launcher(&api, "/login");
    wait_for_label(&mut harness, "Nutzungsbedingungen");
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
