//! `GELABBER_SMOKE=<seconds>`: the start test of CI (desktop/packaging/native/
//! smoke.sh). After that long the app names the screen it reached and ends,
//! with 0 when it painted the setup or the login screen and 1 otherwise.

use std::sync::atomic::{AtomicI32, Ordering};

/// The exit status once the test has decided; -1 before.
static EXIT: AtomicI32 = AtomicI32::new(-1);

/// Frames before the verdict: the first pass can end before anything is shown.
const MIN_FRAMES: u64 = 2;

pub struct Smoke {
    after: f64,
    frames: u64,
    /// `ctx` time of the verdict.
    decided: Option<f64>,
}

impl Smoke {
    /// From the variable's value; unset, empty or not a positive number is off.
    pub fn from_env(value: Option<&str>) -> Option<Self> {
        let after: f64 = value?.trim().parse().ok()?;
        (after.is_finite() && after > 0.0).then_some(Self {
            after,
            frames: 0,
            decided: None,
        })
    }

    pub fn frame(&mut self) {
        self.frames += 1;
    }

    /// The exit status at `now` on `screen` ("setup", or the route), once
    /// the time is up; `None` before and after.
    pub fn check(&mut self, now: f64, screen: &str) -> Option<i32> {
        if self.decided.is_some() || now < self.after {
            return None;
        }
        self.decided = Some(now);
        let reached = matches!(screen, "setup" | "Login") && self.frames >= MIN_FRAMES;
        let code = if reached { 0 } else { 1 };
        println!(
            "gelabber smoke: {} on {screen} after {now:.1} s and {} frames", // i18n-ok
            if reached { "ok" } else { "FAILED" },
            self.frames
        );
        EXIT.store(code, Ordering::SeqCst);
        Some(code)
    }

    /// True when the window did not close within 10 s of the verdict (a
    /// close that something kept from happening).
    pub fn overdue(&self, now: f64) -> bool {
        self.decided.is_some_and(|at| now - at > 10.0)
    }
}

/// The verdict for `main`, once given.
pub fn exit_code() -> Option<i32> {
    let code = EXIT.load(Ordering::SeqCst);
    (code >= 0).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_positive_number_of_seconds_switches_it_on() {
        assert!(Smoke::from_env(None).is_none());
        assert!(Smoke::from_env(Some("")).is_none());
        assert!(Smoke::from_env(Some("0")).is_none());
        assert!(Smoke::from_env(Some("-3")).is_none());
        assert!(Smoke::from_env(Some("bald")).is_none());
        assert!(Smoke::from_env(Some("NaN")).is_none());
        assert_eq!(Smoke::from_env(Some(" 5 ")).map(|s| s.after), Some(5.0));
    }

    #[test]
    fn passes_on_the_setup_or_login_screen_after_frames_and_decides_once() {
        let mut smoke = Smoke::from_env(Some("3")).unwrap();
        smoke.frame();
        smoke.frame();
        assert_eq!(smoke.check(1.0, "setup"), None);
        assert_eq!(smoke.check(3.5, "setup"), Some(0));
        assert_eq!(smoke.check(4.0, "setup"), None);
        assert!(!smoke.overdue(5.0));
        assert!(smoke.overdue(14.0));

        let mut login = Smoke::from_env(Some("1")).unwrap();
        (0..3).for_each(|_| login.frame());
        assert_eq!(login.check(1.0, "Login"), Some(0));
    }

    #[test]
    fn fails_elsewhere_or_without_painted_frames() {
        let mut home = Smoke::from_env(Some("1")).unwrap();
        (0..3).for_each(|_| home.frame());
        assert_eq!(home.check(2.0, "Home"), Some(1));

        let mut blank = Smoke::from_env(Some("1")).unwrap();
        blank.frame();
        assert_eq!(blank.check(2.0, "setup"), Some(1));
    }
}
