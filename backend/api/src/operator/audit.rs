//! The operator audit trail (`operator_audit`). Every operator route except
//! the area bootstrap (`GET /api/operator`) and the feedback image
//! (`GET /api/operator/feedback/{id}/image`) writes exactly one row: a
//! mutation inside its own transaction, a read after it succeeded. There is
//! no read API; `deploy/README.md` documents the SQL.

use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::access::Operator;
use crate::error::ApiError;

/// What an operator did. Stored as text (`as_str`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    InvitationsList,
    InvitationsCreate,
    InvitationsRevoke,
    AccountsList,
    AccountsDisable,
    AccountsEnable,
    ServersList,
    ServersDelist,
    ServersRelist,
    ServersBlock,
    ServersUnblock,
    ReportsList,
    ReportsResolve,
    UsageRead,
    FeedbackList,
    FeedbackRead,
    FeedbackReply,
    FeedbackDone,
    FeedbackReopen,
    FeedbackDelete,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvitationsList => "invitations.list",
            Self::InvitationsCreate => "invitations.create",
            Self::InvitationsRevoke => "invitations.revoke",
            Self::AccountsList => "accounts.list",
            Self::AccountsDisable => "accounts.disable",
            Self::AccountsEnable => "accounts.enable",
            Self::ServersList => "servers.list",
            Self::ServersDelist => "servers.delist",
            Self::ServersRelist => "servers.relist",
            Self::ServersBlock => "servers.block",
            Self::ServersUnblock => "servers.unblock",
            Self::ReportsList => "reports.list",
            Self::ReportsResolve => "reports.resolve",
            Self::UsageRead => "usage.read",
            Self::FeedbackList => "feedback.list",
            Self::FeedbackRead => "feedback.read",
            Self::FeedbackReply => "feedback.reply",
            Self::FeedbackDone => "feedback.done",
            Self::FeedbackReopen => "feedback.reopen",
            Self::FeedbackDelete => "feedback.delete",
        }
    }
}

/// What a row names. No foreign keys: the row outlives its targets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Targets {
    pub user: Option<Uuid>,
    pub server: Option<Uuid>,
    pub report: Option<Uuid>,
    pub invitation: Option<Uuid>,
    pub feedback: Option<Uuid>,
}

/// Writes one audit row on `conn` and logs it.
pub async fn record(
    conn: &mut PgConnection,
    op: &Operator,
    action: Action,
    targets: Targets,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO operator_audit (operator_id, operator_email, action, target_user, \
             target_server, target_report, target_invitation, detail, target_feedback) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(op.user.id)
    .bind(&op.user.email)
    .bind(action.as_str())
    .bind(targets.user)
    .bind(targets.server)
    .bind(targets.report)
    .bind(targets.invitation)
    .bind(&detail)
    .bind(targets.feedback)
    .execute(&mut *conn)
    .await?;
    info!(
        operator_id = %op.user.id,
        action = action.as_str(),
        target_user = ?targets.user,
        target_server = ?targets.server,
        target_report = ?targets.report,
        target_invitation = ?targets.invitation,
        target_feedback = ?targets.feedback,
        "operator action"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_stored_as_area_dot_verb() {
        let all = [
            Action::InvitationsList,
            Action::InvitationsCreate,
            Action::InvitationsRevoke,
            Action::AccountsList,
            Action::AccountsDisable,
            Action::AccountsEnable,
            Action::ServersList,
            Action::ServersDelist,
            Action::ServersRelist,
            Action::ServersBlock,
            Action::ServersUnblock,
            Action::ReportsList,
            Action::ReportsResolve,
            Action::UsageRead,
            Action::FeedbackList,
            Action::FeedbackRead,
            Action::FeedbackReply,
            Action::FeedbackDone,
            Action::FeedbackReopen,
            Action::FeedbackDelete,
        ];
        let names: std::collections::HashSet<_> = all.iter().map(|a| a.as_str()).collect();
        assert_eq!(names.len(), all.len());
        assert!(names.contains("servers.block"));
        assert!(names.contains("usage.read"));
        assert!(names.contains("feedback.delete"));
        assert!(names.iter().all(|name| name.split('.').count() == 2));
    }
}
