//! Choosing a file to upload, where the web has `<input type=file>` (which
//! always works): the FileChooser portal of xdg-desktop-portal over zbus,
//! then zenity or kdialog, and without any of them a path field in the
//! window. Windows and macOS have no portal; the programs and the field
//! still apply.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

/// What a chooser said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Chosen(PathBuf),
    Cancelled,
    /// No chooser answered: ask in the window.
    InApp,
}

/// Where to look for choosers; tests point it at a private bus and at
/// programs that do not exist.
#[derive(Clone, Debug)]
pub struct Choosers {
    /// A bus address; the user's session bus when `None`.
    pub bus: Option<String>,
    pub portal: bool,
    pub zenity: String,
    pub kdialog: String,
}

impl Default for Choosers {
    fn default() -> Self {
        Self {
            bus: None,
            portal: cfg!(target_os = "linux"),
            zenity: "zenity".into(),
            kdialog: "kdialog".into(),
        }
    }
}

fn choosers_id() -> egui::Id {
    egui::Id::unique("gelabber-file-choosers")
}

/// The choosers `ctx` uses (tests set their own).
pub fn choosers(ctx: &egui::Context) -> Choosers {
    ctx.data(|d| d.get_temp::<Choosers>(choosers_id()))
        .unwrap_or_default()
}

pub fn set_choosers(ctx: &egui::Context, choosers: Choosers) {
    ctx.data_mut(|d| d.insert_temp(choosers_id(), choosers));
}

/// Asks the choosers in order, on the calling thread: the portal, zenity,
/// kdialog. `filter` holds glob patterns ("*.png").
pub fn choose(choosers: &Choosers, title: &str, filter: &[&str]) -> Choice {
    #[cfg(target_os = "linux")]
    if choosers.portal
        && let Some(choice) = portal(choosers.bus.as_deref(), title, filter)
    {
        return choice;
    }
    let patterns = filter.join(" ");
    let attempts: [(&str, Vec<String>); 2] = [
        (
            &choosers.zenity,
            vec![
                "--file-selection".into(),
                format!("--title={title}"),
                format!("--file-filter={patterns}"),
            ],
        ),
        (
            &choosers.kdialog,
            vec![
                "--title".into(),
                title.into(),
                "--getopenfilename".into(),
                ".".into(),
                patterns.clone(),
            ],
        ),
    ];
    for (program, args) in attempts {
        match std::process::Command::new(program).args(&args).output() {
            Ok(out) if out.status.success() => {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                return if path.is_empty() {
                    Choice::Cancelled
                } else {
                    Choice::Chosen(path.into())
                };
            }
            // Exit 1 is a cancelled dialog in both; anything else (no
            // display, a broken install) tries the next chooser.
            Ok(out) if out.status.code() == Some(1) => return Choice::Cancelled,
            Ok(_) | Err(_) => continue,
        }
    }
    Choice::InApp
}

/// `org.freedesktop.portal.FileChooser.OpenFile`; `None` when there is no
/// portal (or no session bus) to ask.
#[cfg(target_os = "linux")]
fn portal(bus: Option<&str>, title: &str, filter: &[&str]) -> Option<Choice> {
    use std::collections::HashMap;
    use zbus::blocking::{MessageIterator, connection};
    use zbus::message::Type;
    use zbus::zvariant::{OwnedValue, Value};

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";

    let builder = match bus {
        Some(address) => connection::Builder::address(address).ok()?,
        None => connection::Builder::session().ok()?,
    };
    let conn = builder.build().ok()?;
    let sender = conn
        .unique_name()?
        .trim_start_matches(':')
        .replace('.', "_");
    let token = "gelabber_file";
    let path = format!("{PATH}/request/{sender}/{token}");
    // Subscribed before the call, so the answer cannot come first.
    let rule = zbus::MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(REQUEST)
        .ok()?
        .member("Response")
        .ok()?
        .path(path.as_str())
        .ok()?
        .build();
    let mut answers = MessageIterator::for_match_rule(rule, &conn, None).ok()?;
    let patterns: Vec<(u32, String)> = filter.iter().map(|p| (0u32, (*p).to_owned())).collect();
    let mut options: HashMap<&str, Value<'_>> = HashMap::from([
        ("handle_token", Value::from(token)),
        ("modal", Value::from(true)),
    ]);
    if !patterns.is_empty() {
        let filters = vec![(title.to_owned(), patterns.clone())];
        options.insert("filters", Value::from(filters.clone()));
        options.insert("current_filter", Value::from(filters[0].clone()));
    }
    // No portal, or one without a file chooser: the next chooser.
    conn.call_method(
        Some(DESKTOP),
        PATH,
        Some("org.freedesktop.portal.FileChooser"),
        "OpenFile",
        &("", title, options),
    )
    .ok()?;
    // The user may take their time; the portal always answers.
    let message = answers.next()?.ok()?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().ok()?;
    if code != 0 {
        return Some(Choice::Cancelled);
    }
    let uris = results
        .get("uris")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();
    Some(
        uris.first()
            .and_then(|uri| url::Url::parse(uri).ok())
            .and_then(|url| url.to_file_path().ok())
            .map_or(Choice::Cancelled, Choice::Chosen),
    )
}

/// A file the user picked: its bytes and name, or `None` when nothing was
/// chosen (the slot of `pick_file` in the settings).
pub type Picked = Arc<Mutex<Option<Option<(Vec<u8>, String)>>>>;

/// Reads `path` for a slot: the bytes and the file name.
pub fn read(path: &std::path::Path) -> Option<(Vec<u8>, String)> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::read(path).ok().map(|bytes| (bytes, name))
}

/// A choice that ended in the window: the path field's state.
struct InApp {
    title: String,
    path: String,
    error: Option<String>,
    slot: Picked,
}

fn in_app_id() -> egui::Id {
    egui::Id::unique("gelabber-file-path-field")
}

/// Opens the file chooser for `slot`; it fills with the file, or `None`.
pub fn pick(ctx: &egui::Context, title: &str, filter: &[&str], slot: Picked) {
    let ctx = ctx.clone();
    let title = title.to_owned();
    let filter: Vec<String> = filter.iter().map(|f| (*f).to_owned()).collect();
    let choosers = choosers(&ctx);
    std::thread::spawn(move || {
        let refs: Vec<&str> = filter.iter().map(String::as_str).collect();
        match choose(&choosers, &title, &refs) {
            Choice::Chosen(path) => *slot.lock() = Some(read(&path)),
            Choice::Cancelled => *slot.lock() = Some(None),
            Choice::InApp => {
                let field = InApp {
                    title,
                    path: String::new(),
                    error: None,
                    slot,
                };
                ctx.data_mut(|d| d.insert_temp(in_app_id(), Arc::new(Mutex::new(Some(field)))));
            }
        }
        ctx.request_repaint();
    });
}

type Shared = Arc<Mutex<Option<InApp>>>;

/// The path field, when a choice fell through to the window. The app
/// calls it every frame.
pub fn show_in_app(ctx: &egui::Context) {
    let Some(shared) = ctx.data(|d| d.get_temp::<Shared>(in_app_id())) else {
        return;
    };
    let mut guard = shared.lock();
    let Some(field) = guard.as_mut() else {
        drop(guard);
        ctx.data_mut(|d| d.remove::<Shared>(in_app_id()));
        return;
    };
    let (cancel, submit) = (std::cell::Cell::new(false), std::cell::Cell::new(false));
    let title = field.title.clone();
    let error = field.error.clone();
    let path = &mut field.path;
    let output = crate::widgets::Modal::new("file-path", &title).show(
        ctx,
        |ui| {
            let input = crate::widgets::TextField::new("file-path", path)
                .label("Dateipfad")
                .placeholder("/home/…/bild.png")
                .autofocus(true)
                .error(error.as_deref())
                .show(ui);
            if input.submitted {
                submit.set(true);
            }
            input.response.changed()
        },
        Some(&mut |ui: &mut egui::Ui| {
            crate::screens::shell::overlays::row(ui, 36.0, 8.0, |ui| {
                if ui
                    .add(crate::widgets::Button::new("Abbrechen").ghost())
                    .clicked()
                {
                    cancel.set(true);
                }
                if ui
                    .add(crate::widgets::Button::new("Öffnen").primary())
                    .clicked()
                {
                    submit.set(true);
                }
            });
        }),
    );
    if output.inner {
        field.error = None;
    }
    let mut done = None;
    if output.close_requested || cancel.get() {
        done = Some(None);
    } else if submit.get() {
        done = open(field);
    }
    if let Some(result) = done {
        *field.slot.lock() = Some(result);
        *guard = None;
    }
}

/// The typed path's file, or the field's error.
fn open(field: &mut InApp) -> Option<Option<(Vec<u8>, String)>> {
    let typed = field.path.trim();
    let path = typed
        .strip_prefix("~/")
        .and_then(|rest| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)))
        .unwrap_or_else(|| PathBuf::from(typed));
    match read(&path) {
        Some(file) => Some(Some(file)),
        None => {
            field.error = Some("Diese Datei lässt sich nicht öffnen.".into());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing() -> Choosers {
        Choosers {
            bus: None,
            portal: false,
            zenity: "gelabber-test-no-zenity".into(),
            kdialog: "gelabber-test-no-kdialog".into(),
        }
    }

    /// Without a portal on the bus and without the programs: the window.
    #[test]
    fn without_choosers_the_window_asks() {
        #[cfg(target_os = "linux")]
        {
            let Some((_daemon, address)) = crate::platform::testing::private_bus("files-none")
            else {
                eprintln!("no dbus-daemon: skipped");
                return;
            };
            let choosers = Choosers {
                bus: Some(address),
                portal: true,
                ..nothing()
            };
            assert_eq!(choose(&choosers, "Bild", &["*.png"]), Choice::InApp);
        }
        assert_eq!(choose(&nothing(), "Bild", &["*.png"]), Choice::InApp);
        // A program that ran and found nothing chosen: cancelled, no field.
        let cancelled = Choosers {
            zenity: "false".into(),
            ..nothing()
        };
        assert_eq!(choose(&cancelled, "Bild", &[]), Choice::Cancelled);
        // One that failed otherwise (here ls on zenity's options, exit 2):
        // the next chooser, and at the end the window.
        let broken = Choosers {
            zenity: "ls".into(),
            ..nothing()
        };
        assert_eq!(choose(&broken, "Bild", &[]), Choice::InApp);
        let chosen = Choosers {
            zenity: "echo".into(),
            ..nothing()
        };
        assert!(matches!(choose(&chosen, "Bild", &[]), Choice::Chosen(_)));
    }

    /// The portal comes first and its answer is the file.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_portal_answers_first() {
        use std::collections::HashMap;
        use zbus::zvariant::{OwnedValue, Value};
        struct Fake;
        #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
        impl Fake {
            async fn open_file(
                &self,
                #[zbus(header)] header: zbus::message::Header<'_>,
                #[zbus(connection)] conn: &zbus::Connection,
                _parent: String,
                _title: String,
                options: HashMap<String, OwnedValue>,
            ) -> zbus::zvariant::OwnedObjectPath {
                let sender = header.sender().unwrap().to_string();
                let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
                let path = format!(
                    "/org/freedesktop/portal/desktop/request/{}/{token}",
                    sender.trim_start_matches(':').replace('.', "_")
                );
                let results =
                    HashMap::from([("uris", Value::from(vec!["file:///tmp/gelabber%20bild.png"]))]);
                conn.emit_signal(
                    Some(sender.as_str()),
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0u32, results),
                )
                .await
                .unwrap();
                zbus::zvariant::ObjectPath::try_from(path).unwrap().into()
            }
        }
        let Some((_daemon, address)) = crate::platform::testing::private_bus("files-portal") else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let _service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Fake)
            .unwrap()
            .build()
            .unwrap();
        let choosers = Choosers {
            bus: Some(address),
            portal: true,
            ..nothing()
        };
        assert_eq!(
            choose(&choosers, "Bild", &["*.png"]),
            Choice::Chosen("/tmp/gelabber bild.png".into())
        );
    }

    /// The in-app field: it appears, takes a path and fills the slot.
    #[test]
    fn the_window_asks_for_a_path() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let file = std::env::temp_dir().join(format!("gelabber-pick-{}.png", std::process::id()));
        std::fs::write(&file, b"png").unwrap();
        let slot: Picked = Arc::default();
        let shared = slot.clone();
        let mut asked = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_context_setup(|ctx| {
                crate::fonts::install(ctx);
                set_choosers(ctx, nothing());
            })
            .build_ui(move |ui| {
                if !asked {
                    asked = true;
                    pick(ui.ctx(), "Bild hochladen", &["*.png"], shared.clone());
                }
                show_in_app(ui.ctx());
            });
        let start = std::time::Instant::now();
        while harness.query_by_label("Dateipfad").is_none() {
            assert!(start.elapsed().as_secs() < 10, "no path field");
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let field = harness
            .query_all_by_label("Dateipfad")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .unwrap();
        field.focus();
        harness.step();
        harness
            .query_all_by_label("Dateipfad")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .unwrap()
            .type_text(file.to_str().unwrap());
        harness.step();
        harness.get_by_label("Öffnen").click();
        harness.run_steps(3);
        let picked = slot.lock().clone();
        let _ = std::fs::remove_file(&file);
        let (bytes, name) = picked.flatten().expect("the file");
        assert_eq!(bytes, b"png");
        assert!(name.starts_with("gelabber-pick-"));
        assert!(harness.query_by_label("Dateipfad").is_none());
    }
}
