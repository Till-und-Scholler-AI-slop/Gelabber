//! What a test does when the machine lacks something it needs: a program
//! (`dbus-daemon`, a C++ compiler) or a service. On a developer's machine it
//! says that it skipped and why; in CI (`CI` set, as GitHub Actions sets it)
//! it fails, so a missing package cannot turn a run green (GEL-375).
//!
//! ```ignore
//! let Some(bus) = private_bus() else {
//!     return gelabber_testing::missing("dbus-daemon");
//! };
//! ```

/// Call where `what` is missing, then return from the test. Panics in CI.
#[track_caller]
pub fn missing(what: &str) {
    missing_in(in_ci(), what);
}

/// `CI` is set to anything but the empty string.
pub fn in_ci() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}

#[track_caller]
fn missing_in(ci: bool, what: &str) {
    if ci {
        panic!("{what} is missing; CI must provide it (GEL-375)");
    }
    eprintln!("skipped: {what} is missing on this machine (with CI set, this fails)");
}

#[cfg(test)]
mod tests {
    use super::missing_in;

    #[test]
    fn outside_ci_a_missing_prerequisite_skips() {
        missing_in(false, "a tool");
    }

    #[test]
    #[should_panic(expected = "a tool is missing; CI must provide it")]
    fn in_ci_a_missing_prerequisite_fails() {
        missing_in(true, "a tool");
    }
}
