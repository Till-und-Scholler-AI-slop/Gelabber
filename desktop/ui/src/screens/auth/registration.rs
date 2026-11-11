//! Who may register (web/src/auth/registration.ts, #517): an open
//! instance takes anyone, an invitation-only one a beta invitation or a
//! server invite. The register page decides from this whether it asks for
//! a code, and which codes it sends.

use gelabber_client::RegistrationMode;

/// What decides about the "Einladungscode" field (`InvitationState`).
#[derive(Clone, Copy, Debug, Default)]
pub struct InvitationState<'a> {
    /// `None` while unknown or after a failed check.
    pub mode: Option<RegistrationMode>,
    /// `?invitation=` of the page's address.
    pub invitation: Option<&'a str>,
    /// The page's `?redirect=`.
    pub redirect: Option<&'a str>,
    /// The API (or a sign-in redirect) already asked for a code.
    pub asked: bool,
}

/// `?invitation=` as `validateAuthSearch` keeps it: 1 to 64 letters and
/// digits.
pub fn invitation_param(value: Option<&str>) -> Option<&str> {
    value.filter(|code| {
        (1..=64).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// `inviteCodeOf`: the server invite code of an `/invite/{code}` target.
pub fn invite_code_of(target: Option<&str>) -> Option<&str> {
    let rest = target?.strip_prefix("/invite/")?;
    let code = rest.split(['/', '?', '#']).next().unwrap_or("");
    (!code.is_empty()).then_some(code)
}

/// `showsInvitationField`: with a code in the address, or when the
/// instance needs one and no server invite is being followed (the invite
/// itself admits then).
pub fn shows_invitation_field(state: InvitationState<'_>) -> bool {
    if state.invitation.is_some() {
        return true;
    }
    if invite_code_of(state.redirect).is_some() {
        return false;
    }
    state.mode == Some(RegistrationMode::Invitation) || state.asked
}

/// `showsInvitationOnly`: "Gelabber ist gerade nur mit Einladung offen.",
/// invitation-only and the visitor brought no code of either kind.
pub fn shows_invitation_only(state: InvitationState<'_>) -> bool {
    state.mode == Some(RegistrationMode::Invitation)
        && state.invitation.is_none()
        && invite_code_of(state.redirect).is_none()
}

/// `validateInvitation`: empty is fine unless the instance needs a code
/// and no server invite comes along.
pub fn validate_invitation(
    raw: &str,
    mode: Option<RegistrationMode>,
    redirect: Option<&str>,
) -> Option<&'static str> {
    if !raw.trim().is_empty() || mode != Some(RegistrationMode::Invitation) {
        return None;
    }
    invite_code_of(redirect).is_none().then_some("required")
}

/// `isAdmissionError`: the API refused for want of a valid code.
pub fn is_admission_error(code: &str) -> bool {
    matches!(code, "invitation_required" | "invitation_invalid")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: Option<&str> = Some("/invite/abc123");

    fn state(
        mode: Option<RegistrationMode>,
        invitation: Option<&'static str>,
        redirect: Option<&'static str>,
        asked: bool,
    ) -> InvitationState<'static> {
        InvitationState {
            mode,
            invitation,
            redirect,
            asked,
        }
    }

    /// The cases of auth/registration.test.ts.
    #[test]
    fn the_field_and_the_line_show_like_the_web() {
        use RegistrationMode::{Invitation, Open};
        // A code in the address always brings the field.
        assert!(shows_invitation_field(state(
            Some(Open),
            Some("k7m2"),
            None,
            false
        )));
        assert!(shows_invitation_field(state(
            None,
            Some("k7m2"),
            INVITE,
            false
        )));
        // Invitation-only: the field, unless a server invite admits.
        assert!(shows_invitation_field(state(
            Some(Invitation),
            None,
            None,
            false
        )));
        assert!(!shows_invitation_field(state(
            Some(Invitation),
            None,
            INVITE,
            false
        )));
        assert!(!shows_invitation_field(state(
            Some(Invitation),
            None,
            INVITE,
            true
        )));
        // Open or unknown: only once the API asked.
        assert!(!shows_invitation_field(state(
            Some(Open),
            None,
            None,
            false
        )));
        assert!(!shows_invitation_field(state(None, None, None, false)));
        assert!(shows_invitation_field(state(None, None, None, true)));
        assert!(shows_invitation_field(state(
            Some(Open),
            None,
            Some("/s/x"),
            true
        )));

        assert!(shows_invitation_only(state(
            Some(Invitation),
            None,
            None,
            false
        )));
        assert!(!shows_invitation_only(state(
            Some(Invitation),
            Some("k7m2"),
            None,
            false
        )));
        assert!(!shows_invitation_only(state(
            Some(Invitation),
            None,
            INVITE,
            false
        )));
        assert!(!shows_invitation_only(state(Some(Open), None, None, false)));
        assert!(!shows_invitation_only(state(None, None, None, true)));
    }

    #[test]
    fn codes_are_read_and_checked_like_the_web() {
        use RegistrationMode::{Invitation, Open};
        assert_eq!(invite_code_of(INVITE), Some("abc123"));
        assert_eq!(invite_code_of(Some("/invite/abc?x=1")), Some("abc"));
        assert_eq!(invite_code_of(Some("/invite/")), None);
        assert_eq!(invite_code_of(Some("/s/abc")), None);
        assert_eq!(invite_code_of(None), None);

        assert_eq!(invitation_param(Some("k7m2p9qrs4tx")), Some("k7m2p9qrs4tx"));
        assert_eq!(invitation_param(Some("2026")), Some("2026"));
        assert_eq!(invitation_param(Some("")), None);
        assert_eq!(invitation_param(Some("k7m2-p9")), None);
        assert_eq!(invitation_param(Some(&"a".repeat(65))), None);

        assert_eq!(
            validate_invitation("", Some(Invitation), None),
            Some("required")
        );
        assert_eq!(
            validate_invitation("  ", Some(Invitation), None),
            Some("required")
        );
        assert_eq!(validate_invitation("", Some(Invitation), INVITE), None);
        assert_eq!(validate_invitation("k7m2", Some(Invitation), None), None);
        assert_eq!(validate_invitation("", Some(Open), None), None);
        assert_eq!(validate_invitation("", None, None), None);

        assert!(is_admission_error("invitation_required"));
        assert!(is_admission_error("invitation_invalid"));
        assert!(!is_admission_error("email_taken"));
    }
}
