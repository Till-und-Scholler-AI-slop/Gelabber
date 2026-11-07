//! Push-to-talk gate (web/src/voice/pushToTalk.ts): the microphone may send
//! while the key is held and for the release delay after. The interface
//! reports the key (window keys, or a global hook) with [`Gate::set_held`].

use std::time::{Duration, Instant};

pub const RELEASE_DELAY_MAX_MS: u64 = 1000;
pub const RELEASE_DELAY_DEFAULT_MS: u64 = 200;

pub fn clamp_release_delay(ms: f64) -> u64 {
    if !ms.is_finite() {
        return RELEASE_DELAY_DEFAULT_MS;
    }
    ms.clamp(0.0, RELEASE_DELAY_MAX_MS as f64).round() as u64
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    /// Push-to-talk is on in a running call.
    active: bool,
    release_delay: Duration,
    held: bool,
    open: bool,
    /// When a released key closes the gate.
    closes_at: Option<Instant>,
}

impl Gate {
    /// Turns push-to-talk on (`Some(delay)`) or off. Returns whether the
    /// microphone's gate changed.
    pub(crate) fn configure(&mut self, release_delay: Option<Duration>) -> bool {
        let was = self.is_open();
        match release_delay {
            Some(delay) if self.active => self.release_delay = delay,
            Some(delay) => {
                *self = Self {
                    active: true,
                    release_delay: delay,
                    ..Self::default()
                }
            }
            None => *self = Self::default(),
        }
        was != self.is_open()
    }

    /// The key went down or up. Returns whether the gate changed now; a
    /// release with a delay closes later, through [`Gate::tick`].
    pub(crate) fn set_held(&mut self, held: bool, now: Instant) -> bool {
        if !self.active || held == self.held {
            return false;
        }
        self.held = held;
        self.closes_at = None;
        if held {
            let changed = !self.open;
            self.open = true;
            return changed;
        }
        if self.release_delay.is_zero() {
            return self.close();
        }
        self.closes_at = Some(now + self.release_delay);
        false
    }

    /// Closes after the release delay; true when it did now.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        match self.closes_at {
            Some(at) if now >= at => {
                self.closes_at = None;
                self.close()
            }
            _ => false,
        }
    }

    fn close(&mut self) -> bool {
        if self.held || !self.open {
            return false;
        }
        self.open = false;
        true
    }

    /// Whether the microphone may send as far as push-to-talk goes.
    pub(crate) fn is_open(&self) -> bool {
        !self.active || self.open
    }

    pub(crate) fn held(&self) -> bool {
        self.held
    }

    pub(crate) fn pending(&self) -> Option<Instant> {
        self.closes_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_push_to_talk_the_gate_is_open() {
        let gate = Gate::default();
        assert!(gate.is_open());
    }

    #[test]
    fn the_key_opens_and_the_delay_closes() {
        let start = Instant::now();
        let mut gate = Gate::default();
        assert!(gate.configure(Some(Duration::from_millis(200))), "closes");
        assert!(!gate.is_open());
        assert!(gate.set_held(true, start));
        assert!(gate.is_open());
        assert!(!gate.set_held(false, start), "the delay keeps it open");
        assert!(gate.is_open());
        assert!(!gate.tick(start + Duration::from_millis(100)));
        assert!(gate.tick(start + Duration::from_millis(200)));
        assert!(!gate.is_open());
    }

    #[test]
    fn pressing_again_within_the_delay_keeps_it_open() {
        let start = Instant::now();
        let mut gate = Gate::default();
        gate.configure(Some(Duration::from_millis(200)));
        gate.set_held(true, start);
        gate.set_held(false, start);
        assert!(!gate.set_held(true, start + Duration::from_millis(50)));
        assert!(!gate.tick(start + Duration::from_millis(300)));
        assert!(gate.is_open());
    }

    #[test]
    fn no_delay_closes_at_once_and_off_reopens() {
        let start = Instant::now();
        let mut gate = Gate::default();
        gate.configure(Some(Duration::ZERO));
        gate.set_held(true, start);
        assert!(gate.set_held(false, start));
        assert!(!gate.is_open());
        assert!(gate.configure(None));
        assert!(gate.is_open());
    }

    #[test]
    fn a_new_delay_keeps_the_key_state() {
        let start = Instant::now();
        let mut gate = Gate::default();
        gate.configure(Some(Duration::ZERO));
        gate.set_held(true, start);
        assert!(!gate.configure(Some(Duration::from_millis(500))));
        assert!(gate.is_open());
    }

    #[test]
    fn release_delay_is_clamped() {
        assert_eq!(clamp_release_delay(-5.0), 0);
        assert_eq!(clamp_release_delay(5000.0), 1000);
        assert_eq!(clamp_release_delay(f64::NAN), 200);
        assert_eq!(clamp_release_delay(249.6), 250);
    }
}
