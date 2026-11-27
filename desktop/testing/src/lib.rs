//! What a test does when the machine lacks something it needs: a program
//! (`dbus-daemon`, a C++ compiler) or a service. On a developer's machine it
//! says that it skipped and why; in CI it fails, so a missing package cannot
//! turn a run green (GEL-375). CI means the variable `CI` is set to anything
//! but "", "0" or "false" (GitHub Actions sets "true").
//!
//! ```text
//! let Some(bus) = private_bus() else {
//!     return gelabber_testing::missing("dbus-daemon");
//! };
//! ```

/// Call where `what` is missing, then return from the test. Panics in CI.
#[track_caller]
pub fn missing(what: &str) {
    missing_in(in_ci(), what);
}

/// `CI` is set, and not to "", "0" or "false" (in any case).
pub fn in_ci() -> bool {
    ci_from(std::env::var("CI").ok().as_deref())
}

fn ci_from(value: Option<&str>) -> bool {
    let value = value.unwrap_or("").trim().to_ascii_lowercase();
    !matches!(value.as_str(), "" | "0" | "false")
}

#[track_caller]
fn missing_in(ci: bool, what: &str) {
    if ci {
        panic!(
            "{what} is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)"
        );
    }
    eprintln!("skipped: {what} is missing on this machine (with CI set, this fails)");
}

#[cfg(test)]
mod tests {
    use super::{ci_from, missing_in};

    #[test]
    fn outside_ci_a_missing_prerequisite_skips() {
        missing_in(false, "a tool");
    }

    #[test]
    #[should_panic(
        expected = "a tool is missing; CI must provide it (GEL-375). (CI is set; unset it to skip here)"
    )]
    fn in_ci_a_missing_prerequisite_fails() {
        missing_in(true, "a tool");
    }

    #[test]
    fn ci_counts_unless_unset_empty_0_or_false() {
        for (value, ci) in [
            (Some("true"), true),
            (Some("TRUE"), true),
            (Some("1"), true),
            (Some("yes"), true),
            (Some(""), false),
            (Some("0"), false),
            (Some("false"), false),
            (Some("False"), false),
            (Some(" false "), false),
            (None, false),
        ] {
            assert_eq!(ci_from(value), ci, "CI={value:?}");
        }
    }
}
