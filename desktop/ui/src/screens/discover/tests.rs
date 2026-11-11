//! kittest tests of Entdecken against the stand-in API (DiscoverPage.test.tsx,
//! ServerRail.test.tsx): the rail's tile and the route only while the
//! directory is on, the row buttons, the language filter, the search in
//! the address, joining, and the request and report dialogs.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::app::testkit::{FakeApi, HOME, Reply, launcher, wait_for, wait_for_label, workspace};
use crate::platform::paths::testing::TempConfig;
use crate::route::{DiscoverSearch, Route};

type Harness = egui_kittest::Harness<'static, crate::Launcher>;

const FILMCLUB: &str = "00000000-0000-4000-8000-0000000000d1";
const RUST: &str = "00000000-0000-4000-8000-0000000000d2";
const KELLER: &str = "00000000-0000-4000-8000-0000000000d3";
const SPIELE: &str = "00000000-0000-4000-8000-0000000000d4";
const CHOR: &str = "00000000-0000-4000-8000-0000000000d5";

fn listing(
    id: &str,
    name: &str,
    language: &str,
    mode: &str,
    member: bool,
    request: Value,
) -> Value {
    json!({
        "server_id": id, "name": name, "icon_url": null,
        "description": format!("Über {name}."), "language": language,
        "member_count": 3, "join_mode": mode, "member": member, "request": request,
        "retry_at": if request == json!("declined") { json!("2026-11-18T10:00:00Z") } else { Value::Null },
    })
}

/// One row per button state: member, open, request, pending, declined.
fn rows() -> Vec<Value> {
    vec![
        listing(HOME, "Wohnzimmer", "de", "request", true, Value::Null),
        listing(FILMCLUB, "Filmclub", "de", "open", false, Value::Null),
        listing(RUST, "Rust Study Hall", "en", "open", false, Value::Null),
        listing(
            KELLER,
            "Modellbau-Keller",
            "de",
            "request",
            false,
            Value::Null,
        ),
        listing(
            SPIELE,
            "Spieleabend",
            "de",
            "request",
            false,
            json!("pending"),
        ),
        listing(CHOR, "Chor", "de", "request", false, json!("declined")),
    ]
}

/// Requests with their bodies, for the writes.
type Sent = Arc<Mutex<Vec<(String, String)>>>;

fn api(enabled: bool, sent: Sent) -> FakeApi {
    FakeApi::start(true, move |method, path, body| match (method, path) {
        ("GET", "/directory") if enabled => Some(Reply::json(json!({
            "enabled": true,
            "languages": [
                {"code": "de", "name": "Deutsch", "listings": 5},
                {"code": "en", "name": "English", "listings": 1},
                {"code": "es", "name": "Español", "listings": 0}
            ]
        }))),
        ("GET", "/directory") => Some(Reply::json(json!({"enabled": false, "languages": []}))),
        ("GET", "/directory/listings") if enabled => Some(Reply::json(
            json!({"listings": rows(), "next_offset": null}),
        )),
        ("POST", p) if p == format!("/directory/listings/{FILMCLUB}/join") => {
            sent.lock()
                .push((format!("{method} {path}"), body.to_owned()));
            let mut server = workspace("GET", "/servers")?.body[0].clone();
            server["id"] = json!(FILMCLUB);
            server["name"] = json!("Filmclub");
            server["role"] = json!("member");
            Some(Reply::json(server))
        }
        ("POST", p) if p.ends_with("/request") => {
            sent.lock()
                .push((format!("{method} {path}"), body.to_owned()));
            Some(Reply::status(201, json!({"request": "pending"})))
        }
        ("DELETE", p) if p.ends_with("/request") => {
            sent.lock()
                .push((format!("{method} {path}"), body.to_owned()));
            Some(Reply::status(204, Value::Null))
        }
        ("POST", p) if p.ends_with("/reports") => {
            sent.lock()
                .push((format!("{method} {path}"), body.to_owned()));
            Some(Reply::status(
                201,
                json!({"id": "00000000-0000-4000-8000-0000000000e1",
                           "created_at": "2026-11-11T10:00:00Z"}),
            ))
        }
        _ => workspace(method, path),
    })
}

fn route(harness: &mut Harness) -> Route {
    harness
        .state_mut()
        .app()
        .expect("app")
        .nav()
        .route()
        .clone()
}

fn steps(harness: &mut Harness, n: usize) {
    for _ in 0..n {
        harness.step();
    }
}

fn button<'a>(harness: &'a Harness, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

fn sent_to(sent: &Sent, line: &str) -> Option<String> {
    sent.lock()
        .iter()
        .find(|(l, _)| l == line)
        .map(|(_, b)| b.clone())
}

/// Each row's button says what the caller can do; the pending and the
/// declined one cannot be pressed, and the declined one says from when.
#[test]
fn the_rows_offer_what_the_caller_can_do() {
    let _config = TempConfig::new("discover-states");
    let api = api(true, Arc::default());
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "Mehr zu Wohnzimmer");
    for (name, label) in [
        ("Wohnzimmer", "Öffnen"),
        ("Modellbau-Keller", "Beitritt anfragen"),
        ("Spieleabend", "Angefragt"),
        ("Chor", "Nicht angenommen"),
    ] {
        assert!(harness.query_by_label(&format!("Mehr zu {name}")).is_some());
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    assert_eq!(harness.query_all_by_label("Beitreten").count(), 2);
    assert!(button(&harness, "Angefragt").accesskit_node().is_disabled());
    let declined = button(&harness, "Nicht angenommen");
    assert!(declined.accesskit_node().is_disabled());
    assert!(
        declined
            .accesskit_node()
            .description()
            .is_some_and(|d| d.starts_with("Du kannst ab dem ") && d.ends_with("wieder anfragen."))
    );
    // "Deutsch · 3 Mitglieder" under the rows; the sidebar lists only the
    // languages with listings.
    harness.get_by_role_and_label(Role::Button, "Alle");
    harness.get_by_role_and_label(Role::Button, "English");
    assert!(harness.query_by_label("Español").is_none());
    // The rail's tile is there and marks the page.
    harness.get_by_role_and_label(Role::Link, "Entdecken");
}

/// Off, the rail has no tile and `/discover` is not found.
#[test]
fn off_there_is_no_tile_and_no_page() {
    let _config = TempConfig::new("discover-off");
    let api = api(false, Arc::default());
    let mut harness = launcher(&api, &format!("/s/{HOME}"));
    wait_for_label(&mut harness, "Server hinzufügen");
    wait_for(&mut harness, "the status", |_| api.saw("GET /directory"));
    steps(&mut harness, 20);
    assert!(harness.query_by_label("Entdecken").is_none());
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "Zur Startseite");
    assert!(!api.saw("GET /directory/listings"));
}

/// A language writes `?lang=` and a typed search `?q=` once typing
/// pauses; the rail's tile goes back to the whole directory.
#[test]
fn the_filter_and_the_search_live_in_the_address() {
    let _config = TempConfig::new("discover-filter");
    let api = api(true, Arc::default());
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "English");
    button(&harness, "English").click();
    steps(&mut harness, 3);
    assert_eq!(
        route(&mut harness),
        Route::Discover(DiscoverSearch::new(None, Some("en")))
    );
    let field = harness
        .query_all_by_label("Nach Name oder Thema suchen")
        .find(|n| n.accesskit_node().role() == Role::TextInput)
        .expect("the search");
    field.focus();
    harness.step();
    harness
        .query_all_by_label("Nach Name oder Thema suchen")
        .find(|n| n.accesskit_node().role() == Role::TextInput)
        .expect("the search")
        .type_text("rust");
    let expected = Route::Discover(DiscoverSearch::new(Some("rust"), Some("en")));
    let start = std::time::Instant::now();
    while route(&mut harness) != expected {
        assert!(start.elapsed().as_secs() < 5, "{:?}", route(&mut harness));
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    harness
        .get_by_role_and_label(Role::Link, "Entdecken")
        .click();
    steps(&mut harness, 3);
    assert_eq!(
        route(&mut harness),
        Route::Discover(DiscoverSearch::default())
    );
}

/// "Beitreten" joins and opens the server.
#[test]
fn joining_opens_the_server() {
    let _config = TempConfig::new("discover-join");
    let sent: Sent = Arc::default();
    let api = api(true, Arc::clone(&sent));
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "Mehr zu Filmclub");
    harness
        .query_all_by_label("Beitreten")
        .next()
        .expect("Beitreten")
        .click();
    let target = Route::Server(FILMCLUB.parse().expect("id"));
    let start = std::time::Instant::now();
    while route(&mut harness) != target {
        assert!(start.elapsed().as_secs() < 10, "{:?}", route(&mut harness));
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(sent_to(&sent, &format!("POST /directory/listings/{FILMCLUB}/join")).is_some());
}

/// "Beitritt anfragen" sends the message and the row says "Angefragt";
/// the menu then offers to withdraw it.
#[test]
fn a_request_is_sent_and_can_be_withdrawn() {
    let _config = TempConfig::new("discover-request");
    let sent: Sent = Arc::default();
    let api = api(true, Arc::clone(&sent));
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "Beitritt anfragen");
    button(&harness, "Beitritt anfragen").click();
    wait_for_label(&mut harness, "Anfrage an Modellbau-Keller");
    harness
        .query_all_by_label("Nachricht (optional)")
        .find(|n| n.accesskit_node().role() == Role::TextInput)
        .expect("the message")
        .type_text("Hallo!");
    harness.step();
    button(&harness, "Anfrage senden").click();
    let line = format!("POST /directory/listings/{KELLER}/request");
    wait_for(&mut harness, "the request", |_| {
        sent_to(&sent, &line).is_some()
    });
    assert_eq!(
        sent_to(&sent, &line).as_deref(),
        Some(r#"{"message":"Hallo!"}"#)
    );
    wait_for(&mut harness, "the dialog to close", |h| {
        h.query_by_label("Anfrage an Modellbau-Keller").is_none()
    });
    assert_eq!(harness.query_all_by_label("Angefragt").count(), 2);
    button(&harness, "Mehr zu Modellbau-Keller").click();
    wait_for(&mut harness, "the menu", |h| {
        h.query_by_role_and_label(Role::MenuItem, "Anfrage zurückziehen")
            .is_some()
    });
    harness
        .get_by_role_and_label(Role::MenuItem, "Anfrage zurückziehen")
        .click();
    let line = format!("DELETE /directory/listings/{KELLER}/request");
    wait_for(&mut harness, "the withdrawal", |_| {
        sent_to(&sent, &line).is_some()
    });
    wait_for_label(&mut harness, "Beitritt anfragen");
}

/// "Melden": a reason is needed, "Sonstiges" needs details, then the
/// report goes to the operator.
#[test]
fn a_report_needs_a_reason_and_for_other_details() {
    let _config = TempConfig::new("discover-report");
    let sent: Sent = Arc::default();
    let api = api(true, Arc::clone(&sent));
    let mut harness = launcher(&api, "/discover");
    wait_for_label(&mut harness, "Mehr zu Filmclub");
    button(&harness, "Mehr zu Filmclub").click();
    wait_for(&mut harness, "the menu", |h| {
        h.query_by_role_and_label(Role::MenuItem, "Melden")
            .is_some()
    });
    // Not pending: no withdrawal.
    assert!(
        harness
            .query_by_role_and_label(Role::MenuItem, "Anfrage zurückziehen")
            .is_none()
    );
    harness
        .get_by_role_and_label(Role::MenuItem, "Melden")
        .click();
    wait_for_label(
        &mut harness,
        "Meldung zu Filmclub. Sie geht an den Betreiber.",
    );
    button(&harness, "Melden").click();
    wait_for_label(&mut harness, "Bitte einen Grund wählen.");
    harness
        .get_by_role_and_label(Role::RadioButton, "Sonstiges")
        .click();
    steps(&mut harness, 2);
    button(&harness, "Melden").click();
    steps(&mut harness, 2);
    let details = harness
        .query_all_by_label("Details")
        .find(|n| n.accesskit_node().role() == Role::MultilineTextInput)
        .expect("details");
    assert_eq!(
        details.accesskit_node().description().as_deref(),
        Some("Bei „Sonstiges“ bitte kurz beschreiben, worum es geht.")
    );
    details.focus();
    harness.step();
    harness
        .query_all_by_label("Details")
        .find(|n| n.accesskit_node().role() == Role::MultilineTextInput)
        .expect("details")
        .type_text("Werbung");
    harness.step();
    button(&harness, "Melden").click();
    let line = format!("POST /servers/{FILMCLUB}/reports");
    wait_for(&mut harness, "the report", |_| {
        sent_to(&sent, &line).is_some()
    });
    assert_eq!(
        sent_to(&sent, &line).as_deref(),
        Some(r#"{"reason":"other","details":"Werbung"}"#)
    );
    wait_for_label(&mut harness, "Danke, die Meldung ist beim Betreiber.");
}
