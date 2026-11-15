//! When this client counts as away (#566). Online while there is input on
//! the computer or while sitting in a voice channel undeafened; away after
//! [`CLIENT_IDLE`] without either. The interface feeds window input
//! ([`crate::Client::activity`]); the platform may add the system's last
//! input anywhere through [`set_system_idle`].
//!
//! Kept free of I/O so the rules are tested as plain state transitions, and
//! so a manual status (#590) can sit on top of the automatic one.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::models::PresenceStatus;

/// Away after this long without input (the web's `CLIENT_IDLE_MS`).
pub const CLIENT_IDLE: Duration = Duration::from_secs(300);
/// Input reports online at most this often.
const PULSE: Duration = Duration::from_millis(10_000);
/// While in a call, online is repeated this often, so a server that still
/// times sockets out by itself keeps the user online.
const CALL_PULSE: Duration = Duration::from_secs(60);

/// The time of the last keyboard or mouse input anywhere on the computer,
/// as the platform last read it. Must not block: it is asked every tick.
pub trait SystemIdle: Send + Sync {
    /// `None` while no source can tell (the window's input alone counts).
    fn last_input(&self) -> Option<Instant>;
}

static SYSTEM_IDLE: OnceLock<Arc<dyn SystemIdle>> = OnceLock::new();

/// Installs the process's system idle source once; later calls are ignored.
pub fn set_system_idle(source: Arc<dyn SystemIdle>) {
    let _ = SYSTEM_IDLE.set(source);
}

pub(crate) fn system_last_input() -> Option<Instant> {
    SYSTEM_IDLE.get().and_then(|source| source.last_input())
}

/// The automatic status of this client.
#[derive(Debug, Default)]
pub(crate) struct Presence {
    /// Latest input this client knows of: in the window, anywhere on the
    /// computer, or a call still going on.
    last_input: Option<Instant>,
    /// Latest input in the window: whether someone looks at this window
    /// (`in_front`, which holds Web Push back) ignores the system and calls.
    last_window_input: Option<Instant>,
    last_pulse: Option<Instant>,
    idle: bool,
}

impl Presence {
    /// Away as last reported.
    pub(crate) fn idle(&self) -> bool {
        self.idle
    }

    /// No input in the window for [`CLIENT_IDLE`] (or none yet).
    pub(crate) fn window_idle(&self, now: Instant) -> bool {
        self.last_window_input
            .is_none_or(|t| now.saturating_duration_since(t) >= CLIENT_IDLE)
    }

    /// Pointer or key input in the window.
    pub(crate) fn window_input(&mut self, now: Instant) -> Option<PresenceStatus> {
        self.last_window_input = Some(now);
        self.input(now, now)
    }

    /// The system's last input anywhere. Older input than known changes
    /// nothing.
    pub(crate) fn system_input(&mut self, at: Instant, now: Instant) -> Option<PresenceStatus> {
        if self.last_input.is_some_and(|t| t >= at)
            || now.saturating_duration_since(at) >= CLIENT_IDLE
        {
            return None;
        }
        self.input(at, now)
    }

    fn input(&mut self, at: Instant, now: Instant) -> Option<PresenceStatus> {
        self.last_input = Some(self.last_input.map_or(at, |t| t.max(at)));
        self.report_online(now, PULSE)
    }

    fn report_online(&mut self, now: Instant, every: Duration) -> Option<PresenceStatus> {
        let due = self.idle || self.last_pulse.is_none_or(|t| now - t >= every);
        self.idle = false;
        if due {
            self.last_pulse = Some(now);
        }
        due.then_some(PresenceStatus::Online)
    }

    /// The timer, every tick. `in_call`: in a voice channel and not
    /// deafened, which keeps the user online; the five minutes start when
    /// the call ends or the user deafens.
    pub(crate) fn tick(&mut self, now: Instant, in_call: bool) -> Option<PresenceStatus> {
        if in_call {
            self.last_input = Some(now);
            return self.report_online(now, CALL_PULSE);
        }
        let away = !self.idle
            && self
                .last_input
                .is_some_and(|t| now.saturating_duration_since(t) >= CLIENT_IDLE);
        if away {
            self.idle = true;
        }
        away.then_some(PresenceStatus::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn started() -> (Presence, Instant) {
        let t0 = Instant::now();
        let mut p = Presence::default();
        assert_eq!(p.window_input(t0), Some(PresenceStatus::Online));
        (p, t0)
    }

    #[test]
    fn away_after_five_minutes_without_input() {
        let (mut p, t0) = started();
        assert_eq!(p.tick(t0 + S(299), false), None);
        assert_eq!(p.tick(t0 + S(300), false), Some(PresenceStatus::Idle));
        assert!(p.idle());
        assert_eq!(p.tick(t0 + S(301), false), None, "said once");
        assert_eq!(p.window_input(t0 + S(302)), Some(PresenceStatus::Online));
        assert!(!p.idle());
    }

    #[test]
    fn window_input_pulses_at_most_every_ten_seconds() {
        let (mut p, t0) = started();
        assert_eq!(p.window_input(t0 + S(5)), None);
        assert_eq!(p.window_input(t0 + S(10)), Some(PresenceStatus::Online));
    }

    #[test]
    fn input_elsewhere_on_the_computer_keeps_the_user_online() {
        let (mut p, t0) = started();
        // The user types in another app; the platform reads it every 30 s.
        for minute in 1..=20 {
            let now = t0 + S(60 * minute);
            p.system_input(now - S(20), now);
            assert_eq!(p.tick(now, false), None, "minute {minute}");
        }
        assert!(!p.idle());
        // Then nobody touches anything: away five minutes after the last input.
        let last = t0 + S(60 * 20 - 20);
        assert_eq!(p.system_input(last, last + S(60)), None, "nothing newer");
        assert_eq!(p.tick(last + S(299), false), None);
        assert_eq!(p.tick(last + S(300), false), Some(PresenceStatus::Idle));
    }

    #[test]
    fn system_input_after_away_comes_back_online() {
        let (mut p, t0) = started();
        assert_eq!(p.tick(t0 + S(300), false), Some(PresenceStatus::Idle));
        let now = t0 + S(330);
        assert_eq!(
            p.system_input(now - S(2), now),
            Some(PresenceStatus::Online)
        );
        assert!(!p.idle());
    }

    #[test]
    fn stale_system_input_changes_nothing() {
        let (mut p, t0) = started();
        assert_eq!(p.tick(t0 + S(300), false), Some(PresenceStatus::Idle));
        // Read late: the input was already five minutes ago.
        let now = t0 + S(700);
        assert_eq!(p.system_input(now - S(300), now), None);
        assert!(p.idle());
    }

    #[test]
    fn a_call_keeps_the_user_online_and_leaving_starts_the_timer() {
        let (mut p, t0) = started();
        // Joined at once, then no input at all for an hour.
        let mut said = 0;
        for second in (1..=3600).step_by(1) {
            match p.tick(t0 + S(second), true) {
                Some(PresenceStatus::Online) => said += 1,
                None => {}
                other => panic!("{other:?} at {second}"),
            }
        }
        assert!(!p.idle());
        assert_eq!(said, 60, "online once a minute during the call");
        // Left (or deafened) at 3600: away five minutes later, not at once.
        let left = t0 + S(3600);
        assert_eq!(p.tick(left + S(1), false), None);
        assert_eq!(p.tick(left + S(299), false), None);
        assert_eq!(p.tick(left + S(300), false), Some(PresenceStatus::Idle));
    }

    #[test]
    fn joining_a_call_while_away_comes_back_online() {
        let (mut p, t0) = started();
        assert_eq!(p.tick(t0 + S(300), false), Some(PresenceStatus::Idle));
        assert_eq!(p.tick(t0 + S(400), true), Some(PresenceStatus::Online));
        assert!(!p.idle());
    }

    #[test]
    fn the_window_goes_out_of_front_on_window_input_alone() {
        let (mut p, t0) = started();
        assert!(!p.window_idle(t0 + S(299)));
        p.system_input(t0 + S(290), t0 + S(295));
        p.tick(t0 + S(296), true);
        assert!(
            p.window_idle(t0 + S(300)),
            "system input and calls do not count"
        );
        assert!(Presence::default().window_idle(t0));
    }

    #[test]
    fn no_input_ever_never_goes_away() {
        let mut p = Presence::default();
        assert_eq!(p.tick(Instant::now() + S(3600), false), None);
    }
}
