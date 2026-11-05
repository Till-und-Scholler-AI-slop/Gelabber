//! The desktop around the window, what the Tauri app (desktop/app) does
//! outside its page: persistence in its config directory, system
//! notifications, the tray with close-to-tray, one app per user, the
//! global push-to-talk key, and the update check. The binary owns one
//! [`Platform`] and calls it every pass; parity runs go without one.

pub mod instance;
pub mod legacy;
pub mod notify;
pub mod paths;
pub mod ptt;
pub mod session;
pub mod settings;
pub mod state;
pub mod tray;
pub mod update;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use egui::{Context, Id, Ui, ViewportCommand};
use gelabber_client::Client;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::GelabberApp;
use crate::screens::settings::prefs;
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
    next_tick: f64,
    stored: state::Stored,
    saved: state::Stored,
    restored_for: Option<Uuid>,
    volumes: Volumes,
    ptt: PttLink,
    updates: Updates,
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
            next_tick: 0.0,
            saved: stored.clone(),
            stored,
            restored_for: None,
            volumes: Volumes::default(),
            ptt: PttLink::default(),
            updates: Updates::default(),
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
        // Dark title bar and decorations, the app's only theme.
        ctx.send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Dark));
        if let Some(instance) = &self.instance {
            instance.set_wake(waker(&self.wake));
        }
        self.tray = tray::Tray::start(self.tray_events.clone(), waker(&self.wake));
        self.updates.next = ctx.input(|i| i.time) + update::FIRST_CHECK.as_secs_f64();
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
        let now = ctx.input(|i| i.time);
        let Some(app) = app else {
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
        let outcome = self.notifier.frame(
            &client,
            notify::Context {
                in_front,
                desktop_enabled: p.desktop_notify,
                now_ms: state::now_ms(),
                server_now: client.http().server_now(),
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
        self.ptt.tick(ctx, &call, &p);
        if now >= self.next_tick {
            self.next_tick = now + 0.5;
            self.tick(ctx, &client, &call, in_front);
        }
        self.updates.tick(&client, now);
        if !self.hidden {
            ctx.request_repaint_after(Duration::from_millis(500));
        } else {
            // Hidden: the logic pass still runs for tray clicks and events.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// Twice a second: the sign-in, the tray's dot, what is remembered.
    fn tick(&mut self, ctx: &Context, client: &Client, call: &gelabber_call::Call, in_front: bool) {
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
        if let Some(tray) = &mut self.tray {
            tray.set_unread(unread);
        }
        self.remember(ctx, me, call);
    }

    /// Restores an account's memory once, then keeps the file current.
    /// Every pass, before the shell's route effects: "/" must already find
    /// the remembered place on the pass the account becomes known.
    fn restore(&mut self, ctx: &Context, me: Option<Uuid>) {
        if let Some(user) = me
            && self.restored_for != Some(user)
        {
            self.restored_for = Some(user);
            self.volumes = Volumes::default();
            if let Some(account) = self.stored.accounts.get(&user) {
                shell::restore_memory(ctx, user, account.memory());
                chat::restore_drafts(ctx, account.fresh_drafts(state::now_ms()));
            }
        }
    }

    fn remember(&mut self, ctx: &Context, me: Option<Uuid>, call: &gelabber_call::Call) {
        let now = state::now_ms();
        if let Some(user) = me.filter(|u| self.restored_for == Some(*u)) {
            let account = self.stored.accounts.entry(user).or_default();
            if let Some(memory) = shell::memory_of(ctx, user) {
                account.set_memory(&memory);
            }
            account.set_drafts(&chat::drafts(ctx), now);
            self.volumes.tick(call, account);
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
        // A maximized window keeps the size it comes back to.
        if !window.maximized
            && inner.width() >= state::MIN_SIZE[0]
            && inner.height() >= state::MIN_SIZE[1]
        {
            window.width = inner.width().round();
            window.height = inner.height().round();
            if let Some(outer) = outer {
                window.x = Some(outer.min.x.round());
                window.y = Some(outer.min.y.round());
            }
        }
        self.stored.window = Some(window);
    }

    /// Over the page: the update notice.
    pub fn paint(&mut self, ui: &mut Ui, app: Option<&GelabberApp>) {
        let in_call = app.is_some_and(|a| {
            a.call()
                .with_state(|s| s.status != gelabber_call::CallStatus::Idle)
        });
        self.updates.paint(ui, in_call);
    }

    /// The app ends: what it showed goes, what it remembers is written.
    pub fn on_exit(&mut self) {
        self.notifier.clear();
        // Written here as well: the last background write may not finish.
        if self.stored != state::Stored::default()
            && let Err(e) = paths::write_json(
                "native-state.json",
                &serde_json::to_value(&self.stored).unwrap_or_default(),
                true,
            )
        {
            tracing::warn!("state: {e}");
        }
    }
}

/// Per-person volumes: stored ones go to people who join the call, the
/// user's changes are kept (voice/userVolume.ts).
#[derive(Default)]
struct Volumes {
    channel: Option<Uuid>,
    /// Per person: the volume last set or seen, and whether the call
    /// reported it back (only then is a difference the user's change).
    applied: std::collections::HashMap<Uuid, (f32, bool)>,
}

impl Volumes {
    fn tick(&mut self, call: &impl VolumeSink, account: &mut state::Account) {
        let (channel, people) = call.people();
        if channel != self.channel {
            self.channel = channel;
            self.applied.clear();
        }
        let same = |a: f32, b: f32| (a - b).abs() <= 0.001;
        for (user, volume) in people {
            match self.applied.get(&user).copied() {
                None => {
                    let stored = account.volumes.get(&user).map_or(1.0, |v| v.volume);
                    let confirmed = same(stored, volume);
                    if !confirmed {
                        call.set_volume(user, stored);
                    }
                    self.applied.insert(user, (stored, confirmed));
                }
                Some((target, false)) => {
                    if same(target, volume) {
                        self.applied.insert(user, (target, true));
                    }
                }
                Some((applied, true)) if !same(applied, volume) => {
                    account.set_volume(user, volume);
                    self.applied.insert(user, (volume, true));
                }
                Some(_) => {}
            }
        }
    }
}

/// The call as [`Volumes`] uses it.
trait VolumeSink {
    /// The call's channel and the others in it with their volumes.
    fn people(&self) -> (Option<Uuid>, Vec<(Uuid, f32)>);
    fn set_volume(&self, user: Uuid, volume: f32);
}

impl VolumeSink for gelabber_call::Call {
    fn people(&self) -> (Option<Uuid>, Vec<(Uuid, f32)>) {
        self.with_state(|s| {
            (
                s.channel,
                s.participants
                    .iter()
                    .filter(|p| !p.me)
                    .map(|p| (p.user, p.volume))
                    .collect(),
            )
        })
    }

    fn set_volume(&self, user: Uuid, volume: f32) {
        gelabber_call::Call::set_volume(self, user, volume);
    }
}

/// Binds the global key while a call wants push-to-talk and hands the key
/// to [`gelabber_call::Call::push_to_talk`].
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
                call.push_to_talk(false);
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
                    call.push_to_talk(true);
                }
                ptt::Event::Release => {
                    self.down = false;
                    call.push_to_talk(false);
                }
                ptt::Event::Bound { trigger } => self.status = Some(PttStatus::Bound { trigger }),
                ptt::Event::Failed { reason } => self.status = Some(PttStatus::Failed { reason }),
            }
        }
        let status = self.status.clone();
        ctx.data_mut(|d| d.insert_temp(ptt_id(), status));
    }
}

type Check = Arc<Mutex<Option<Result<Option<update::Found>, String>>>>;

#[derive(Default)]
struct Updates {
    /// `ctx` time of the next look.
    next: f64,
    last: Option<SystemTime>,
    running: Option<Check>,
    found: Option<update::Found>,
    /// "Später": no notice for this version until the next start.
    dismissed: Option<String>,
}

impl Updates {
    fn tick(&mut self, client: &Client, now: f64) {
        if let Some(slot) = &self.running {
            let done = slot.lock().take();
            if let Some(result) = done {
                self.running = None;
                self.last = Some(SystemTime::now());
                match result {
                    Ok(found) => self.found = found,
                    Err(e) => tracing::info!("update check: {e}"),
                }
            }
            return;
        }
        if now < self.next {
            return;
        }
        self.next = now + 60.0;
        if !update::auto_check() || !update::due(self.last, SystemTime::now()) {
            return;
        }
        let slot: Check = Arc::default();
        self.running = Some(slot.clone());
        let http = client.http().reqwest().clone();
        client.spawn(async move {
            *slot.lock() = Some(update::check(http).await);
        });
    }

    fn paint(&mut self, ui: &mut Ui, in_call: bool) {
        let Some(found) = self.found.clone() else {
            return;
        };
        // Not over a call, as the Tauri app's notice.
        if in_call || self.dismissed.as_deref() == Some(found.version.as_str()) {
            return;
        }
        let ctx = ui.ctx().clone();
        let theme = crate::theme::Theme::of(&ctx);
        let c = &theme.c;
        let screen = ctx.content_rect();
        let width = (screen.width() - 32.0).min(560.0);
        egui::Area::new(Id::unique("gelabber-update-notice"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .show(&ctx, |ui| {
                egui::Frame::new()
                    .fill(c.panel)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(width);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let line = format!("Gelabber {} ist da.", found.version);
                            let block =
                                crate::text::Block::single(ui, &line, crate::text::BODY, c.text);
                            let (rect, _) =
                                ui.allocate_exact_size(block.size(), egui::Sense::hover());
                            block.paint(ui.painter(), rect.min);
                            if ui
                                .add(
                                    crate::widgets::Button::new("Herunterladen")
                                        .primary()
                                        .small(),
                                )
                                .clicked()
                            {
                                ctx.open_url(egui::OpenUrl::new_tab(update::release_page(
                                    &found.version,
                                )));
                            }
                            if ui
                                .add(crate::widgets::Button::new("Später").small())
                                .clicked()
                            {
                                self.dismissed = Some(found.version.clone());
                            }
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call whose volumes follow `set_volume` once `answers` is on.
    struct FakeCall {
        channel: Option<Uuid>,
        people: Mutex<Vec<(Uuid, f32)>>,
        answers: bool,
        sent: Mutex<Vec<(Uuid, f32)>>,
    }

    impl VolumeSink for FakeCall {
        fn people(&self) -> (Option<Uuid>, Vec<(Uuid, f32)>) {
            (self.channel, self.people.lock().clone())
        }
        fn set_volume(&self, user: Uuid, volume: f32) {
            self.sent.lock().push((user, volume));
            if self.answers {
                for p in self.people.lock().iter_mut().filter(|p| p.0 == user) {
                    p.1 = volume;
                }
            }
        }
    }

    #[test]
    fn stored_volumes_reach_who_joins_and_the_users_changes_are_kept() {
        let (anna, tom) = (Uuid::new_v4(), Uuid::new_v4());
        let mut account = state::Account::default();
        account.set_volume(anna, 1.5);
        let call = FakeCall {
            channel: Some(Uuid::new_v4()),
            people: Mutex::new(vec![(anna, 1.0), (tom, 1.0)]),
            answers: true,
            sent: Mutex::default(),
        };
        let mut volumes = Volumes::default();
        volumes.tick(&call, &mut account);
        assert_eq!(*call.sent.lock(), vec![(anna, 1.5)]);
        volumes.tick(&call, &mut account);
        // The user turns Tom down: kept.
        call.people.lock()[1].1 = 0.5;
        volumes.tick(&call, &mut account);
        assert_eq!(account.volumes[&tom].volume, 0.5);
        assert_eq!(account.volumes[&anna].volume, 1.5);
        // A call that has not taken the stored volume yet changes nothing.
        let slow = FakeCall {
            channel: Some(Uuid::new_v4()),
            people: Mutex::new(vec![(anna, 0.0)]),
            answers: false,
            sent: Mutex::default(),
        };
        let mut volumes = Volumes::default();
        volumes.tick(&slow, &mut account);
        volumes.tick(&slow, &mut account);
        assert_eq!(account.volumes[&anna].volume, 1.5);
    }

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
                platform.paint(ui, Some(&app));
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
}

#[cfg(test)]
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
