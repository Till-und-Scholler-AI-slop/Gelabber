//! System notifications for new messages (web messages/useMessageToasts.ts,
//! the desktop half): D-Bus on Linux (`dbus.rs`), toasts on Windows
//! (`toast.rs`), nothing elsewhere. The rules are the web's (`rules.rs`); a
//! click brings the window to the front and opens the conversation.

#[cfg(target_os = "linux")]
mod dbus;
pub mod rules;
#[cfg(windows)]
mod toast;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use gelabber_client::{Client, EventFrame, EventKind, EventTap, State};
use parking_lot::Mutex;
use uuid::Uuid;

use rules::{Dedupe, Pacing, Setting};

/// Longest tag and text, as desktop/app/src/notify.rs caps them.
const TAG_MAX: usize = 200;
const TEXT_MAX: usize = 400;
/// Events kept while the UI thread is busy; a flood beyond is dropped.
const INBOX_MAX: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub tag: String,
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(tag: String, title: String, body: String) -> Option<Self> {
        if tag.is_empty() || tag.len() > TAG_MAX {
            return None;
        }
        let cut = |text: String| match text.char_indices().nth(TEXT_MAX) {
            Some((end, _)) => format!("{}…", &text[..end]),
            None => text,
        };
        Some(Self {
            tag,
            title: cut(title),
            body: cut(body),
        })
    }
}

/// What a backend reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// A click on the notification with this tag; on Wayland with the
    /// desktop's activation token.
    Clicked {
        tag: String,
        activation: Option<String>,
    },
}

pub type Callback = Arc<dyn Fn(Report) + Send + Sync>;

pub trait Backend: Send + Sync {
    /// Whether notifications can be shown; may wait a moment for the service.
    fn available(&self) -> bool;
    /// The same without waiting; `None` while unknown.
    fn known(&self) -> Option<bool>;
    fn show(&self, notice: Notice) -> Result<(), String>;
    fn close(&self, tag: &str, prefix: bool);
    /// Takes down everything this run showed (the app ends).
    fn clear(&self);
}

/// Whether a shown tag goes with a close of `tag` (or all under it).
pub fn matches(shown: &str, tag: &str, prefix: bool) -> bool {
    if prefix {
        shown.starts_with(tag)
    } else {
        shown == tag
    }
}

/// Body text for a service that reads markup.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Text inside the toast's XML, attributes included.
pub fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            other => out.push(other),
        }
    }
    out
}

/// A toast's template (Windows): bold title, the text below, silent.
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

/// The platform's notifications, or none.
pub fn start_backend(report: Callback) -> Option<Arc<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        Some(Arc::new(dbus::Backend::start(None, report)))
    }
    #[cfg(windows)]
    {
        match toast::Backend::start(report) {
            Ok(b) => Some(Arc::new(b)),
            Err(e) => {
                tracing::warn!("notifications: {e}");
                None
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = report;
        None
    }
}

/// What the app does after a pass of the notifier.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// A notification was clicked: the window comes to the front.
    pub focus: bool,
    /// Its conversation, for this account.
    pub navigate: Option<String>,
    /// A direct message while the window is not in front: the taskbar
    /// button flashes (`requestDesktopAttention`).
    pub attention: bool,
    /// A notification was shown: the message cue plays once for a burst.
    pub sound: bool,
}

/// What a pass needs to know about the window and the app.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// Focused, not minimized and not hidden in the tray.
    pub in_front: bool,
    pub desktop_enabled: bool,
    pub now_ms: i64,
    pub server_now: DateTime<Utc>,
}

pub struct Notifier {
    backend: Option<Arc<dyn Backend>>,
    inbox: Arc<Mutex<Vec<EventFrame>>>,
    clicks: Arc<Mutex<Vec<Report>>>,
    /// tag → (account, conversation path) of what was shown.
    targets: HashMap<String, (Uuid, String)>,
    account: Option<Uuid>,
    dedupe: Dedupe,
    pacing: Pacing,
    sound: Pacing,
    attention: Pacing,
    /// The conversation whose notifications were last taken down because
    /// it was read.
    silenced: Option<Uuid>,
    hid_content: bool,
}

impl Notifier {
    /// `repaint` wakes the UI thread when something arrives.
    pub fn new(
        backend: impl FnOnce(Callback) -> Option<Arc<dyn Backend>>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let clicks: Arc<Mutex<Vec<Report>>> = Arc::default();
        let sink = clicks.clone();
        let backend = backend(Arc::new(move |report| {
            sink.lock().push(report);
            repaint();
        }));
        Self {
            backend,
            inbox: Arc::default(),
            clicks,
            targets: HashMap::new(),
            account: None,
            dedupe: Dedupe::default(),
            pacing: Pacing::default(),
            sound: Pacing::default(),
            attention: Pacing::default(),
            silenced: None,
            hid_content: false,
        }
    }

    pub fn backend(&self) -> Option<&Arc<dyn Backend>> {
        self.backend.as_ref()
    }

    /// For [`Client::set_event_tap`]: chat creates go to the inbox.
    pub fn tap(&self, repaint: Arc<dyn Fn() + Send + Sync>) -> EventTap {
        let inbox = self.inbox.clone();
        Arc::new(move |event: &EventFrame| {
            if event.t != EventKind::C || event.c.is_none() {
                return;
            }
            let mut inbox = inbox.lock();
            if inbox.len() < INBOX_MAX {
                inbox.push(event.clone());
                repaint();
            }
        })
    }

    /// One pass on the UI thread.
    pub fn frame(&mut self, client: &Client, cx: Context) -> Outcome {
        let state = client.state();
        self.pass(&state, cx)
    }

    fn pass(&mut self, state: &State, cx: Context) -> Outcome {
        let mut out = Outcome::default();
        let events = std::mem::take(&mut *self.inbox.lock());
        let clicks = std::mem::take(&mut *self.clicks.lock());
        let me = state.me().map(|u| u.id);
        if me != self.account {
            // An account takes its notifications with it.
            if let (Some(old), Some(backend)) = (self.account, &self.backend) {
                backend.close(&rules::tag_of(old, None), true);
            }
            self.targets.retain(|_, (user, _)| Some(*user) == me);
            self.account = me;
            self.dedupe = Dedupe::default();
            self.pacing = Pacing::default();
            self.silenced = None;
            self.hid_content = false;
        }
        for Report::Clicked { tag, .. } in clicks {
            out.focus = true;
            if let Some((user, path)) = self.targets.remove(&tag)
                && Some(user) == me
            {
                out.navigate = Some(path);
            }
        }
        let Some(me) = me else {
            return out;
        };
        let hide = hides_content(state);
        if hide && !self.hid_content {
            // Once switched on, what shows the message already goes.
            if let Some(backend) = &self.backend {
                backend.close(&rules::tag_of(me, None), true);
            }
        }
        self.hid_content = hide;
        // The conversation being read takes its notification down.
        let reading = state.view.channel.filter(|_| cx.in_front);
        if reading != self.silenced {
            if let (Some(channel), Some(backend)) = (reading, &self.backend) {
                backend.close(&rules::tag_of(me, Some(channel)), false);
                self.targets.remove(&rules::tag_of(me, Some(channel)));
            }
            self.silenced = reading;
        }
        for event in events {
            if let Some(notice) = self.decide(state, me, &event, cx, &mut out) {
                let shown = match &self.backend {
                    Some(backend) => backend.show(notice.0.clone()).is_ok(),
                    None => false,
                };
                if shown {
                    self.targets.insert(notice.0.tag, (me, notice.1));
                    if self.sound.take(Uuid::nil(), cx.now_ms) {
                        out.sound = true;
                    }
                }
            }
        }
        out
    }

    /// The notice for one event, with where its click leads.
    fn decide(
        &mut self,
        state: &State,
        me: Uuid,
        event: &EventFrame,
        cx: Context,
        out: &mut Outcome,
    ) -> Option<(Notice, String)> {
        let channel = event.c?;
        let message = event.message()?;
        if !self.dedupe.first(channel, message.id) {
            return None;
        }
        let own = message.author.id == me;
        let dm = event.s == channel;
        let mentioned = !own && (message.mention_everyone || message.mentions.contains(&me));
        let setting = state
            .read
            .get(&channel)
            .map(Setting::of_row)
            .unwrap_or_else(|| {
                rules::conversation_setting(
                    &state.notification_settings,
                    channel,
                    (!dm).then_some(event.s),
                    cx.server_now,
                )
            });
        if !rules::wants_notification(Some(setting), dm, mentioned, cx.server_now) {
            return None;
        }
        let live = (cx.server_now - message.created_at).num_milliseconds() < rules::LIVE_WINDOW_MS;
        if dm && !own && !cx.in_front && live && self.attention.take(Uuid::nil(), cx.now_ms) {
            out.attention = true;
        }
        if !rules::wants_desktop(cx.desktop_enabled, cx.in_front, own) || !live {
            return None;
        }
        if !self.pacing.take(channel, cx.now_ms) {
            return None;
        }
        let label = label(state, event.s, channel, dm);
        let system = message.system.as_ref().map(|s| {
            if s.kind == "live_started" {
                let voice = s
                    .params
                    .get("voice_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                format!("{} ist live in {voice}.", message.author.name)
            } else {
                format!("Systemnachricht von {}", message.author.name)
            }
        });
        let text = system.unwrap_or_else(|| {
            rules::readable_mentions(&message.content, &names(state, event.s, channel, dm))
        });
        let preview = rules::preview_text(
            &text,
            !message.attachments.is_empty(),
            message.gif.is_some(),
        );
        let (title, body) = rules::notification_text(
            hides_content(state),
            &message.author.name,
            mentioned,
            &label,
            &preview,
        );
        let tag = rules::tag_of(me, Some(channel));
        let path = rules::conversation_path(dm, event.s, channel);
        Notice::new(tag, title, body).map(|n| (n, path))
    }

    /// Takes down everything this run showed (the app ends).
    pub fn clear(&self) {
        if let Some(backend) = &self.backend {
            backend.clear();
        }
    }
}

/// `hidesContent`: unknown counts as hidden.
fn hides_content(state: &State) -> bool {
    state.notification_privacy.is_none_or(|p| p.hide_content)
}

/// `channelLabel`.
fn label(state: &State, server: Uuid, channel: Uuid, dm: bool) -> String {
    if dm {
        return state
            .dms
            .iter()
            .find(|d| d.id == channel)
            .map(|d| d.peer.name.clone())
            .unwrap_or_else(|| "DM".into());
    }
    state
        .server_details
        .get(&server)
        .and_then(|d| d.channels.iter().find(|c| c.id == channel))
        .map(|c| format!("#{}", c.name))
        .unwrap_or_else(|| "Kanal".into())
}

/// `conversationMembers`: names for the mention tokens of a preview.
fn names(state: &State, server: Uuid, channel: Uuid, dm: bool) -> HashMap<String, String> {
    let mut names = HashMap::new();
    if dm {
        if let Some(me) = state.me() {
            names.insert(me.id.to_string(), me.name.clone());
        }
        if let Some(d) = state.dms.iter().find(|d| d.id == channel) {
            names.insert(d.peer.id.to_string(), d.peer.name.clone());
        }
    } else if let Some(detail) = state.server_details.get(&server) {
        for m in &detail.members {
            names.insert(m.user_id.to_string(), m.name.clone());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_client::{NotificationPrivacy, NotificationSetting, Session, SettingScope, User};
    use serde_json::json;

    /// Records what it is asked to show and close.
    #[derive(Default)]
    struct Fake {
        shown: Mutex<Vec<Notice>>,
        closed: Mutex<Vec<(String, bool)>>,
    }

    impl Backend for Fake {
        fn available(&self) -> bool {
            true
        }
        fn known(&self) -> Option<bool> {
            Some(true)
        }
        fn show(&self, notice: Notice) -> Result<(), String> {
            self.shown.lock().push(notice);
            Ok(())
        }
        fn close(&self, tag: &str, prefix: bool) {
            self.closed.lock().push((tag.to_owned(), prefix));
        }
        fn clear(&self) {}
    }

    fn state(me: Uuid) -> State {
        let user: User = serde_json::from_value(json!({
            "id": me, "email": "lena@example.org", "name": "Lena",
            "created_at": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        let mut state = State::default();
        state.session = Session::Authenticated(Box::new(user));
        state.notification_privacy = Some(NotificationPrivacy {
            hide_content: false,
        });
        state
    }

    fn created(
        server: Uuid,
        channel: Uuid,
        author: Uuid,
        content: &str,
        mentions: &[Uuid],
    ) -> EventFrame {
        let message = json!({
            "id": Uuid::new_v4(), "channel_id": channel,
            "author": {"id": author, "name": "Anna", "avatar_url": null},
            "content": content, "created_at": Utc::now(), "edited_at": null,
            "revision": 1, "created_order": 1, "mentions": mentions,
        });
        EventFrame {
            t: EventKind::C,
            s: server,
            c: Some(channel),
            n: 1,
            ep: None,
            i: None,
            d: Some(message),
            r: None,
        }
    }

    fn cx(in_front: bool) -> Context {
        Context {
            in_front,
            desktop_enabled: true,
            now_ms: 0,
            server_now: Utc::now(),
        }
    }

    #[test]
    fn a_foreign_message_notifies_once_while_the_window_is_away_and_a_click_opens_it() {
        let fake = Arc::new(Fake::default());
        let backend: Arc<dyn Backend> = fake.clone();
        let clicks: Arc<Mutex<Option<Callback>>> = Arc::default();
        let keep = clicks.clone();
        let mut notifier = Notifier::new(
            move |report| {
                *keep.lock() = Some(report);
                Some(backend)
            },
            Arc::new(|| {}),
        );
        let (me, anna, server, channel) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let mut st = state(me);
        let tap = notifier.tap(Arc::new(|| {}));
        let first = created(
            server,
            channel,
            anna,
            "hallo <@00000000-0000-0000-0000-000000000000>",
            &[],
        );
        tap(&first);
        tap(&first);
        // In front: nothing; then away.
        let out = notifier.pass(&st, cx(true));
        assert!(fake.shown.lock().is_empty() && !out.sound);
        tap(&created(server, channel, anna, "noch mal", &[me]));
        let out = notifier.pass(&st, cx(false));
        let shown = fake.shown.lock().clone();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Anna hat dich erwähnt · Kanal");
        assert_eq!(shown[0].body, "noch mal");
        assert_eq!(shown[0].tag, format!("gelabber:{me}:{channel}"));
        assert!(out.sound && !out.attention);
        // Five seconds per conversation; own messages never.
        tap(&created(server, channel, anna, "und noch", &[]));
        tap(&created(server, Uuid::new_v4(), me, "meins", &[]));
        notifier.pass(&st, cx(false));
        assert_eq!(fake.shown.lock().len(), 1);
        // The click brings the window and the conversation.
        let report = clicks.lock().clone().unwrap();
        report(Report::Clicked {
            tag: shown[0].tag.clone(),
            activation: None,
        });
        let out = notifier.pass(&st, cx(false));
        assert!(out.focus);
        assert_eq!(out.navigate, Some(format!("/s/{server}/c/{channel}")));
        // Reading the conversation takes its notification down.
        st.view.channel = Some(channel);
        notifier.pass(&st, cx(true));
        assert!(fake.closed.lock().contains(&(shown[0].tag.clone(), false)));
    }

    #[test]
    fn mutes_hidden_content_and_direct_messages() {
        let fake = Arc::new(Fake::default());
        let backend: Arc<dyn Backend> = fake.clone();
        let mut notifier = Notifier::new(move |_| Some(backend), Arc::new(|| {}));
        let (me, anna, server, channel) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let mut st = state(me);
        st.notification_settings = vec![NotificationSetting {
            scope: SettingScope::Server,
            id: server,
            level: None,
            muted: true,
            muted_until: None,
        }];
        let tap = notifier.tap(Arc::new(|| {}));
        tap(&created(server, channel, anna, "leise", &[]));
        notifier.pass(&st, cx(false));
        assert!(fake.shown.lock().is_empty());
        // A DM, with the content hidden: who wrote, nothing more, and the
        // taskbar asks for attention.
        st.notification_privacy = Some(NotificationPrivacy { hide_content: true });
        let dm = Uuid::new_v4();
        tap(&created(dm, dm, anna, "geheim", &[]));
        let out = notifier.pass(&st, cx(false));
        let shown = fake.shown.lock().clone();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Neue Nachricht von Anna");
        assert_eq!(shown[0].body, "");
        assert!(out.attention);
        // Switching it on took down what showed the message before.
        assert!(
            fake.closed
                .lock()
                .contains(&(format!("gelabber:{me}:"), true))
        );
        // Signed out: the account's notifications go.
        st.session = Session::Anonymous;
        notifier.pass(&st, cx(false));
        assert_eq!(
            fake.closed.lock().last().unwrap(),
            &(format!("gelabber:{me}:"), true)
        );
    }

    #[test]
    fn notices_need_a_tag_and_long_texts_are_cut() {
        assert!(Notice::new(String::new(), "t".into(), "b".into()).is_none());
        assert!(Notice::new("x".repeat(201), "t".into(), "b".into()).is_none());
        let n = Notice::new("tag".into(), "ä".repeat(TEXT_MAX + 10), "kurz".into()).unwrap();
        assert_eq!(n.title.chars().count(), TEXT_MAX + 1);
        assert!(n.title.ends_with('…'));
    }

    #[test]
    fn message_text_cannot_become_markup() {
        assert_eq!(
            escape_markup("<b>Tom & Jerry</b>"),
            "&lt;b&gt;Tom &amp; Jerry&lt;/b&gt;"
        );
        assert_eq!(
            toast_xml("Anna · #a&b", "<hi>"),
            concat!(
                r#"<toast><visual><binding template="ToastGeneric">"#,
                r#"<text hint-maxLines="2">Anna · #a&amp;b</text><text>&lt;hi&gt;</text>"#,
                r#"</binding></visual><audio silent="true"/></toast>"#
            )
        );
        assert!(!toast_xml("Neue Nachricht von Anna", "").contains("<text></text>"));
        assert!(matches("gelabber:u1:c1", "gelabber:u1:", true));
        assert!(!matches("gelabber:u1:c1", "gelabber:u1:", false));
    }
}
