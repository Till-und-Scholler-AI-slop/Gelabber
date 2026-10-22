//! The system's own notifications for the page's message notifications:
//! Windows toasts (`notify/toast.rs`) and, on Linux, the desktop's
//! notification service on the session bus (`notify/dbus.rs`; mako on
//! Omarchy/Hyprland, KDE, GNOME, dunst, swaync).
//!
//! The web client decides what is announced and how it reads, as it does for
//! a browser: level and mute of the conversation, whether the window is in
//! front, and whether a notification may show the message at all (#268). It
//! hands over a tag (one notification per account and conversation; a newer
//! one replaces the older), a title and a text. A click brings the window to
//! the front and tells the page through the channel it gave, which then
//! opens the conversation. The notifications stay silent: the page plays its
//! own cue, as set in its settings.
//!
//! What the system shows belongs to this run of the app: a click once the app
//! has ended could not bring anything back. The app takes them down when it
//! ends, and on Windows also when it starts (a run that crashed). A page load
//! keeps them; a click on one of an earlier page only brings the window back.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use serde::Serialize;
use tauri::{AppHandle, Manager, ipc::Channel};

#[cfg(target_os = "linux")]
mod dbus;
#[cfg(windows)]
mod toast;

#[cfg(target_os = "linux")]
use dbus::Backend;
#[cfg(windows)]
use toast::Backend;

/// Longest tag the page may give: the web client's are
/// `gelabber:<user>:<conversation>`, ids of 36 characters.
const TAG_MAX: usize = 200;
/// Characters of a title or text beyond this are cut; the web client's
/// previews stay well below.
const TEXT_MAX: usize = 400;

/// What the page hears about a notification it showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    /// The user clicked it; the window is in front again.
    Click,
}

/// One notification as the page words it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub tag: String,
    pub title: String,
    pub body: String,
}

impl Notice {
    fn new(tag: String, title: String, body: String) -> Result<Self, String> {
        if tag.is_empty() || tag.len() > TAG_MAX {
            return Err("notification tag missing or too long".into());
        }
        let cut = |text: String| -> String {
            match text.char_indices().nth(TEXT_MAX) {
                Some((end, _)) => format!("{}…", &text[..end]),
                None => text,
            }
        };
        Ok(Self {
            tag,
            title: cut(title),
            body: cut(body),
        })
    }
}

/// Whether a shown notification goes with a close of `tag`: the tag itself,
/// or with `prefix` every tag that starts with it (all of an account's).
pub fn matches(shown: &str, tag: &str, prefix: bool) -> bool {
    if prefix {
        shown.starts_with(tag)
    } else {
        shown == tag
    }
}

/// What a backend reports about its notifications.
pub enum Report {
    /// The user clicked the notification with this tag. On Wayland the
    /// desktop hands over a token that lets the window come to the front.
    Clicked {
        tag: String,
        #[cfg_attr(windows, allow(dead_code))]
        activation: Option<String>,
    },
}

#[derive(Default)]
pub struct Notifications {
    backend: OnceLock<Result<Backend, String>>,
    /// The page's channel for each tag it showed, until a click or a page load.
    listeners: Mutex<HashMap<String, Channel<Event>>>,
}

impl Notifications {
    fn backend(&self) -> Result<&Backend, String> {
        match self.backend.get() {
            Some(Ok(backend)) => Ok(backend),
            Some(Err(reason)) => Err(reason.clone()),
            None => Err("notifications not started".into()),
        }
    }

    /// A new page cannot hear the old page's channels.
    pub fn reset(&self) {
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.clear();
        }
    }

    /// Takes down everything this run showed; when the app ends.
    pub fn clear(&self) {
        if let Ok(backend) = self.backend() {
            backend.clear();
        }
    }
}

/// Connects to the system's notifications, once, from the app's setup.
pub fn start(app: &AppHandle) {
    let notifications = app.state::<Notifications>();
    let handle = app.clone();
    let report = move |report: Report| reported(&handle, report);
    let backend = notifications
        .backend
        .get_or_init(|| start_backend(app, Box::new(report)));
    if let Err(reason) = backend {
        eprintln!("gelabber: notifications: {reason}");
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn start_backend(
    app: &AppHandle,
    report: Box<dyn Fn(Report) + Send + Sync>,
) -> Result<Backend, String> {
    Backend::start(app, report)
}

/// Neither Windows nor Linux: the page keeps the webview's notifications.
#[cfg(not(any(windows, target_os = "linux")))]
type Backend = Unsupported;

#[cfg(not(any(windows, target_os = "linux")))]
pub struct Unsupported;

#[cfg(not(any(windows, target_os = "linux")))]
impl Unsupported {
    fn available(&self) -> bool {
        false
    }
    fn show(&self, _: Notice) -> Result<(), String> {
        Err("no system notifications here".into())
    }
    fn close(&self, _: &str, _: bool) {}
    fn clear(&self) {}
}

#[cfg(not(any(windows, target_os = "linux")))]
fn start_backend(_: &AppHandle, _: Box<dyn Fn(Report) + Send + Sync>) -> Result<Backend, String> {
    Err("no system notifications here".into())
}

fn reported(app: &AppHandle, report: Report) {
    match report {
        Report::Clicked { tag, activation } => {
            let listener = app
                .state::<Notifications>()
                .listeners
                .lock()
                .ok()
                .and_then(|mut listeners| listeners.remove(&tag));
            let handle = app.clone();
            // Windows report from a thread of the system's, Linux from the
            // bus thread; the window wants the main thread.
            let _ = app.run_on_main_thread(move || {
                #[cfg(target_os = "linux")]
                if let Some(token) = activation {
                    dbus::activate(&handle, &token);
                }
                #[cfg(not(target_os = "linux"))]
                let _ = activation;
                crate::shell::show(&handle);
                if let Some(listener) = listener {
                    let _ = listener.send(Event::Click);
                }
            });
        }
    }
}

#[derive(Serialize)]
pub struct NotifyInfo {
    /// The page shows its message notifications through `notify_show`.
    /// Without, it keeps the webview's web notifications.
    available: bool,
}

/// Whether the system takes notifications. On Linux the first answer waits
/// for the notification service (it may start on demand), a few seconds at
/// most.
#[tauri::command]
pub async fn notify_info(app: AppHandle) -> NotifyInfo {
    let available = tauri::async_runtime::spawn_blocking(move || {
        app.state::<Notifications>()
            .backend()
            .is_ok_and(|backend| backend.available())
    })
    .await
    .unwrap_or(false);
    NotifyInfo { available }
}

/// Shows a notification, or replaces the one with the same tag. A click
/// arrives through `events`.
#[tauri::command]
pub fn notify_show(
    app: AppHandle,
    tag: String,
    title: String,
    body: String,
    events: Channel<Event>,
) -> Result<(), String> {
    let notice = Notice::new(tag, title, body)?;
    let notifications = app.state::<Notifications>();
    let backend = notifications.backend()?;
    if let Ok(mut listeners) = notifications.listeners.lock() {
        listeners.insert(notice.tag.clone(), events);
    }
    backend.show(notice)
}

/// Takes down the notification with `tag`, or with `prefix` every one whose
/// tag starts with it: a conversation that is being read, an account that
/// signed out.
#[tauri::command]
pub fn notify_close(app: AppHandle, tag: String, prefix: bool) {
    if tag.is_empty() && !prefix {
        return;
    }
    let notifications = app.state::<Notifications>();
    if let Ok(mut listeners) = notifications.listeners.lock() {
        listeners.retain(|shown, _| !matches(shown, &tag, prefix));
    }
    if let Ok(backend) = notifications.backend() {
        backend.close(&tag, prefix);
    }
}

/// The text of a notification for a service that reads markup, which the
/// specification allows only in the body: `&`, `<` and `>` stand for
/// themselves.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Text inside XML (the toast's template): also quotes, for attributes.
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Not allowed in XML 1.0 at all; the template would not load.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            other => out.push(other),
        }
    }
    out
}

/// A toast's template (Windows): the title in bold, the text below, which a
/// notification that hides the message leaves out. Silent, as on Linux.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn toast_xml(title: &str, body: &str) -> String {
    let body = if body.is_empty() {
        String::new()
    } else {
        format!("<text>{}</text>", escape_xml(body))
    };
    format!(
        concat!(
            r#"<toast><visual><binding template="ToastGeneric">"#,
            r#"<text hint-maxLines="2">{}</text>{}"#,
            r#"</binding></visual><audio silent="true"/></toast>"#
        ),
        escape_xml(title),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::{Notice, TEXT_MAX, escape_markup, escape_xml, matches, toast_xml};
    use crate::media::NOTIFY_COMMANDS;

    /// The server origin's `notify` set grants exactly these commands.
    #[test]
    fn notify_permission_set_matches_the_commands() {
        let set = include_str!("../permissions/notify.toml");
        let granted: Vec<&str> = set
            .lines()
            .filter_map(|line| line.trim().strip_prefix("\"allow-"))
            .filter_map(|rest| rest.strip_suffix("\","))
            .collect();
        let expected: Vec<String> = NOTIFY_COMMANDS
            .iter()
            .map(|c| c.replace('_', "-"))
            .collect();
        assert_eq!(granted, expected);
        let main = include_str!("main.rs");
        for command in NOTIFY_COMMANDS {
            assert!(
                main.contains(&format!("notify::{command},")),
                "{command} missing from generate_handler!"
            );
        }
        assert!(main.contains(".permission(\"notify\")"));
    }

    #[test]
    fn a_close_takes_the_tag_or_everything_under_it() {
        let a = "gelabber:u1:c1";
        assert!(matches(a, "gelabber:u1:c1", false));
        assert!(!matches(a, "gelabber:u1:c", false));
        assert!(matches(a, "gelabber:u1:", true));
        assert!(!matches(a, "gelabber:u2:", true));
    }

    #[test]
    fn notices_need_a_tag_and_long_texts_are_cut() {
        assert!(Notice::new(String::new(), "t".into(), "b".into()).is_err());
        assert!(Notice::new("x".repeat(201), "t".into(), "b".into()).is_err());
        let long = "ä".repeat(TEXT_MAX + 10);
        let notice = Notice::new("tag".into(), long.clone(), "kurz".into()).unwrap();
        assert_eq!(notice.title.chars().count(), TEXT_MAX + 1);
        assert!(notice.title.ends_with('…'));
        assert_eq!(notice.body, "kurz");
    }

    #[test]
    fn message_text_cannot_become_markup() {
        assert_eq!(
            escape_markup("<b>Tom & Jerry</b>"),
            "&lt;b&gt;Tom &amp; Jerry&lt;/b&gt;"
        );
        assert_eq!(escape_markup("\"so\""), "\"so\"");
        assert_eq!(
            escape_xml("<a href='x'>\"&\u{1}</a>"),
            "&lt;a href=&apos;x&apos;&gt;&quot;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn toasts_carry_the_text_escaped_and_no_sound() {
        assert_eq!(
            toast_xml("Anna · #a&b", "<hi>"),
            concat!(
                r#"<toast><visual><binding template="ToastGeneric">"#,
                r#"<text hint-maxLines="2">Anna · #a&amp;b</text><text>&lt;hi&gt;</text>"#,
                r#"</binding></visual><audio silent="true"/></toast>"#
            )
        );
        assert!(!toast_xml("Neue Nachricht von Anna", "").contains("<text></text>"));
    }
}
