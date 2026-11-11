//! Who may create an account (`API_REGISTRATION`, #517). On an open
//! instance anyone; on an invitation-only one a new account needs a beta
//! invitation (`beta_invitations`, page "Beta-Einladungen") or a server
//! invite whose server is not blocked: a group's founder gets the former,
//! and the founder's invite links admit the group.
//!
//! Registration and the OpenID Connect first sign-in call [`admit`] before
//! they create anything and [`redeem`] in the transaction that inserts the
//! account. A sign-in to an existing account needs neither.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::config::RegistrationMode;
use crate::error::ApiError;
use crate::servers::invite;
use crate::state::AppState;

/// Why a new account may be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admitted {
    /// The instance is open; codes are ignored.
    Open,
    /// This beta invitation, used up by [`redeem`].
    Invitation(Uuid),
    /// A server invite. It is not used up here: joining the server after
    /// sign-up counts it, as for anyone else.
    ServerInvite,
}

/// Checks the codes for a *new* account; `email` is canonical (lower-case,
/// ASCII domain). Empty codes count as absent. With both codes given, a
/// valid beta invitation wins; otherwise a valid server invite admits.
/// Every reason a code is not valid answers alike (`410 invitation_invalid`),
/// and nothing here looks at existing accounts.
pub async fn admit(
    state: &AppState,
    conn: &mut PgConnection,
    email: &str,
    invitation: Option<&str>,
    server_invite: Option<&str>,
) -> Result<Admitted, ApiError> {
    if state.registration == RegistrationMode::Open {
        return Ok(Admitted::Open);
    }
    let (invitation, server_invite) = (present(invitation), present(server_invite));
    if invitation.is_none() && server_invite.is_none() {
        return Err(ApiError::InvitationRequired);
    }
    if let Some(code) = invitation.and_then(invite::normalise_code)
        && let Some(id) = valid_invitation(conn, &code, email).await?
    {
        return Ok(Admitted::Invitation(id));
    }
    if let Some(code) = server_invite.and_then(invite::normalise_code)
        && valid_server_invite(conn, &code).await?
    {
        return Ok(Admitted::ServerInvite);
    }
    Err(ApiError::InvitationInvalid)
}

/// A code that is absent, empty or blank is no code.
fn present(code: Option<&str>) -> Option<&str> {
    code.map(str::trim).filter(|code| !code.is_empty())
}

/// Uses up a beta invitation, inside the transaction that inserted the
/// account `user_id`. Of two registrations racing for one code only the
/// first gets the row; the other gets `invitation_invalid` and its insert
/// rolls back.
pub async fn redeem(
    conn: &mut PgConnection,
    admitted: &Admitted,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let Admitted::Invitation(id) = admitted else {
        return Ok(());
    };
    let redeemed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE beta_invitations SET used_by = $2, used_at = now() \
         WHERE id = $1 AND used_at IS NULL AND revoked_at IS NULL RETURNING id",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    redeemed.map(|_| ()).ok_or(ApiError::InvitationInvalid)
}

/// The open beta invitation `code` for `email`, if there is one.
async fn valid_invitation(
    conn: &mut PgConnection,
    code: &str,
    email: &str,
) -> Result<Option<Uuid>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM beta_invitations \
         WHERE code = $1 AND used_at IS NULL AND revoked_at IS NULL \
           AND (expires_at IS NULL OR expires_at > now()) \
           AND (email IS NULL OR email = $2)",
    )
    .bind(code)
    .bind(email)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Whether `code` is a server invite someone could join with now (the
/// rules of `servers::invite`'s join) of a server that is not blocked.
async fn valid_server_invite(conn: &mut PgConnection, code: &str) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM invites i JOIN servers s ON s.id = i.server_id \
         WHERE i.code = $1 \
           AND (i.expires_at IS NULL OR i.expires_at > now()) \
           AND (i.max_uses IS NULL OR i.uses < i.max_uses) \
           AND s.blocked_at IS NULL)",
    )
    .bind(code)
    .fetch_one(&mut *conn)
    .await?)
}
