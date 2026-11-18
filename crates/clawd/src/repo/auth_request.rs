use claw_core::types::AuthIdentity;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RequestIdentityError {
    Revoked,
    Changed,
}

impl RequestIdentityError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Revoked => "request_credential_revoked",
            Self::Changed => "request_identity_changed",
        }
    }
}

impl std::fmt::Display for RequestIdentityError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(self.code())
    }
}

impl std::error::Error for RequestIdentityError {}

/// Reuse authentication's live key/binding/principal query. Another surviving
/// account credential cannot authorize this original request, nor can a stale role.
pub(crate) fn require_request_identity(
    db: &Connection,
    expected: &AuthIdentity,
) -> anyhow::Result<()> {
    match super::auth_principal::active_auth_key_identity(db, &expected.user_key)? {
        Some((role, principal)) if role == expected.role && principal == expected.principal_id => {
            Ok(())
        }
        Some(_) => Err(RequestIdentityError::Changed.into()),
        None => Err(RequestIdentityError::Revoked.into()),
    }
}
