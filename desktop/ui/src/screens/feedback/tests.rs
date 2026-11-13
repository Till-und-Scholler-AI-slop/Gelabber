//! The "Feedback geben" dialog against the stand-in API
//! (FeedbackDialog.test.tsx).

use std::sync::Arc;

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use parking_lot::Mutex;
use serde_json::json;

use crate::app::testkit::{FakeApi, HOME, Reply, launcher, wait_for, wait_for_label, workspace};
use crate::platform::paths::testing::TempConfig;
use crate::screens::auth::tests::type_into;

type App = egui_kittest::Harness<'static, crate::Launcher>;

const IMAGE: &str = "00000000-0000-4000-8000-0000000000f5";

fn entry() -> serde_json::Value {
    json!({
        "id": "00000000-0000-4000-8000-0000000000f1", "kind": "problem", "status": "open",
        "text": "Schwarz.", "created_at": "2026-11-12T09:00:00Z",
        "last_activity_at": "2026-11-12T09:00:00Z", "image": null,
        "context": {"client": "desktop", "os": "linux", "app_version": "0.10.0",
                    "locale": "de", "page": "/"},
        "unread": false, "messages": []
    })
}

/// Lena's workspace on an instance that takes feedback, with `unread`
/// replies. Every write is kept with its body.
fn api(unread: u32, posts: Arc<Mutex<Vec<(String, String)>>>) -> FakeApi {
    FakeApi::start(true, move |method, path, body| match (method, path) {
        ("GET", "/feedback/status") => {
            Some(Reply::json(json!({"enabled": true, "unread": unread})))
        }
        ("POST", "/feedback/images") => {
            posts.lock().push((path.to_owned(), body.to_owned()));
            Some(Reply::status(201, json!({"image": IMAGE})))
        }
        ("POST", "/feedback") => {
            posts.lock().push((path.to_owned(), body.to_owned()));
            Some(Reply::status(201, entry()))
        }
        _ => workspace(method, path),
    })
}

fn open_dialog(harness: &mut App) {
    wait_for(harness, "the menu item", |h| {
        h.query_by_label("Benutzermenü").is_some()
            || h.query_by_label_contains("Benutzermenü, ").is_some()
    });
    let chevron = harness
        .query_all_by_role(Role::Button)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Benutzermenü"))
        })
        .expect("the chevron");
    chevron.click();
    wait_for(harness, "Feedback geben", |h| {
        h.query_all_by_role(Role::Button).any(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Feedback geben"))
        })
    });
    harness
        .query_all_by_role(Role::Button)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Feedback geben"))
        })
        .expect("the item")
        .click();
    wait_for_label(harness, "Wird mitgeschickt");
}

fn text_field<'a>(harness: &'a App) -> egui_kittest::Node<'a> {
    harness
        .query_all_by_label("Deine Nachricht")
        .find(|n| n.accesskit_node().role() == Role::MultilineTextInput)
        .expect("the text")
}

fn type_text(harness: &mut App, text: &str) {
    text_field(harness).focus();
    harness.step();
    text_field(harness).type_text(text);
    harness.run_steps(2);
}

#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("read on the UI thread")
    }
}

fn drop_file(harness: &mut App, path: std::path::PathBuf) {
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(path)));
}

/// An image refusal: under "Bild anhängen", and said by the polite status
/// line as the web's `refuseImage` does.
fn wait_for_refusal(harness: &mut App, message: &str) {
    wait_for(harness, message, |h| {
        h.query_by_role_and_label(Role::Alert, message).is_some()
            && h.query_by_role_and_label(Role::Status, message).is_some()
    });
}

/// The label of "Senden", or of its pending state.
fn submit_label(harness: &egui_kittest::Harness<'_, crate::Launcher>) -> Option<String> {
    harness
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .find(|l| l == "Senden" || l == "Wird gesendet…")
}

/// What goes along is shown as it is sent: the app, this build's system,
/// the version, the language and the page.
#[test]
fn the_dialog_lists_what_goes_along() {
    let _config = TempConfig::new("feedback-along");
    let api = api(0, Arc::default());
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    let system = crate::screens::settings::security::os_name(gelabber_client::DeviceOs::THIS_BUILD)
        .expect("a known system");
    for shown in [
        "Desktop-App",
        system,
        env!("CARGO_PKG_VERSION"),
        "Deutsch",
        &format!("/s/{HOME}"),
        "Sonst nichts: keine Protokolle, keine weiteren Daten.",
    ] {
        assert!(harness.query_by_label(shown).is_some(), "{shown}");
    }
    // Idee is chosen; no unread line without a reply.
    let idea = harness.get_by_role_and_label(Role::RadioButton, "Idee");
    assert_eq!(idea.accesskit_node().toggled(), Some(Toggled::True));
    assert!(harness.query_by_label("Ansehen").is_none());
    // The list closes and opens again.
    harness
        .get_by_role_and_label(Role::DisclosureTriangle, "Wird mitgeschickt")
        .click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Desktop-App").is_none());
}

/// An unread reply: the chevron and the item say so, and the dialog has
/// the line with "Ansehen", which leaves for Einstellungen → Feedback.
#[test]
fn an_unread_reply_shows_in_the_menu_and_the_dialog() {
    let _config = TempConfig::new("feedback-unread");
    let api = api(1, Arc::default());
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    wait_for_label(&mut harness, "Benutzermenü, neue Antwort auf dein Feedback");
    harness
        .get_by_role_and_label(Role::Button, "Benutzermenü, neue Antwort auf dein Feedback")
        .click();
    wait_for_label(
        &mut harness,
        "Feedback geben, neue Antwort auf dein Feedback",
    );
    harness
        .get_by_role_and_label(
            Role::Button,
            "Feedback geben, neue Antwort auf dein Feedback",
        )
        .click();
    wait_for_label(&mut harness, "Ansehen");
    harness.get_by_role_and_label(Role::Link, "Ansehen").click();
    wait_for(&mut harness, "the settings", |h| {
        h.query_by_label("Wird mitgeschickt").is_none()
    });
    harness.run_steps(3);
    let route = harness.state_mut().app().unwrap().nav().route().clone();
    assert!(
        matches!(route, crate::route::Route::Settings(_)),
        "{route:?}"
    );
}

/// A text is required; a wrong type and a large file are refused on the
/// spot; a PNG goes up first, then the entry with its id, the chosen kind
/// and the context. The dialog closes with the toast and an empty draft.
#[test]
fn sending_uploads_the_image_first_then_creates_the_entry() {
    let config = TempConfig::new("feedback-send");
    let posts: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let api = api(0, posts.clone());
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);

    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    wait_for_label(&mut harness, "Schreib ein paar Worte.");
    assert!(posts.lock().is_empty());

    harness
        .get_by_role_and_label(Role::RadioButton, "Problem")
        .click();
    type_text(&mut harness, "Das Vorschaubild bleibt schwarz.");
    assert!(harness.query_by_label("Schreib ein paar Worte.").is_none());

    let gif = config.dir.join("bild.gif");
    std::fs::write(&gif, b"GIF89a\x01\0\x01\0\0\0\0;").expect("gif");
    drop_file(&mut harness, gif);
    wait_for_refusal(&mut harness, "Nur PNG, JPEG oder WebP.");

    // One byte over 5 MiB, a PNG by its first bytes.
    let large = config.dir.join("gross.png");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(gelabber_client::FEEDBACK_IMAGE_MAX_BYTES + 1, 0);
    std::fs::write(&large, &bytes).expect("large");
    drop_file(&mut harness, large);
    wait_for_refusal(&mut harness, "Das Bild ist größer als 5 MB.");

    let png = config.dir.join("bild.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 40, 40, 255]))
        .save(&png)
        .expect("png");
    let sent = std::fs::read(&png).expect("read");
    drop_file(&mut harness, png);
    wait_for_label(&mut harness, "Bild entfernen");
    assert!(
        harness
            .query_by_label("Das Bild ist größer als 5 MB.")
            .is_none()
    );

    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    wait_for_label(&mut harness, "Danke! Dein Feedback ist angekommen.");
    assert!(harness.query_by_label("Wird mitgeschickt").is_none());
    let posts = posts.lock().clone();
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert_eq!(posts[0].0, "/feedback/images");
    // The bytes as they were (the stand-in reads the body as text).
    assert_eq!(posts[0].1, String::from_utf8_lossy(&sent));
    assert_eq!(posts[1].0, "/feedback");
    let body: serde_json::Value = serde_json::from_str(&posts[1].1).expect("json");
    assert_eq!(body["kind"], "problem");
    assert_eq!(body["text"], "Das Vorschaubild bleibt schwarz.");
    assert_eq!(body["image"], IMAGE);
    assert_eq!(body["context"]["client"], "desktop");
    assert_eq!(body["context"]["page"], format!("/s/{HOME}"));
    assert_eq!(body["context"]["app_version"], env!("CARGO_PKG_VERSION"));

    // Sent: the next dialog starts empty, on Idee.
    open_dialog(&mut harness);
    assert!(harness.query_by_label("Bild entfernen").is_none());
    let idea = harness.get_by_role_and_label(Role::RadioButton, "Idee");
    assert_eq!(idea.accesskit_node().toggled(), Some(Toggled::True));
    assert_eq!(
        text_field(&harness).accesskit_node().value().as_deref(),
        Some("")
    );
}

/// "Abbrechen" keeps the draft; a refused send keeps it too and says why
/// under the field.
#[test]
fn closing_keeps_the_draft_and_errors_go_under_their_field() {
    let _config = TempConfig::new("feedback-draft");
    let api = FakeApi::start(true, |method, path, _| match (method, path) {
        ("GET", "/feedback/status") => Some(Reply::json(json!({"enabled": true, "unread": 0}))),
        ("POST", "/feedback") => Some(Reply::status(
            422,
            json!({"error": "validation_failed", "message": "x", "fields": {"text": "too_long"}}),
        )),
        _ => workspace(method, path),
    });
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    harness
        .get_by_role_and_label(Role::RadioButton, "Lob")
        .click();
    type_text(&mut harness, "Schön hier.");
    harness
        .get_by_role_and_label(Role::Button, "Abbrechen")
        .click();
    wait_for(&mut harness, "closed", |h| {
        h.query_by_label("Wird mitgeschickt").is_none()
    });
    open_dialog(&mut harness);
    assert_eq!(
        text_field(&harness).accesskit_node().value().as_deref(),
        Some("Schön hier.")
    );
    let praise = harness.get_by_role_and_label(Role::RadioButton, "Lob");
    assert_eq!(praise.accesskit_node().toggled(), Some(Toggled::True));
    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    wait_for_label(&mut harness, "Höchstens 4.000 Zeichen.");
    assert_eq!(
        text_field(&harness).accesskit_node().value().as_deref(),
        Some("Schön hier.")
    );
}

/// An instance without feedback (an API without the routes answers 404):
/// no item, no dot.
#[test]
fn without_feedback_the_menu_has_no_item() {
    let _config = TempConfig::new("feedback-off");
    let api = FakeApi::start(true, |method, path, _| workspace(method, path));
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    wait_for(&mut harness, "the status", |_| {
        api.saw("GET /feedback/status")
    });
    harness.run_steps(5);
    harness
        .get_by_role_and_label(Role::Button, "Benutzermenü")
        .click();
    wait_for_label(&mut harness, "Abmelden");
    assert!(harness.query_by_label_contains("Feedback geben").is_none());
}

#[test]
fn files_are_checked_by_their_first_bytes() {
    use super::dialog::sniff;
    assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
    assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
    assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(sniff(b"GIF89a"), None);
    assert_eq!(sniff(b"<svg/>"), None);
    assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
}

/// The hint says what the app takes: a file dropped on the window, not a
/// pasted image (the web's hint ends on pasting).
#[test]
fn the_hint_offers_dropping_and_a_dropped_image_attaches() {
    let config = TempConfig::new("feedback-drop");
    let api = api(0, Arc::default());
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    harness.get_by_label(
        "PNG, JPEG oder WebP, höchstens 5 MB. Du kannst ein Bild auch hierher ziehen.",
    );
    let png = config.dir.join("bild.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 40, 40, 255]))
        .save(&png)
        .expect("png");
    drop_file(&mut harness, png);
    wait_for(&mut harness, "the image", |h| {
        h.query_by_role_and_label(Role::Image, "Angehängtes Bild")
            .is_some()
            && h.query_by_role_and_label(Role::Status, "Bild angehängt.")
                .is_some()
    });
}

/// A drop on the dialog is the dialog's: the chat's composer under it
/// takes nothing.
#[test]
fn a_drop_on_the_dialog_does_not_reach_the_composer_under_it() {
    let config = TempConfig::new("feedback-drop-chat");
    // Lena may write and send files in allgemein.
    let api = FakeApi::start(true, |method, path, _| match (method, path) {
        ("GET", "/feedback/status") => Some(Reply::json(json!({"enabled": true, "unread": 0}))),
        ("GET", p) if p == format!("/servers/{HOME}") => workspace(method, path).map(|mut r| {
            r.body["permissions"] = json!(["manage_server", "send_messages", "send_files"]);
            r
        }),
        _ => workspace(method, path),
    });
    let mut harness = launcher(
        &api,
        &format!("/s/{HOME}/c/{}", crate::app::testkit::ALLGEMEIN),
    );
    wait_for_label(&mut harness, "Datei anhängen");
    open_dialog(&mut harness);
    let png = config.dir.join("bild.png");
    image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 40, 40, 255]))
        .save(&png)
        .expect("png");
    drop_file(&mut harness, png);
    wait_for_label(&mut harness, "Bild entfernen");
    harness.run_steps(20);
    harness
        .get_by_role_and_label(Role::Button, "Abbrechen")
        .click();
    wait_for(&mut harness, "closed", |h| {
        h.query_by_label("Wird mitgeschickt").is_none()
    });
    harness.run_steps(20);
    assert!(harness.query_by_label("Anhang entfernen").is_none());
    assert!(harness.query_by_label_contains("bild.png").is_none());
}

/// A send runs on when the dialog closes: the toast with "Ansehen", the
/// empty draft and a fresh status follow. A dialog opened again meanwhile
/// shows "Senden" pending and sends nothing twice.
#[test]
fn a_send_finishes_after_the_dialog_closed() {
    let _config = TempConfig::new("feedback-send-closed");
    let posts: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let kept = posts.clone();
    let api = FakeApi::start(true, move |method, path, body| match (method, path) {
        ("GET", "/feedback/status") => Some(Reply::json(json!({"enabled": true, "unread": 0}))),
        ("POST", "/feedback") => {
            kept.lock().push((path.to_owned(), body.to_owned()));
            let _ = gate.lock().recv();
            Some(Reply::status(201, entry()))
        }
        _ => workspace(method, path),
    });
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    type_text(&mut harness, "Bitte dunkler.");
    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    wait_for(&mut harness, "the send", |_| api.saw("POST /feedback"));
    // "Wird gesendet…" is wider: "Abbrechen" moves once it shows.
    wait_for(&mut harness, "pending", |h| {
        submit_label(h).as_deref() == Some("Wird gesendet…")
    });
    harness
        .get_by_role_and_label(Role::Button, "Abbrechen")
        .click();
    wait_for(&mut harness, "closed while sending", |h| {
        h.query_by_label("Wird mitgeschickt").is_none()
    });

    // Opened again while it sends: "Wird gesendet…", and a press sends
    // nothing more.
    open_dialog(&mut harness);
    assert_eq!(submit_label(&harness).as_deref(), Some("Wird gesendet…"));
    harness
        .get_by_role_and_label(Role::Button, "Wird gesendet…")
        .click();
    harness.run_steps(5);
    harness
        .get_by_role_and_label(Role::Button, "Abbrechen")
        .click();
    wait_for(&mut harness, "closed again", |h| {
        h.query_by_label("Wird mitgeschickt").is_none()
    });

    let reads = |api: &FakeApi| {
        api.seen
            .lock()
            .iter()
            .filter(|r| *r == "GET /feedback/status")
            .count()
    };
    let before = reads(&api);
    release.send(()).expect("release");
    wait_for_label(&mut harness, "Danke! Dein Feedback ist angekommen.");
    harness.get_by_role_and_label(Role::Button, "Ansehen");
    wait_for(&mut harness, "the status read again", |_| {
        reads(&api) > before
    });
    assert!(harness.query_by_label("Wird mitgeschickt").is_none());
    assert_eq!(posts.lock().len(), 1);
    open_dialog(&mut harness);
    assert_eq!(submit_label(&harness).as_deref(), Some("Senden"));
    assert_eq!(
        text_field(&harness).accesskit_node().value().as_deref(),
        Some("")
    );
}

/// A dialog left open when the session ends neither opens again nor
/// offers its draft to the next sign-in, the same account's included.
#[test]
fn a_dialog_left_open_stays_with_its_sign_in() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _config = TempConfig::new("feedback-sign-out");
    let signed_in = Arc::new(AtomicBool::new(true));
    let flag = signed_in.clone();
    let api = FakeApi::start(true, move |method, path, _| {
        let user = if flag.load(Ordering::SeqCst) {
            crate::app::testkit::lena()
        } else {
            serde_json::Value::Null
        };
        match (method, path) {
            ("GET", "/feedback/status") => Some(Reply::json(json!({"enabled": true, "unread": 0}))),
            ("GET", "/auth/session") => {
                Some(Reply::json(json!({"user": user, "csrf_token": "csrf"})))
            }
            ("POST", "/auth/logout") => {
                flag.store(false, Ordering::SeqCst);
                Some(Reply::json(json!({"csrf_token": "csrf"})))
            }
            ("POST", "/auth/login") => {
                flag.store(true, Ordering::SeqCst);
                Some(Reply::json(json!({
                    "user": crate::app::testkit::lena(), "csrf_token": "csrf"
                })))
            }
            _ => workspace(method, path),
        }
    });
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    type_text(&mut harness, "Von der letzten Sitzung.");
    harness.state_mut().app().unwrap().client().logout();
    wait_for(&mut harness, "signed out", |_| {
        !signed_in.load(Ordering::SeqCst)
    });
    wait_for_label(&mut harness, "E-Mail-Adresse");
    assert!(harness.query_by_label("Wird mitgeschickt").is_none());
    type_into(&mut harness, "E-Mail-Adresse", "lena@example.test");
    type_into(&mut harness, "Passwort", "parity-pass-123");
    harness
        .get_by_role_and_label(Role::Button, "Anmelden")
        .click();
    wait_for(&mut harness, "the workspace", |h| {
        h.query_by_label("Benutzermenü").is_some()
    });
    harness.run_steps(10);
    assert!(harness.query_by_label("Wird mitgeschickt").is_none());
    open_dialog(&mut harness);
    assert_eq!(
        text_field(&harness).accesskit_node().value().as_deref(),
        Some("")
    );
}

/// A refused context field has no control in the dialog: the form's line
/// says it. A refused image from the server is said like one refused here.
#[test]
fn refusals_without_a_field_show_for_the_form() {
    let _config = TempConfig::new("feedback-context-refused");
    let refusal = Arc::new(Mutex::new(json!({"page": "invalid"})));
    let answer = refusal.clone();
    let api = FakeApi::start(true, move |method, path, _| match (method, path) {
        ("GET", "/feedback/status") => Some(Reply::json(json!({"enabled": true, "unread": 0}))),
        ("POST", "/feedback/images") => Some(Reply::status(201, json!({"image": IMAGE}))),
        ("POST", "/feedback") => Some(Reply::status(
            422,
            json!({"error": "validation_failed", "message": "x",
                   "fields": answer.lock().clone()}),
        )),
        _ => workspace(method, path),
    });
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    open_dialog(&mut harness);
    type_text(&mut harness, "Hallo.");
    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    let page = crate::rules::field_message("page", "invalid");
    wait_for_label(&mut harness, &page);

    *refusal.lock() = json!({"image": "invalid"});
    harness
        .get_by_role_and_label(Role::Button, "Senden")
        .click();
    wait_for_refusal(
        &mut harness,
        "Das Bild ließ sich nicht verwenden. Versuch ein anderes.",
    );
}
