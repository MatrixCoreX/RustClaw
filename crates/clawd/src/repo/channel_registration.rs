// This journal fences effects; it never grants actor/session authorization.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::auth::PendingChannelBindSession;

const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS channel_configuration_revisions (
    channel TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision >= 0)
);
CREATE TABLE IF NOT EXISTS channel_registration_applications (
    flow_digest TEXT PRIMARY KEY,
    session_id INTEGER NOT NULL UNIQUE REFERENCES pending_channel_bind_sessions(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    revision INTEGER NOT NULL,
    config_digest TEXT NOT NULL,
    device_digest TEXT,
    phase TEXT NOT NULL CHECK (phase IN ('prepared', 'claimed', 'applied', 'uncertain')),
    claim_token TEXT,
    outcome_code TEXT,
    updated_at INTEGER NOT NULL
);
"#;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum RegistrationJournalError {
    #[error("channel_registration_identity_invalid")]
    InvalidIdentity,
    #[error("channel_registration_digest_invalid")]
    InvalidDigest,
    #[error("channel_registration_already_prepared")]
    AlreadyPrepared,
    #[error("channel_registration_superseded")]
    Superseded,
    #[error("channel_registration_claim_lost")]
    ClaimLost,
    #[error("channel_configuration_revision_exhausted")]
    RevisionExhausted,
    #[error("channel_registration_journal_phase_invalid")]
    InvalidPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrationGate {
    Ready { config_digest: String },
    Applied,
    RecoveryRequired,
    Superseded,
    Unprepared,
}

#[derive(Debug, Clone)]
pub(crate) struct RegistrationClaim {
    flow_digest: String,
    token: String,
}

#[derive(Debug)]
pub(crate) enum RegistrationClaimOutcome {
    Acquired(RegistrationClaim),
    Gate(RegistrationGate),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RegistrationOutcome {
    ServiceStarted,
    ServiceStartFailed,
    ConfigurationWriteFailed,
}

impl RegistrationOutcome {
    fn tokens(self) -> (&'static str, &'static str) {
        match self {
            Self::ServiceStarted => ("applied", "service_started"),
            Self::ServiceStartFailed => ("applied", "service_start_failed"),
            Self::ConfigurationWriteFailed => ("uncertain", "configuration_write_failed"),
        }
    }
}

fn ensure_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_SQL)?;
    Ok(())
}

fn flow_digest(session: &PendingChannelBindSession) -> anyhow::Result<String> {
    if session.id <= 0 || session.channel.trim().is_empty() || session.bind_token.trim().is_empty()
    {
        return Err(RegistrationJournalError::InvalidIdentity.into());
    }
    let identity = serde_json::to_vec(&(session.id, &session.channel, &session.bind_token))?;
    Ok(hex_digest(&identity))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_digest(value: &str) -> anyhow::Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistrationJournalError::InvalidDigest.into());
    }
    Ok(())
}

fn revision(db: &Connection, channel: &str) -> anyhow::Result<i64> {
    Ok(db.query_row(
        "SELECT revision FROM channel_configuration_revisions WHERE channel = ?1",
        [channel],
        |row| row.get(0),
    )?)
}

fn bump_revision(db: &Connection, channel: &str) -> anyhow::Result<i64> {
    db.execute(
        "INSERT OR IGNORE INTO channel_configuration_revisions(channel, revision) VALUES (?1, 0)",
        [channel],
    )?;
    let changed = db.execute(
        "UPDATE channel_configuration_revisions SET revision = revision + 1
         WHERE channel = ?1 AND revision < 9223372036854775807",
        [channel],
    )?;
    if changed != 1 {
        return Err(RegistrationJournalError::RevisionExhausted.into());
    }
    revision(db, channel)
}

pub(crate) fn advance_configuration_revision(
    db: &mut Connection,
    channel: &str,
) -> anyhow::Result<i64> {
    ensure_schema(db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = bump_revision(&tx, channel)?;
    tx.commit()?;
    Ok(next)
}

pub(crate) fn configuration_requires_recovery(
    db: &Connection,
    channel: &str,
) -> anyhow::Result<bool> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table'
         AND name = 'channel_registration_applications')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_registration_applications AS application
         JOIN channel_configuration_revisions AS configuration
           ON configuration.channel = application.channel
          AND configuration.revision = application.revision
         WHERE application.channel = ?1 AND application.phase IN ('claimed', 'uncertain'))",
        [channel],
        |row| row.get(0),
    )?)
}

pub(crate) fn prepare_registration(
    db: &mut Connection,
    session: &PendingChannelBindSession,
    config_digest: &str,
) -> anyhow::Result<()> {
    let flow = flow_digest(session)?;
    validate_digest(config_digest)?;
    ensure_schema(db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_registration_applications WHERE session_id = ?1)",
        [session.id],
        |row| row.get(0),
    )?;
    if exists {
        return Err(RegistrationJournalError::AlreadyPrepared.into());
    }
    let revision = bump_revision(&tx, &session.channel)?;
    tx.execute(
        "INSERT INTO channel_registration_applications(
             flow_digest, session_id, channel, revision, config_digest, phase, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', ?6)",
        params![
            flow,
            session.id,
            session.channel,
            revision,
            config_digest,
            crate::now_ts_u64() as i64
        ],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn validate_registration_preparation(
    db: &Connection,
    session: &PendingChannelBindSession,
    observed_config_digest: &str,
) -> anyhow::Result<()> {
    let flow = flow_digest(session)?;
    validate_digest(observed_config_digest)?;
    ensure_schema(db)?;
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_registration_applications
         WHERE flow_digest = ?1 AND phase = 'prepared' AND device_digest IS NULL
           AND config_digest = ?2
           AND revision = (SELECT revision FROM channel_configuration_revisions WHERE channel = ?3))",
        params![flow, observed_config_digest, session.channel], |row| row.get(0),
    )?;
    if !valid {
        return Err(RegistrationJournalError::Superseded.into());
    }
    Ok(())
}

pub(crate) fn record_registration_device(
    db: &mut Connection,
    session: &PendingChannelBindSession,
    device_code: &str,
    observed_config_digest: &str,
) -> anyhow::Result<()> {
    let flow = flow_digest(session)?;
    validate_digest(observed_config_digest)?;
    if device_code.trim().is_empty() {
        return Err(RegistrationJournalError::InvalidIdentity.into());
    }
    ensure_schema(db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE channel_registration_applications SET device_digest = ?2, updated_at = ?3
         WHERE flow_digest = ?1 AND phase = 'prepared' AND config_digest = ?4
           AND (device_digest IS NULL OR device_digest = ?2)
           AND revision = (SELECT revision FROM channel_configuration_revisions WHERE channel = ?5)",
        params![flow, hex_digest(device_code.trim().as_bytes()), crate::now_ts_u64() as i64,
            observed_config_digest, session.channel],
    )?;
    if changed != 1 {
        return Err(RegistrationJournalError::Superseded.into());
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn registration_gate(
    db: &Connection,
    session: &PendingChannelBindSession,
) -> anyhow::Result<RegistrationGate> {
    let flow = flow_digest(session)?;
    ensure_schema(db)?;
    let row = db
        .query_row(
            "SELECT phase, config_digest, device_digest, revision
         FROM channel_registration_applications WHERE flow_digest = ?1",
            [flow],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((phase, config_digest, device_digest, prepared_revision)) = row else {
        return Ok(RegistrationGate::Unprepared);
    };
    match phase.as_str() {
        "applied" => return Ok(RegistrationGate::Applied),
        "claimed" | "uncertain" => return Ok(RegistrationGate::RecoveryRequired),
        "prepared" => {}
        _ => return Err(RegistrationJournalError::InvalidPhase.into()),
    }
    let expected_device = session
        .install_device_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| hex_digest(value.as_bytes()));
    if expected_device.is_none()
        || expected_device != device_digest
        || revision(db, &session.channel)? != prepared_revision
    {
        return Ok(RegistrationGate::Superseded);
    }
    Ok(RegistrationGate::Ready { config_digest })
}

pub(crate) fn claim_registration_application(
    db: &mut Connection,
    session: &PendingChannelBindSession,
    observed_config_digest: &str,
) -> anyhow::Result<RegistrationClaimOutcome> {
    validate_digest(observed_config_digest)?;
    ensure_schema(db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let gate = registration_gate(&tx, session)?;
    match gate {
        RegistrationGate::Ready { config_digest } if config_digest == observed_config_digest => {}
        RegistrationGate::Ready { .. } => {
            return Ok(RegistrationClaimOutcome::Gate(RegistrationGate::Superseded));
        }
        gate => return Ok(RegistrationClaimOutcome::Gate(gate)),
    }
    let claim = RegistrationClaim {
        flow_digest: flow_digest(session)?,
        token: uuid::Uuid::new_v4().to_string(),
    };
    let changed = tx.execute(
        "UPDATE channel_registration_applications SET phase = 'claimed', claim_token = ?2,
             updated_at = ?3 WHERE flow_digest = ?1 AND phase = 'prepared'",
        params![claim.flow_digest, claim.token, crate::now_ts_u64() as i64],
    )?;
    if changed != 1 {
        return Err(RegistrationJournalError::ClaimLost.into());
    }
    tx.commit()?;
    Ok(RegistrationClaimOutcome::Acquired(claim))
}

pub(crate) fn finish_registration_application(
    db: &Connection,
    claim: &RegistrationClaim,
    outcome: RegistrationOutcome,
) -> anyhow::Result<()> {
    let (phase, outcome) = outcome.tokens();
    let changed = db.execute(
        "UPDATE channel_registration_applications SET phase = ?3, outcome_code = ?4,
             updated_at = ?5 WHERE flow_digest = ?1 AND claim_token = ?2 AND phase = 'claimed'",
        params![
            claim.flow_digest,
            claim.token,
            phase,
            outcome,
            crate::now_ts_u64() as i64
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let already_finished: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_registration_applications
         WHERE flow_digest = ?1 AND claim_token = ?2 AND phase = ?3 AND outcome_code = ?4)",
        params![claim.flow_digest, claim.token, phase, outcome],
        |row| row.get(0),
    )?;
    if !already_finished {
        return Err(RegistrationJournalError::ClaimLost.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_registration_tests.rs"]
mod tests;
