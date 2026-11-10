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
