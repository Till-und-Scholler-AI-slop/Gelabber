//! The desktop around the window, what the Tauri app (up to 0.8) did
//! outside its page: persistence in its config directory, system
//! notifications, the tray with close-to-tray, one app per user, the
//! global push-to-talk key, and the update check. The binary owns one
//! [`Platform`] and calls it every pass; parity runs go without one.

pub mod aumid;
pub mod files;
pub mod instance;
pub mod legacy;
pub mod notify;
mod open;
pub mod paths;
pub mod ptt;
#[cfg(target_os = "linux")]
pub mod scheme;
pub mod session;
pub mod settings;
pub mod state;
pub mod tray;
pub mod update;
pub mod webview;

pub use open::{open_url, open_url_logged};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{Context, Id, ViewportCommand};
use gelabber_client::Client;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::GelabberApp;
use crate::screens::settings::prefs;
use crate::screens::voice::call_bar::{PttSource, push_to_talk_from};
use crate::screens::{chat, shell};

/// The desktop entry's and the window's id (Wayland app id, notification
/// `desktop-entry`, portal registration, tray id).
pub const APP_ID: &str = "gelabber";

/// What the settings page shows only in the desktop app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopInfo {
    /// A tray is on screen: closing can hide the window there.
    pub tray: bool,
    pub close_to_tray: bool,
    /// The system shows the notifications (no browser permission).
    pub system_notifications: bool,
}

fn info_id() -> Id {
    Id::unique("gelabber-desktop-info")
}

/// `None` outside the desktop app (parity runs, tests).
pub fn desktop_info(ctx: &Context) -> Option<DesktopInfo> {
    ctx.data(|d| d.get_temp::<Option<DesktopInfo>>(info_id()))
        .flatten()
}

/// Tests: the desktop app's facts as [`Platform::logic`] publishes them.
#[cfg(test)]
pub(crate) fn show_desktop_info(ctx: &Context, info: Option<DesktopInfo>) {
    ctx.data_mut(|d| d.insert_temp(info_id(), info));
}

/// `close_to_tray` of desktop.json, as the window's close reads it.
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(false);

/// The settings switch "Fenster schließen minimiert in den Infobereich".
pub fn set_close_to_tray(ctx: &Context, enabled: bool) -> Result<(), String> {
    settings::set_close_to_tray(enabled)?;
    CLOSE_TO_TRAY.store(enabled, Ordering::Relaxed);
    ctx.data_mut(|d| {
        if let Some(info) = d
            .get_temp_mut_or_default::<Option<DesktopInfo>>(info_id())
            .as_mut()
        {
            info.close_to_tray = enabled;
        }
    });
    Ok(())
}

/// The global push-to-talk key as the call sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PttStatus {
    /// Works outside the window; `trigger` as the desktop names it.
    Bound { trigger: Option<String> },
    /// Only inside the window, and why.
    Failed { reason: String },
}

fn ptt_id() -> Id {
    Id::unique("gelabber-ptt-status")
}

/// `None` while no global key is wanted (not in a call, voice activity).
pub fn ptt_status(ctx: &Context) -> Option<PttStatus> {
    ctx.data(|d| d.get_temp::<Option<PttStatus>>(ptt_id()))
        .flatten()
}

type Wake = Arc<Mutex<Option<Context>>>;

fn waker(slot: &Wake) -> Arc<dyn Fn() + Send + Sync> {
    let slot = slot.clone();
    Arc::new(move || {
        if let Some(ctx) = slot.lock().as_ref() {
            ctx.request_repaint();
        }
    })
}

pub struct Platform {
    wake: Wake,
    instance: Option<instance::Instance>,
    tray: Option<tray::Tray>,
    tray_events: tray::Events,
    notifier: notify::Notifier,
    keeper: Arc<Mutex<Option<session::Keeper>>>,
    started: bool,
    hidden: bool,
    /// False on Wayland, where winit can neither hide nor raise the window.
    window_can_hide: bool,
    quitting: bool,
    front: Option<bool>,
    /// Its own clock: logic-only passes (hidden, not composited) keep egui's still.
    next_tick: Option<Instant>,
    stored: state::Stored,
    saved: state::Stored,
    restored_for: Option<Uuid>,
    ptt: PttLink,
    updates: update::Updater,
    /// The window title last sent (`titleWith`).
    title: String,
    /// The title bar's theme last sent.
    title_bar: Option<egui::SystemTheme>,
    /// The last pass showed the setup screen.
    in_setup: bool,
    /// The unread count on the Linux launcher icon.
    #[cfg(target_os = "linux")]
    badge: Option<(tray::unity::Badge, u32)>,
}

impl Platform {
    /// Before the window opens; `instance` keeps other starts out.
    pub fn new(instance: Option<instance::Instance>) -> Self {
        let wake: Wake = Arc::default();
        let notifier = notify::Notifier::new(notify::start_backend, waker(&wake));
        let stored = state::load();
        CLOSE_TO_TRAY.store(settings::close_to_tray(), Ordering::Relaxed);
        Self {
            instance,
            tray: None,
            tray_events: Arc::default(),
            notifier,
            keeper: Arc::default(),
            started: false,
            hidden: false,
            window_can_hide: true,
            quitting: false,
            front: None,
            next_tick: None,
            saved: stored.clone(),
            stored,
            restored_for: None,
            ptt: PttLink::default(),
            updates: update::Updater::default(),
            title: "Gelabber".into(),
            title_bar: None,
            in_setup: false,
            #[cfg(target_os = "linux")]
            badge: None,
            wake,
        }
    }

    /// For the launcher: each new client gets the remembered sign-in and
    /// reports its chat events, before its first request.
    pub fn prepare_hook(&self) -> Box<dyn Fn(&Client)> {
        let keeper = self.keeper.clone();
        let tap = self.notifier.tap(waker(&self.wake));
        Box::new(move |client: &Client| {
            *keeper.lock() = Some(session::Keeper::restore(client));
            client.set_event_tap(Some(tap.clone()));
        })
    }

    /// Once the window exists. On Wayland winit 0.30 ignores hiding, un-minimizing
    /// and focus, so a window closed to the tray would stay and never return.
    pub fn set_window(&mut self, cc: &eframe::CreationContext<'_>) {
        use eframe::egui_wgpu::wgpu::rwh::{HasDisplayHandle, RawDisplayHandle};
        let wayland = matches!(
            cc.display_handle().map(|h| h.as_raw()),
            Ok(RawDisplayHandle::Wayland(_))
        );
        self.window_can_hide = !wayland;
    }

    fn start(&mut self, ctx: &Context) {
        self.started = true;
        *self.wake.lock() = Some(ctx.clone());
        #[cfg(target_os = "linux")]
        {
            self.badge = tray::unity::Badge::start(None).map(|b| (b, u32::MAX));
            // winit reports no light or dark on Linux; "System" and the
            // setup screen follow the portal's preference.
            let scheme = ctx.clone();
            scheme::watch(None, move |dark| crate::theme::set_os_dark(&scheme, dark));
        }
        if let Some(instance) = &self.instance {
            instance.set_wake(waker(&self.wake));
        }
        self.tray = tray::Tray::start(self.tray_events.clone(), waker(&self.wake));
        self.updates.start(Instant::now());
    }

    fn show_window(&mut self, ctx: &Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// Every pass, also while the window is hidden in the tray.
    pub fn logic(&mut self, ctx: &Context, app: Option<&mut GelabberApp>) {
        if !self.started {
            self.start(ctx);
        }
        if self.instance.as_ref().is_some_and(|i| i.take_show()) {
            self.show_window(ctx);
        }
        let events = std::mem::take(&mut *self.tray_events.lock());
        for event in events {
            match event {
                tray::Event::Show => self.show_window(ctx),
                tray::Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        // Close-to-tray needs a tray and a window that can hide and return.
        let tray_shown =
            self.window_can_hide && self.tray.as_ref().is_some_and(tray::Tray::visible);
        let (close, focused, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.focused, v.minimized)
        });
        // A call keeps running in the hidden window.
        if close && !self.quitting && CLOSE_TO_TRAY.load(Ordering::Relaxed) && tray_shown {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.hidden = true;
        }
        let in_front = focused.unwrap_or(true) && minimized != Some(true) && !self.hidden;
        ctx.data_mut(|d| {
            d.insert_temp(
                info_id(),
                Some(DesktopInfo {
                    tray: tray_shown,
                    close_to_tray: CLOSE_TO_TRAY.load(Ordering::Relaxed),
                    system_notifications: self
                        .notifier
                        .backend()
                        .is_some_and(|b| b.known() == Some(true)),
                }),
            )
        });
        let now = Instant::now();
        // Title bar and decorations in the theme's mode, the setup screen's
        // too (it shows in the active theme, as the sign-in pages).
        let title_bar = if crate::theme::Theme::of(ctx).is_dark() {
            egui::SystemTheme::Dark
        } else {
            egui::SystemTheme::Light
        };
        if self.title_bar != Some(title_bar) {
            self.title_bar = Some(title_bar);
            ctx.send_viewport_cmd(ViewportCommand::SetTheme(title_bar));
        }
        self.updates.setup(ctx);
        self.in_setup = app.is_none();
        let Some(app) = app else {
            self.show_unread(ctx, tray::Unread::default());
            // The call ended with the app: the global key goes with it.
            self.ptt.stop(ctx);
            // "Jetzt installieren" on the setup screen: no call to wait for.
            if self.updates.tick_setup(ctx) {
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        };
        let client = app.client().clone();
        let me = client.with_state(|s| s.me().map(|u| u.id));
        self.restore(ctx, me);
        if self.front != Some(in_front) {
            self.front = Some(in_front);
            client.set_focused(in_front);
        }
        let p = prefs::get(ctx);
        let (joined, deafened) = app
            .call()
            .with_state(|s| (crate::screens::voice::session::joined(s), s.deafened));
        let outcome = self.notifier.frame(
            &client,
            notify::Context {
                in_front,
                desktop_enabled: p.desktop_notify,
                now_ms: state::now_ms(),
                server_now: client.http().server_now(),
                joined,
                deafened,
            },
        );
        if outcome.focus {
            self.show_window(ctx);
        }
        if let Some(path) = outcome.navigate {
            app.navigate(&path);
        }
        if outcome.attention {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
        let call = app.call().clone();
        if outcome.sound && p.message_sound && !call.with_state(|s| s.deafened) {
            call.play_sound(gelabber_call::Sound::Message);
        }
        if outcome.live_sound && p.message_sound && !deafened {
            call.play_sound(gelabber_call::Sound::LiveStart);
        }
        self.ptt.tick(ctx, &call, &p);
        if self.next_tick.is_none_or(|next| now >= next) {
            self.next_tick = Some(now + Duration::from_millis(500));
            self.tick(ctx, &client, in_front);
        }
        // An accepted update: the window closes, main.rs runs the installer.
        if self.updates.tick(&client, now, update::call_busy(&call)) {
            self.quitting = true;
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if !self.hidden {
            ctx.request_repaint_after(Duration::from_millis(500));
        } else {
            // Hidden: the logic pass still runs for tray clicks and events.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// Twice a second: the sign-in, the tray's dot, what is remembered.
    fn tick(&mut self, ctx: &Context, client: &Client, in_front: bool) {
        if let Some(keeper) = self.keeper.lock().as_mut() {
            keeper.tick(client);
        }
        let (me, unread) = client.with_state(|s| {
            let reading = s.view.channel.filter(|_| in_front);
            let now = client.http().server_now();
            let (mut count, mut dot) = (0i64, false);
            for row in s.read.values() {
                let badge = row.badge(reading, now);
                count += badge.count;
                dot |= badge.dot;
            }
            let count = u32::try_from(count.max(0)).unwrap_or(u32::MAX);
            (
                s.me().map(|u| u.id),
                tray::Unread {
                    count,
                    any: count > 0 || dot,
                },
            )
        });
        // Signed out: nothing unread (useUnreadIndicators unmounts).
        let unread = if me.is_some() {
            unread
        } else {
            tray::Unread::default()
        };
        self.show_unread(ctx, unread);
        self.remember(ctx, me);
    }

    /// The tray's dot, the window title and the launcher's count.
    fn show_unread(&mut self, ctx: &Context, unread: tray::Unread) {
        if let Some(tray) = &mut self.tray {
            tray.set_unread(unread);
        }
        let title = unread.title();
        if self.title != title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        #[cfg(target_os = "linux")]
        if let Some((badge, shown)) = &mut self.badge
            && *shown != unread.count
        {
            badge.set(unread.count);
            *shown = unread.count;
        }
    }

    /// Restores an account's memory once, then keeps the file current.
    /// Every pass, before the shell's route effects: "/" must already find
    /// the remembered place on the pass the account becomes known.
    fn restore(&mut self, ctx: &Context, me: Option<Uuid>) {
        if let Some(user) = me
            && self.restored_for != Some(user)
        {
            self.restored_for = Some(user);
            chat::switch_account(ctx, user);
            shell::switch_account(ctx, user);
            if let Some(account) = self.stored.accounts.get(&user) {
                shell::restore_memory(ctx, user, account.memory());
                chat::restore_drafts(ctx, account.fresh_drafts(state::now_ms()));
            }
        }
    }

    fn remember(&mut self, ctx: &Context, me: Option<Uuid>) {
        let now = state::now_ms();
        if let Some(user) = me.filter(|u| self.restored_for == Some(*u)) {
            let account = self.stored.accounts.entry(user).or_default();
            if let Some(memory) = shell::memory_of(ctx, user) {
                account.set_memory(&memory);
            }
            account.set_drafts(&chat::drafts(ctx), now);
        }
        self.track_window(ctx);
        if self.stored != self.saved {
            state::save(&self.stored);
            self.saved = self.stored.clone();
        }
    }

    fn track_window(&mut self, ctx: &Context) {
        let (inner, outer, maximized, fullscreen, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.inner_rect,
                v.outer_rect,
                v.maximized,
                v.fullscreen,
                v.minimized,
            )
        });
        let Some(inner) = inner else { return };
        let mut window = self.stored.window.unwrap_or(state::Window {
            width: state::DEFAULT_SIZE[0],
            height: state::DEFAULT_SIZE[1],
            x: None,
            y: None,
            maximized: false,
        });
        if minimized == Some(true) || fullscreen == Some(true) || self.hidden {
            return;
        }
        window.maximized = maximized == Some(true);
        // egui's points shrink with the text size; the window opens before
        // any zoom is set, so it is stored in the desktop's logical pixels.
        let zoom = ctx.zoom_factor();
        let (width, height) = (inner.width() * zoom, inner.height() * zoom);
        // A maximized window keeps the size it comes back to.
        if !window.maximized && width >= state::MIN_SIZE[0] && height >= state::MIN_SIZE[1] {
            window.width = width.round();
            window.height = height.round();
            if let Some(outer) = outer {
                window.x = Some((outer.min.x * zoom).round());
                window.y = Some((outer.min.y * zoom).round());
            }
        }
        self.stored.window = Some(window);
    }

    /// Over the page: the update notice. Not over a setup screen whose card
    /// shows the update and installs it already.
    pub fn paint(&mut self, ctx: &Context) {
        if self.in_setup && update::setup_view(ctx).is_some_and(|v| v.installs) {
            return;
        }
        self.updates.paint(ctx);
    }

    /// The app ends: what it showed goes, what it remembers is written.
    pub fn on_exit(&mut self) {
        self.notifier.clear();
        if self.stored != self.saved {
            state::save(&self.stored);
            self.saved = self.stored.clone();
        }
        // The queued writes (state, settings, volumes) finish before the process ends.
        if !paths::flush(Duration::from_secs(3)) {
            tracing::warn!("state: writes still running at exit");
        }
    }
}

/// Binds the global key while a call wants push-to-talk and hands the key
/// to the call beside the window's own ([`push_to_talk_from`]).
#[derive(Default)]
struct PttLink {
    /// The key last bound or tried; a failed one is not tried every pass.
    key: Option<String>,
    binding: Option<ptt::Ptt>,
    events: Arc<Mutex<Vec<ptt::Event>>>,
    status: Option<PttStatus>,
    down: bool,
}

impl PttLink {
    /// No app (the setup screen), so no call: the binding (Windows hook,
    /// portal session) ends.
    fn stop(&mut self, ctx: &Context) {
        if self.key.is_some() || self.status.is_some() {
            self.binding = None;
            self.events.lock().clear();
            self.down = false;
            self.status = None;
            self.key = None;
            ctx.data_mut(|d| d.insert_temp::<Option<PttStatus>>(ptt_id(), None));
        }
    }

    fn tick(&mut self, ctx: &Context, call: &gelabber_call::Call, p: &prefs::Prefs) {
        let in_call = call.with_state(|s| {
            matches!(
                s.status,
                gelabber_call::CallStatus::Connecting
                    | gelabber_call::CallStatus::Connected
                    | gelabber_call::CallStatus::Reconnecting
            )
        });
        let want = (in_call && p.voice_input == "ptt" && !p.ptt_key.is_empty())
            .then_some(p.ptt_key.as_str());
        if self.key.as_deref() != want {
            // The old binding ends before a new one starts.
            self.binding = None;
            self.events.lock().clear();
            if self.down {
                self.down = false;
                push_to_talk_from(ctx, call, PttSource::Hook, false);
            }
            self.status = None;
            self.key = want.map(str::to_owned);
            if let Some(key) = want {
                let events = self.events.clone();
                let wake = ctx.clone();
                let report: ptt::Report = Arc::new(move |e| {
                    events.lock().push(e);
                    wake.request_repaint();
                });
                match ptt::bind(key, report) {
                    Ok(binding) => self.binding = Some(binding),
                    Err(reason) => self.status = Some(PttStatus::Failed { reason }),
                }
            }
        }
        for event in std::mem::take(&mut *self.events.lock()) {
            match event {
                ptt::Event::Press => {
                    self.down = true;
                    push_to_talk_from(ctx, call, PttSource::Hook, true);
                }
                ptt::Event::Release => {
                    self.down = false;
                    push_to_talk_from(ctx, call, PttSource::Hook, false);
                }
                ptt::Event::Bound { trigger } => self.status = Some(PttStatus::Bound { trigger }),
                ptt::Event::Failed { reason } => self.status = Some(PttStatus::Failed { reason }),
            }
        }
        let status = self.status.clone();
        ctx.data_mut(|d| d.insert_temp(ptt_id(), status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a tray, a notification service or a call the passes run,
    /// remember the window and publish what the settings page shows.
    #[test]
    fn passes_run_without_a_desktop_and_remember_the_window() {
        let config = paths::testing::TempConfig::new("platform-pass");
        let ctx = Context::default();
        let client = Client::new("http://127.0.0.1:9").unwrap();
        let mut platform = Platform::new(None);
        platform.prepare_hook()(&client);
        let mut app = GelabberApp::with_context(&ctx, client, None);
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .inner_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        ));
        for _ in 0..2 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                platform.logic(ui.ctx(), Some(&mut app));
                platform.paint(ui.ctx());
            });
            out.textures_delta.clear();
        }
        let info = desktop_info(&ctx).unwrap();
        assert!(!info.close_to_tray);
        assert_eq!(ptt_status(&ctx), None);
        platform.on_exit();
        let stored = state::load();
        let window = stored.window.unwrap();
        assert_eq!((window.width, window.height), (900.0, 700.0));
        drop(config);
    }

    /// A hidden or uncomposited window gets logic-only passes whose input
    /// time stands still; the twice-a-second duties must go on regardless.
    #[test]
    fn the_tick_runs_on_passes_whose_input_time_stands_still() {
        let config = paths::testing::TempConfig::new("platform-tick-clock");
        let ctx = Context::default();
        let client = Client::new("http://127.0.0.1:9").unwrap();
        let mut platform = Platform::new(None);
        let mut app = GelabberApp::with_context(&ctx, client, None);
        let pass = |platform: &mut Platform, app: &mut GelabberApp, size: egui::Vec2| {
            let mut input = egui::RawInput {
                time: Some(10.0),
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
            let mut out = ctx.run_ui(input, |ui| platform.logic(ui.ctx(), Some(app)));
            out.textures_delta.clear();
        };
        pass(&mut platform, &mut app, egui::vec2(900.0, 700.0));
        assert_eq!(platform.stored.window.unwrap().width, 900.0);
        std::thread::sleep(Duration::from_millis(600));
        pass(&mut platform, &mut app, egui::vec2(1000.0, 700.0));
        assert_eq!(platform.stored.window.unwrap().width, 1000.0);
        drop(config);
    }

    /// "Server wechseln …" in a push-to-talk call: the setup screen has no
    /// call, so the global key (hook, portal session) is let go.
    #[test]
    fn the_setup_screen_lets_the_global_key_go() {
        let config = paths::testing::TempConfig::new("platform-ptt-setup");
        let ctx = Context::default();
        let mut platform = Platform::new(None);
        platform.ptt.key = Some("F13".into());
        platform.ptt.status = Some(PttStatus::Bound { trigger: None });
        ctx.data_mut(|d| d.insert_temp(ptt_id(), platform.ptt.status.clone()));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            platform.logic(ui.ctx(), None);
        });
        out.textures_delta.clear();
        assert_eq!(platform.ptt.key, None);
        assert_eq!(ptt_status(&ctx), None);
        drop(config);
    }

    /// "Groß" zooms egui's points by 1.25; the window opens before any zoom,
    /// so what is stored must be the window's own size, not the points.
    #[test]
    fn the_window_is_remembered_without_the_text_size() {
        let config = paths::testing::TempConfig::new("platform-zoom");
        let ctx = Context::default();
        ctx.set_zoom_factor(1.25);
        let mut platform = Platform::new(None);
        let mut input = egui::RawInput::default();
        let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.inner_rect = Some(egui::Rect::from_min_size(
            egui::pos2(80.0, 40.0),
            egui::vec2(800.0, 560.0),
        ));
        viewport.outer_rect = Some(egui::Rect::from_min_size(
            egui::pos2(80.0, 16.0),
            egui::vec2(800.0, 584.0),
        ));
        let mut out = ctx.run_ui(input, |ui| platform.track_window(ui.ctx()));
        out.textures_delta.clear();
        let window = platform.stored.window.unwrap();
        assert_eq!((window.width, window.height), (1000.0, 700.0));
        assert_eq!((window.x, window.y), (Some(100.0), Some(20.0)));
        drop(config);
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) mod testing {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    /// Ends the bus with the test, also one that failed.
    pub struct Daemon(std::process::Child);

    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A session bus of its own without service files, or none where
    /// dbus-daemon is missing (headless CI).
    pub fn private_bus(name: &str) -> Option<(Daemon, String)> {
        const CONFIG: &str = r#"<busconfig>
          <type>session</type>
          <listen>unix:tmpdir=/tmp</listen>
          <auth>EXTERNAL</auth>
          <policy context="default">
            <allow send_destination="*" eavesdrop="true"/>
            <allow eavesdrop="true"/>
            <allow own="*"/>
          </policy>
        </busconfig>"#;
        let config = std::env::temp_dir().join(format!(
            "gelabber-ui-bus-{}-{name}.conf",
            std::process::id()
        ));
        std::fs::write(&config, CONFIG).ok()?;
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let output = daemon.stdout.take();
        let daemon = Daemon(daemon);
        let mut address = String::new();
        BufReader::new(output?).read_line(&mut address).ok()?;
        let _ = std::fs::remove_file(config);
        Some((daemon, address.trim().to_owned()))
    }
}
