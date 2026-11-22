//! Per-socket budget for the frames a client sends (GEL-55).
//!
//! Every frame costs the socket a session and access check in the gateway
//! pool, and typing and voice signal frames are sent on to every subscriber,
//! each of which checks its access to receive them. Token buckets bound
//! that per socket, before any database or Redis work:
//!
//! - every frame needs a token of the frame budget. Without one the socket
//!   holds the frame and reads nothing more until the budget refills, so a
//!   fast client is slowed down by TCP backpressure and loses nothing;
//! - a voice signal frame also needs a signal token and waits the same way:
//!   it changes a seat's state (mute, deafen, publish), so dropping one would
//!   leave the server's view different from the client's;
//! - a typing frame over the typing budget is dropped. Typing is best effort
//!   and repeated every few seconds while someone types; a missed stop ends
//!   with the typing TTL.
//!
//! Nothing is sent back and the socket stays open: a close would only make
//! the client reconnect and subscribe to everything again.

use std::time::{Duration, Instant};

use crate::limits::{FrameRate, Limits};

/// What a frame needs from the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Frame,
    Typing,
    Signal,
}

/// What [`FrameBudget::admit`] decided for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Admit {
    Handle,
    /// A typing frame over its budget.
    Drop,
    /// Hold the frame for this long, then ask again. `bucket` names the
    /// budget that ran out, as in `gelabber_rate_limited_total`.
    Wait {
        wait: Duration,
        bucket: &'static str,
    },
}

pub(super) const FRAMES_BUCKET: &str = "ws_frames";
pub(super) const TYPING_BUCKET: &str = "ws_typing";
pub(super) const SIG_BUCKET: &str = "ws_sig";

struct Bucket {
    tokens: f64,
    burst: f64,
    /// Tokens per second; `0` turns the bucket off.
    rate: f64,
    at: Instant,
}

impl Bucket {
    fn new(limit: FrameRate, now: Instant) -> Self {
        let burst = f64::from(limit.burst.max(1));
        Self {
            tokens: burst,
            burst,
            rate: f64::from(limit.per_min) / 60.0,
            at: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.at = now;
    }

    /// How long until one token is there; zero when it is.
    fn wait(&mut self, now: Instant) -> Duration {
        if self.rate == 0.0 {
            return Duration::ZERO;
        }
        self.refill(now);
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / self.rate)
        }
    }

    /// Takes the token [`Self::wait`] found.
    fn take(&mut self) {
        if self.rate > 0.0 {
            self.tokens -= 1.0;
        }
    }
}

pub(super) struct FrameBudget {
    frames: Bucket,
    typing: Bucket,
    signal: Bucket,
}

impl FrameBudget {
    pub(super) fn new(limits: &Limits, now: Instant) -> Self {
        Self {
            frames: Bucket::new(limits.ws_frames, now),
            typing: Bucket::new(limits.ws_typing, now),
            signal: Bucket::new(limits.ws_sig, now),
        }
    }

    /// Spends the tokens a frame of `class` needs, or says why not.
    pub(super) fn admit(&mut self, class: Class, now: Instant) -> Admit {
        let frames = self.frames.wait(now);
        let signal = match class {
            Class::Signal => self.signal.wait(now),
            Class::Frame | Class::Typing => Duration::ZERO,
        };
        if !signal.is_zero() {
            return Admit::Wait {
                wait: signal.max(frames),
                bucket: SIG_BUCKET,
            };
        }
        if !frames.is_zero() {
            return Admit::Wait {
                wait: frames,
                bucket: FRAMES_BUCKET,
            };
        }
        self.frames.take();
        match class {
            Class::Frame => {}
            Class::Signal => self.signal.take(),
            // The frame token stays spent: a typing flood is read no faster
            // than any other.
            Class::Typing if self.typing.wait(now).is_zero() => self.typing.take(),
            Class::Typing => return Admit::Drop,
        }
        Admit::Handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(frames: (u32, u32), typing: (u32, u32), sig: (u32, u32)) -> Limits {
        let rate = |(per_min, burst)| FrameRate { per_min, burst };
        Limits {
            ws_frames: rate(frames),
            ws_typing: rate(typing),
            ws_sig: rate(sig),
            ..Limits::default()
        }
    }

    #[test]
    fn frames_past_the_burst_wait_for_the_refill() {
        let now = Instant::now();
        // Burst 2, one per second.
        let mut budget = FrameBudget::new(&limits((60, 2), (60, 5), (60, 5)), now);
        assert_eq!(budget.admit(Class::Frame, now), Admit::Handle);
        assert_eq!(budget.admit(Class::Frame, now), Admit::Handle);
        let Admit::Wait { wait, bucket } = budget.admit(Class::Frame, now) else {
            panic!("third frame of the burst");
        };
        assert_eq!(bucket, FRAMES_BUCKET);
        assert!(
            wait > Duration::from_millis(990) && wait <= Duration::from_secs(1),
            "{wait:?}"
        );
        let later = now + Duration::from_millis(500);
        assert!(matches!(
            budget.admit(Class::Frame, later),
            Admit::Wait { .. }
        ));
        assert_eq!(
            budget.admit(Class::Frame, now + Duration::from_secs(1)),
            Admit::Handle
        );
    }

    #[test]
    fn typing_over_its_budget_is_dropped_and_still_costs_a_frame() {
        let now = Instant::now();
        let mut budget = FrameBudget::new(&limits((60, 3), (60, 1), (60, 5)), now);
        assert_eq!(budget.admit(Class::Typing, now), Admit::Handle);
        assert_eq!(budget.admit(Class::Typing, now), Admit::Drop);
        assert_eq!(budget.admit(Class::Frame, now), Admit::Handle);
        assert!(matches!(
            budget.admit(Class::Typing, now),
            Admit::Wait {
                bucket: FRAMES_BUCKET,
                ..
            }
        ));
    }

    #[test]
    fn signal_frames_wait_for_their_own_budget() {
        let now = Instant::now();
        let mut budget = FrameBudget::new(&limits((60, 10), (60, 5), (30, 1)), now);
        assert_eq!(budget.admit(Class::Signal, now), Admit::Handle);
        let Admit::Wait { wait, bucket } = budget.admit(Class::Signal, now) else {
            panic!("second signal frame");
        };
        assert_eq!(bucket, SIG_BUCKET);
        assert!(wait > Duration::from_millis(1990), "{wait:?}");
        // Other frames do not wait for the signal budget.
        assert_eq!(budget.admit(Class::Frame, now), Admit::Handle);
        assert_eq!(
            budget.admit(Class::Signal, now + Duration::from_secs(2)),
            Admit::Handle
        );
    }

    #[test]
    fn a_rate_of_zero_turns_a_budget_off() {
        let now = Instant::now();
        let mut budget = FrameBudget::new(&limits((0, 1), (0, 1), (0, 1)), now);
        for class in [Class::Frame, Class::Typing, Class::Signal] {
            for _ in 0..100 {
                assert_eq!(budget.admit(class, now), Admit::Handle);
            }
        }
    }
}
