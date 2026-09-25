use anyhow::Context;
use claw_core::conversation_input::{
    ConversationInputDeliveryMode, ConversationInputDisposition, ConversationInputErrorCode,
    ConversationInputPreparationState, ConversationInputReceipt, ConversationInputRecord,
    ConversationInputSource, ConversationInputSubmission, OwnedConversationInputScope,
    CONVERSATION_INPUT_SCHEMA_VERSION,
};
use claw_core::types::SubmitTaskRequest;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::db_init::DbPool;

const MIGRATION_ID: &str = "018_conversation_inputs_v1";
const MIGRATION_MANIFEST: &str = include_str!("../../../../migrations/018_conversation_inputs.sql");
const TASK_CLAIM_MIGRATION_ID: &str = "019_conversation_task_creation_claims_v1";
const TASK_CLAIM_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/019_conversation_task_creation_claims.sql");
const ACTION_DISPATCH_MIGRATION_ID: &str = "020_conversation_action_dispatch_claims_v1";
const ACTION_DISPATCH_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/020_conversation_action_dispatch_claims.sql");
const TERMINAL_BOUNDARY_MIGRATION_ID: &str = "021_conversation_terminal_boundaries_v1";
const TERMINAL_BOUNDARY_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/021_conversation_terminal_boundaries.sql");
const ATTACHMENT_MIGRATION_ID: &str = "022_conversation_input_attachments_v1";
const ATTACHMENT_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/022_conversation_input_attachments.sql");
const TASK_CLAIM_FENCING_MIGRATION_ID: &str = "023_conversation_task_claim_fencing_v1";
const TASK_CLAIM_FENCING_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/023_conversation_task_claim_fencing.sql");
const TASK_TEMPLATE_MIGRATION_ID: &str = "024_conversation_task_templates_v1";
const TASK_TEMPLATE_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/024_conversation_task_templates.sql");
const DECISION_KIND_MIGRATION_ID: &str = "028_conversation_input_decision_kind_v1";
const DECISION_KIND_MIGRATION_MANIFEST: &str =
    include_str!("../../../../migrations/028_conversation_input_decision_kind.sql");
const MAX_LIST_LIMIT: u32 = 100;
const TASK_CREATION_CLAIM_TTL_SECONDS: u64 = 30;
const MAX_PENDING_INPUTS_PER_SCOPE: u64 = 128;
const MAX_PENDING_INPUT_BYTES_PER_SCOPE: u64 = 512 * 1024;

const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_schema_migrations (
    migration_id TEXT PRIMARY KEY,
    schema_digest TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS conversation_input_scopes (
    owner_principal_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    channel_account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    next_input_seq INTEGER NOT NULL DEFAULT 1,
    next_event_seq INTEGER NOT NULL DEFAULT 1,
    focus_task_id TEXT,
    instruction_revision INTEGER NOT NULL DEFAULT 0,
    execution_epoch INTEGER NOT NULL DEFAULT 0,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    PRIMARY KEY (
        owner_principal_id, agent_id, channel, channel_account_id, conversation_id
    )
);

CREATE TABLE IF NOT EXISTS conversation_inputs (
    input_id TEXT PRIMARY KEY,
    owner_principal_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    channel_account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    input_seq INTEGER NOT NULL,
    request_digest TEXT NOT NULL,
    content_json TEXT NOT NULL,
    delivery_mode TEXT NOT NULL CHECK (delivery_mode IN ('auto', 'defer')),
    preparation_state TEXT NOT NULL
        CHECK (preparation_state IN ('pending', 'ready', 'failed')),
    disposition TEXT NOT NULL
        CHECK (disposition IN (
            'pending', 'deferred', 'needs_clarification', 'applied', 'rejected', 'withdrawn'
        )),
    expected_task_id TEXT,
    expected_instruction_revision INTEGER,
    target_task_id TEXT,
    decision_ref TEXT,
    decision_kind TEXT,
    applied_checkpoint_ref TEXT,
    source_json TEXT NOT NULL,
    instruction_revision INTEGER NOT NULL,
    execution_epoch INTEGER NOT NULL,
    accepted_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    UNIQUE (
        owner_principal_id, agent_id, channel, channel_account_id,
        conversation_id, client_message_id
    ),
    UNIQUE (
        owner_principal_id, agent_id, channel, channel_account_id,
        conversation_id, input_seq
    )
);
CREATE INDEX IF NOT EXISTS idx_conversation_inputs_pending
    ON conversation_inputs(
        owner_principal_id, agent_id, channel, channel_account_id,
        conversation_id, disposition, input_seq
    );
CREATE INDEX IF NOT EXISTS idx_conversation_inputs_target
    ON conversation_inputs(target_task_id, disposition, input_seq)
    WHERE target_task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS conversation_input_events (
    owner_principal_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    channel_account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    input_id TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    PRIMARY KEY (
        owner_principal_id, agent_id, channel, channel_account_id,
        conversation_id, event_seq
    )
);
CREATE INDEX IF NOT EXISTS idx_conversation_input_events_input
    ON conversation_input_events(input_id, event_seq);

CREATE TABLE IF NOT EXISTS conversation_input_task_claims (
    owner_principal_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    channel_account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    input_id TEXT NOT NULL,
    claim_token TEXT NOT NULL DEFAULT '',
    claimed_at_ts INTEGER NOT NULL,
    expires_at_ts INTEGER NOT NULL,
    PRIMARY KEY (
        owner_principal_id, agent_id, channel, channel_account_id, conversation_id
    )
);

CREATE TABLE IF NOT EXISTS conversation_action_dispatch_claims (
    claim_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    instruction_revision INTEGER NOT NULL,
    execution_epoch INTEGER NOT NULL,
    round_no INTEGER NOT NULL,
    global_step INTEGER NOT NULL,
    action_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('claimed', 'settled_ok', 'settled_error')),
    claimed_at_ts INTEGER NOT NULL,
    settled_at_ts INTEGER,
    UNIQUE (task_id, execution_epoch, round_no, global_step, action_fingerprint)
);
CREATE INDEX IF NOT EXISTS idx_conversation_action_dispatch_task
    ON conversation_action_dispatch_claims(task_id, execution_epoch, status);

CREATE TABLE IF NOT EXISTS conversation_terminal_boundaries (
    task_id TEXT PRIMARY KEY,
    instruction_revision INTEGER NOT NULL,
    execution_epoch INTEGER NOT NULL,
    claimed_at_ts INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS conversation_input_attachments (
    attachment_id TEXT PRIMARY KEY,
    input_id TEXT NOT NULL,
    owner_principal_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    workspace_rel_path TEXT NOT NULL,
    mime_type TEXT,
    display_name TEXT,
    size_bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    UNIQUE (input_id, workspace_rel_path)
);
CREATE INDEX IF NOT EXISTS idx_conversation_input_attachments_input
    ON conversation_input_attachments(input_id, attachment_id);

CREATE TABLE IF NOT EXISTS conversation_input_task_templates (
    input_id TEXT PRIMARY KEY,
    owner_principal_id TEXT NOT NULL,
    template_json TEXT NOT NULL,
    template_digest TEXT NOT NULL,
    next_attempt_at_ts INTEGER NOT NULL DEFAULT 0,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error_code TEXT,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conversation_input_task_templates_retry
    ON conversation_input_task_templates(next_attempt_at_ts, input_id);
"#;

#[derive(Debug, Clone)]
pub(crate) struct AcceptConversationInput {
    pub(crate) owner_principal_id: String,
    pub(crate) submission: ConversationInputSubmission,
    pub(crate) preparation_state: ConversationInputPreparationState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptConversationInputOutcome {
    pub(crate) record: ConversationInputRecord,
    pub(crate) event_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationInputAttachmentBinding {
    pub(crate) attachment_id: String,
    pub(crate) kind: String,
    pub(crate) workspace_rel_path: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConversationInputTaskTemplate {
    pub(crate) schema_version: u16,
    pub(crate) task: SubmitTaskRequest,
    #[serde(default)]
    pub(crate) client_origin: Option<String>,
    #[serde(default)]
    pub(crate) execution_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecoverableConversationInputTask {
    pub(crate) scope: OwnedConversationInputScope,
    pub(crate) record: ConversationInputRecord,
    pub(crate) template: ConversationInputTaskTemplate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationInputTaskBinding {
    InitialTaskPayload,
    ExistingActiveTask,
    ActiveTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConversationInputTaskClaimOutcome {
    Bound(ConversationInputRecord),
    Creator {
        record: ConversationInputRecord,
        claim_token: Uuid,
    },
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConversationExecutionSnapshot {
    pub(crate) instruction_revision: u64,
    pub(crate) execution_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConversationActionDispatchClaimOutcome {
    Untracked,
    Claimed {
        claim_id: Uuid,
    },
    Stale {
        current: ConversationExecutionSnapshot,
        pending_input: bool,
    },
    Existing {
        claim_id: Uuid,
        status: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationTerminalBoundaryOutcome {
    Untracked,
    Claimed,
    PendingOrStale,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct ConversationInputEventRecord {
    pub(crate) schema_version: u32,
    pub(crate) event_seq: u64,
    pub(crate) input_id: Uuid,
    pub(crate) event_kind: String,
    pub(crate) payload: serde_json::Value,
    pub(crate) created_at_ts: u64,
}

#[derive(Debug, Error)]
pub(crate) enum ConversationInputStoreError {
    #[error("conversation_input_invalid_request")]
    InvalidRequest,
    #[error("conversation_input_idempotency_conflict")]
    IdempotencyConflict,
    #[error("conversation_input_target_conflict")]
    TargetConflict,
    #[error("conversation_input_capacity_exceeded")]
    CapacityExceeded,
    #[error("conversation_input_authorization_revoked")]
    AuthorizationRevoked,
    #[error("conversation_input_not_found")]
    NotFound,
    #[error("conversation_input_database_failed")]
    Database(#[source] anyhow::Error),
}

impl ConversationInputStoreError {
    pub(crate) fn code(&self) -> ConversationInputErrorCode {
        match self {
            Self::InvalidRequest => ConversationInputErrorCode::InvalidRequest,
            Self::IdempotencyConflict => ConversationInputErrorCode::IdempotencyConflict,
            Self::TargetConflict => ConversationInputErrorCode::TargetConflict,
            Self::CapacityExceeded => ConversationInputErrorCode::CapacityExceeded,
            Self::AuthorizationRevoked => ConversationInputErrorCode::Unauthorized,
            Self::NotFound => ConversationInputErrorCode::NotFound,
            Self::Database(_) => ConversationInputErrorCode::DatabaseFailed,
        }
    }
}

include!("conversation_inputs/admission.rs");

pub(crate) fn claim_or_bind_conversation_input_task(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
) -> Result<ConversationInputTaskClaimOutcome, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let record = load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    if scope_for_record(&tx, &input_id)? != *scope {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    if record.delivery_mode != ConversationInputDeliveryMode::Auto
        || record.receipt.preparation_state != ConversationInputPreparationState::Ready
        || !matches!(
            record.receipt.disposition,
            ConversationInputDisposition::Pending | ConversationInputDisposition::Applied
        )
    {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    if record.receipt.target_task_id.is_some() {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationInputTaskClaimOutcome::Bound(record));
    }

    let current_focus = load_scope_counters(&tx, scope)?.2;
    if let Some(task_id) = current_focus.as_deref() {
        if task_is_active_and_owned(&tx, task_id, &scope.owner_principal_id)? {
            let task_id = Uuid::parse_str(task_id).map_err(|error| {
                ConversationInputStoreError::Database(anyhow::anyhow!(
                    "conversation_input_focus_task_invalid:{error}"
                ))
            })?;
            bind_conversation_input_to_task_in_tx(
                &tx,
                scope,
                input_id,
                task_id,
                ConversationInputTaskBinding::ActiveTask,
            )?;
            let bound =
                load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
                    .ok_or(ConversationInputStoreError::NotFound)?;
            tx.commit().map_err(database_error)?;
            return Ok(ConversationInputTaskClaimOutcome::Bound(bound));
        }
        tx.execute(
            "UPDATE conversation_input_scopes
             SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
                 updated_at_ts = ?6
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                to_i64(crate::now_ts_u64())?,
            ],
        )
        .map_err(database_error)?;
    }

    let now_ts = crate::now_ts_u64();
    let current_claim = tx
        .query_row(
            "SELECT input_id, expires_at_ts
             FROM conversation_input_task_claims
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((claimed_input_id, expires_at_ts)) = current_claim {
        if expires_at_ts > to_i64(now_ts)? {
            tx.commit().map_err(database_error)?;
            let _ = claimed_input_id;
            return Ok(ConversationInputTaskClaimOutcome::Waiting);
        }
    }
    let creator =
        load_oldest_unbound_auto_input(&tx, scope)?.ok_or(ConversationInputStoreError::NotFound)?;
    let claim_token = Uuid::new_v4();
    tx.execute(
        "INSERT INTO conversation_input_task_claims(
            owner_principal_id, agent_id, channel, channel_account_id,
            conversation_id, input_id, claim_token, claimed_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(owner_principal_id, agent_id, channel, channel_account_id, conversation_id)
         DO UPDATE SET input_id = excluded.input_id,
                       claim_token = excluded.claim_token,
                       claimed_at_ts = excluded.claimed_at_ts,
                       expires_at_ts = excluded.expires_at_ts",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            creator.receipt.input_id.to_string(),
            claim_token.to_string(),
            to_i64(now_ts)?,
            to_i64(now_ts.saturating_add(TASK_CREATION_CLAIM_TTL_SECONDS))?,
        ],
    )
    .map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(ConversationInputTaskClaimOutcome::Creator {
        record: creator,
        claim_token,
    })
}

pub(crate) fn complete_conversation_input_task_claim(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    claim_token: Uuid,
    task_id: Uuid,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let claim_matches = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_input_task_claims
                WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                  AND channel_account_id = ?4 AND conversation_id = ?5
                  AND input_id = ?6 AND claim_token = ?7 AND expires_at_ts >= ?8
             )",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                input_id.to_string(),
                claim_token.to_string(),
                to_i64(crate::now_ts_u64())?,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?
        != 0;
    if !claim_matches {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    bind_conversation_input_to_task_in_tx(
        &tx,
        scope,
        input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )?;
    let waiting_input_ids = {
        let mut statement = tx
            .prepare(
                "SELECT input_id FROM conversation_inputs
                 WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                   AND channel_account_id = ?4 AND conversation_id = ?5
                   AND input_id != ?6 AND target_task_id IS NULL
                   AND delivery_mode = 'auto' AND preparation_state = 'ready'
                   AND disposition = 'pending'
                 ORDER BY input_seq ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(
                params![
                    scope.owner_principal_id,
                    scope.conversation.agent_id,
                    scope.conversation.channel,
                    scope.conversation.channel_account_id,
                    scope.conversation.conversation_id,
                    input_id.to_string(),
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        rows
    };
    for waiting_input_id in waiting_input_ids {
        let waiting_input_id = Uuid::parse_str(&waiting_input_id).map_err(|error| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_id_invalid:{error}"
            ))
        })?;
        bind_conversation_input_to_task_in_tx(
            &tx,
            scope,
            waiting_input_id,
            task_id,
            ConversationInputTaskBinding::ActiveTask,
        )?;
    }
    delete_task_creation_claim(&tx, scope, input_id, claim_token)?;
    let record = load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    tx.commit().map_err(database_error)?;
    Ok(record)
}

pub(crate) fn release_conversation_input_task_claim(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    claim_token: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    delete_task_creation_claim(&db, scope, input_id, claim_token)
}

fn delete_task_creation_claim(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    claim_token: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    db.execute(
        "DELETE FROM conversation_input_task_claims
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5 AND input_id = ?6
           AND claim_token = ?7",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            input_id.to_string(),
            claim_token.to_string(),
        ],
    )
    .map(|changed| changed == 1)
    .map_err(database_error)
}

fn load_oldest_unbound_auto_input(
    db: &Connection,
    scope: &OwnedConversationInputScope,
) -> Result<Option<ConversationInputRecord>, ConversationInputStoreError> {
    db.query_row(
        "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                channel_account_id, conversation_id, content_json, delivery_mode,
                preparation_state, disposition, expected_task_id,
                expected_instruction_revision, target_task_id, decision_ref,
                source_json, instruction_revision, execution_epoch,
                accepted_at_ts, updated_at_ts
         FROM conversation_inputs
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5
           AND target_task_id IS NULL AND delivery_mode = 'auto'
           AND preparation_state = 'ready' AND disposition = 'pending'
         ORDER BY input_seq ASC LIMIT 1",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
        ],
        row_to_record,
    )
    .optional()
    .map_err(database_error)
}

pub(crate) fn bind_conversation_input_to_task(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    task_id: Uuid,
    binding: ConversationInputTaskBinding,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    bind_conversation_input_to_task_in_tx(&tx, scope, input_id, task_id, binding)?;
    let record = load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    tx.commit().map_err(database_error)?;
    Ok(record)
}

pub(crate) fn task_has_pending_conversation_inputs(
    pool: &DbPool,
    task_id: &str,
) -> Result<bool, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM conversation_inputs
            WHERE target_task_id = ?1 AND preparation_state = 'ready'
              AND disposition = 'pending'
         )",
        [task_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(database_error)
}

pub(crate) fn record_conversation_input_decision(
    pool: &DbPool,
    task_id: &str,
    instruction_revision: u64,
    decision_kind: &str,
    decision_ref: &str,
) -> Result<usize, ConversationInputStoreError> {
    if task_id.trim().is_empty()
        || instruction_revision == 0
        || !valid_machine_token(decision_kind, 64)
        || !valid_machine_token(decision_ref, 256)
    {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let records = {
        let mut statement = tx
            .prepare(
                "SELECT input_id, owner_principal_id, agent_id, channel,
                        channel_account_id, conversation_id
                 FROM conversation_inputs
                 WHERE target_task_id = ?1 AND disposition = 'applied'
                   AND applied_checkpoint_ref = 'agent_loop_context'
                   AND decision_ref IS NULL AND instruction_revision <= ?2
                 ORDER BY input_seq ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![task_id, to_i64(instruction_revision)?], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    OwnedConversationInputScope {
                        owner_principal_id: row.get(1)?,
                        conversation: claw_core::conversation_input::ConversationInputScopeRef {
                            agent_id: row.get(2)?,
                            channel: row.get(3)?,
                            channel_account_id: row.get(4)?,
                            conversation_id: row.get(5)?,
                        },
                    },
                ))
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        rows
    };
    let now_ts = to_i64(crate::now_ts_u64())?;
    for (input_id, scope) in &records {
        tx.execute(
            "UPDATE conversation_inputs
             SET decision_ref = ?2, decision_kind = ?3, updated_at_ts = ?4
             WHERE input_id = ?1 AND decision_ref IS NULL",
            params![input_id, decision_ref, decision_kind, now_ts],
        )
        .map_err(database_error)?;
        append_event(
            &tx,
            scope,
            Uuid::parse_str(input_id)
                .map_err(|error| ConversationInputStoreError::Database(error.into()))?,
            "decision_recorded",
            serde_json::json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "decision_kind": decision_kind,
                "decision_ref": decision_ref,
                "instruction_revision": instruction_revision,
                "task_id": task_id,
            }),
            now_ts,
        )?;
    }
    tx.commit().map_err(database_error)?;
    Ok(records.len())
}

pub(crate) fn withdraw_conversation_input(
    pool: &DbPool,
    owner_principal_id: &str,
    input_id: Uuid,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !valid_machine_token(owner_principal_id, 256) {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let record = load_record_by_input_id(&tx, owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    if record.receipt.disposition == ConversationInputDisposition::Withdrawn {
        tx.commit().map_err(database_error)?;
        return Ok(record);
    }
    if !matches!(
        record.receipt.disposition,
        ConversationInputDisposition::Pending
            | ConversationInputDisposition::Deferred
            | ConversationInputDisposition::NeedsClarification
    ) {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let scope = scope_for_record(&tx, &input_id)?;
    let invalidates_active_execution = record.receipt.target_task_id.is_some()
        && matches!(
            record.receipt.disposition,
            ConversationInputDisposition::Pending
                | ConversationInputDisposition::NeedsClarification
        );
    let next_epoch = if invalidates_active_execution {
        let current_epoch = tx
            .query_row(
                "SELECT execution_epoch FROM conversation_input_scopes
                 WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                   AND channel_account_id = ?4 AND conversation_id = ?5",
                params![
                    scope.owner_principal_id,
                    scope.conversation.agent_id,
                    scope.conversation.channel,
                    scope.conversation.channel_account_id,
                    scope.conversation.conversation_id,
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error)?;
        current_epoch.checked_add(1).ok_or_else(|| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_epoch_overflow"
            ))
        })?
    } else {
        to_i64(record.receipt.execution_epoch)?
    };
    let now_ts = to_i64(crate::now_ts_u64())?;
    if invalidates_active_execution {
        tx.execute(
            "UPDATE conversation_input_scopes
             SET execution_epoch = ?6, updated_at_ts = ?7
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                next_epoch,
                now_ts,
            ],
        )
        .map_err(database_error)?;
    }
    let decision_ref = format!("withdraw:{input_id}:{now_ts}");
    let changed = tx
        .execute(
            "UPDATE conversation_inputs
             SET disposition = 'withdrawn', decision_ref = ?3,
                 execution_epoch = ?4, updated_at_ts = ?5
             WHERE input_id = ?1 AND owner_principal_id = ?2
               AND disposition IN ('pending', 'deferred', 'needs_clarification')",
            params![
                input_id.to_string(),
                owner_principal_id,
                decision_ref,
                next_epoch,
                now_ts,
            ],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    tx.execute(
        "DELETE FROM conversation_input_task_templates WHERE input_id = ?1",
        [input_id.to_string()],
    )
    .map_err(database_error)?;
    delete_task_creation_claim_for_input(&tx, &scope, input_id)?;
    append_event(
        &tx,
        &scope,
        input_id,
        "withdrawn",
        serde_json::json!({
            "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
            "input_id": input_id,
            "target_task_id": record.receipt.target_task_id,
            "instruction_revision": record.receipt.instruction_revision,
            "execution_epoch": next_epoch,
            "decision_ref": decision_ref,
        }),
        now_ts,
    )?;
    let withdrawn = load_record_by_input_id(&tx, owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    tx.commit().map_err(database_error)?;
    Ok(withdrawn)
}

fn delete_task_creation_claim_for_input(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    db.execute(
        "DELETE FROM conversation_input_task_claims
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5 AND input_id = ?6",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            input_id.to_string(),
        ],
    )
    .map(|changed| changed == 1)
    .map_err(database_error)
}

pub(crate) fn activate_deferred_conversation_input(
    pool: &DbPool,
    owner_principal_id: &str,
    input_id: Uuid,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !valid_machine_token(owner_principal_id, 256) {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let mut record = load_record_by_input_id(&tx, owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    if matches!(
        record.receipt.disposition,
        ConversationInputDisposition::Pending | ConversationInputDisposition::Applied
    ) && record.delivery_mode == ConversationInputDeliveryMode::Auto
    {
        tx.commit().map_err(database_error)?;
        record.receipt.replayed = true;
        return Ok(record);
    }
    if record.receipt.disposition != ConversationInputDisposition::Deferred {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let scope = scope_for_record(&tx, &input_id)?;
    let (_, _, current_focus, _, _) = load_scope_counters(&tx, &scope)?;
    let target_task_id = match current_focus.as_deref() {
        Some(task_id) if task_is_active_and_owned(&tx, task_id, owner_principal_id)? => {
            Some(task_id.to_string())
        }
        Some(_) => {
            tx.execute(
                "UPDATE conversation_input_scopes
                 SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
                     updated_at_ts = ?6
                 WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                   AND channel_account_id = ?4 AND conversation_id = ?5",
                params![
                    scope.owner_principal_id,
                    scope.conversation.agent_id,
                    scope.conversation.channel,
                    scope.conversation.channel_account_id,
                    scope.conversation.conversation_id,
                    to_i64(crate::now_ts_u64())?,
                ],
            )
            .map_err(database_error)?;
            None
        }
        None => None,
    };
    let now_ts = to_i64(crate::now_ts_u64())?;
    let changed = tx
        .execute(
            "UPDATE conversation_inputs
             SET delivery_mode = 'auto', disposition = 'pending',
                 target_task_id = ?3, decision_ref = NULL, decision_kind = NULL,
                 updated_at_ts = ?4
             WHERE input_id = ?1 AND owner_principal_id = ?2
               AND disposition = 'deferred'",
            params![
                input_id.to_string(),
                owner_principal_id,
                target_task_id,
                now_ts,
            ],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    append_event(
        &tx,
        &scope,
        input_id,
        "activated",
        serde_json::json!({
            "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
            "input_id": input_id,
            "previous_target_task_id": record.receipt.target_task_id,
            "target_task_id": target_task_id,
        }),
        now_ts,
    )?;
    let activated = load_record_by_input_id(&tx, owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    tx.commit().map_err(database_error)?;
    Ok(activated)
}

pub(crate) fn defer_pending_conversation_inputs_for_cancel(
    db: &mut Connection,
    task_id: &str,
) -> Result<usize, ConversationInputStoreError> {
    let task_id = task_id.trim();
    if !valid_machine_token(task_id, 160) {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    ensure_conversation_input_schema(db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let deferred = defer_pending_conversation_inputs_for_cancel_in_db(&tx, task_id)?;
    tx.commit().map_err(database_error)?;
    Ok(deferred)
}

pub(crate) fn defer_pending_conversation_inputs_for_cancel_in_db(
    db: &rusqlite::Transaction<'_>,
    task_id: &str,
) -> Result<usize, ConversationInputStoreError> {
    let task_id = task_id.trim();
    if !valid_machine_token(task_id, 160) {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let records = {
        let mut statement = db
            .prepare(
                "SELECT input_id, input_seq, owner_principal_id, agent_id, channel,
                        channel_account_id, conversation_id
                 FROM conversation_inputs
                 WHERE target_task_id = ?1 AND disposition = 'pending'
                 ORDER BY input_seq ASC",
            )
            .map_err(database_error)?;
        let records = statement
            .query_map([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    OwnedConversationInputScope {
                        owner_principal_id: row.get(2)?,
                        conversation: claw_core::conversation_input::ConversationInputScopeRef {
                            agent_id: row.get(3)?,
                            channel: row.get(4)?,
                            channel_account_id: row.get(5)?,
                            conversation_id: row.get(6)?,
                        },
                    },
                ))
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        records
    };
    let now_ts = to_i64(crate::now_ts_u64())?;
    let decision_ref = format!("cancel:{task_id}:{now_ts}");
    for (input_id, input_seq, scope) in &records {
        let changed = db
            .execute(
                "UPDATE conversation_inputs
                 SET disposition = 'deferred', decision_ref = ?2, updated_at_ts = ?3
                 WHERE input_id = ?1 AND target_task_id = ?4 AND disposition = 'pending'",
                params![input_id, decision_ref, now_ts, task_id],
            )
            .map_err(database_error)?;
        if changed != 1 {
            return Err(ConversationInputStoreError::TargetConflict);
        }
        append_event(
            db,
            scope,
            Uuid::parse_str(input_id)
                .map_err(|error| ConversationInputStoreError::Database(error.into()))?,
            "deferred_on_cancel",
            serde_json::json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "task_id": task_id,
                "input_seq": from_i64(*input_seq).map_err(database_error)?,
                "decision_ref": decision_ref,
            }),
            now_ts,
        )?;
    }
    db.execute(
        "UPDATE conversation_input_scopes
         SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
             updated_at_ts = ?2
         WHERE focus_task_id = ?1",
        params![task_id, now_ts],
    )
    .map_err(database_error)?;
    Ok(records.len())
}

pub(crate) fn apply_pending_conversation_inputs(
    pool: &DbPool,
    task_id: &str,
    limit: u32,
) -> Result<Vec<ConversationInputRecord>, ConversationInputStoreError> {
    let Ok(task_id) = Uuid::parse_str(task_id) else {
        return Ok(Vec::new());
    };
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let pending = load_task_records(
        &tx,
        &task_id.to_string(),
        "pending",
        limit.clamp(1, MAX_PENDING_INPUTS_PER_SCOPE as u32),
    )?;
    if pending.is_empty() {
        tx.commit().map_err(database_error)?;
        return Ok(Vec::new());
    }
    let first = &pending[0];
    let scope = scope_for_record(&tx, &first.receipt.input_id)?;
    if !scope_authorization_is_current(&tx, &scope, &task_id.to_string())? {
        reject_pending_inputs_after_authorization_revocation(
            &tx,
            &scope,
            &task_id.to_string(),
            &pending,
        )?;
        tx.commit().map_err(database_error)?;
        return Err(ConversationInputStoreError::AuthorizationRevoked);
    }
    let task_id_string = task_id.to_string();
    let current_target = tx
        .query_row(
            "SELECT focus_task_id FROM conversation_input_scopes
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(database_error)?;
    if current_target.as_deref() != Some(task_id_string.as_str())
        || !task_is_active_and_owned(&tx, &task_id_string, &scope.owner_principal_id)?
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let (current_revision, current_epoch) = tx
        .query_row(
            "SELECT instruction_revision, execution_epoch FROM conversation_input_scopes
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(database_error)?;
    let next_revision = current_revision.checked_add(1).ok_or_else(|| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_input_revision_overflow"
        ))
    })?;
    let next_epoch = current_epoch.checked_add(1).ok_or_else(|| {
        ConversationInputStoreError::Database(anyhow::anyhow!("conversation_input_epoch_overflow"))
    })?;
    let now_ts = to_i64(crate::now_ts_u64())?;
    tx.execute(
        "UPDATE conversation_input_scopes
         SET instruction_revision = ?6, execution_epoch = ?7, updated_at_ts = ?8
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            next_revision,
            next_epoch,
            now_ts,
        ],
    )
    .map_err(database_error)?;
    let mut applied = pending;
    for record in &mut applied {
        let changed = tx
            .execute(
                "UPDATE conversation_inputs
                 SET disposition = 'applied', instruction_revision = ?3,
                     execution_epoch = ?4, applied_checkpoint_ref = 'agent_loop_context',
                     updated_at_ts = ?5
                 WHERE input_id = ?1 AND target_task_id = ?2
                   AND preparation_state = 'ready' AND disposition = 'pending'",
                params![
                    record.receipt.input_id.to_string(),
                    task_id.to_string(),
                    next_revision,
                    next_epoch,
                    now_ts,
                ],
            )
            .map_err(database_error)?;
        if changed != 1 {
            return Err(ConversationInputStoreError::TargetConflict);
        }
        append_event(
            &tx,
            &scope,
            record.receipt.input_id,
            "applied",
            serde_json::json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "input_id": record.receipt.input_id,
                "task_id": task_id,
                "instruction_revision": next_revision,
                "execution_epoch": next_epoch,
            }),
            now_ts,
        )?;
        record.receipt.disposition = ConversationInputDisposition::Applied;
        record.receipt.instruction_revision = u64::try_from(next_revision).map_err(|error| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_revision_invalid:{error}"
            ))
        })?;
        record.receipt.execution_epoch = u64::try_from(next_epoch).map_err(|error| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_epoch_invalid:{error}"
            ))
        })?;
        record.receipt.updated_at_ts = u64::try_from(now_ts).map_err(|error| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_timestamp_invalid:{error}"
            ))
        })?;
    }
    tx.commit().map_err(database_error)?;
    Ok(applied)
}

pub(crate) fn applied_conversation_inputs_for_task(
    pool: &DbPool,
    task_id: &str,
    after_input_seq: u64,
    limit: u32,
) -> Result<Vec<ConversationInputRecord>, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    load_task_records_after_seq(
        &db,
        task_id,
        "applied",
        "agent_loop_context",
        after_input_seq,
        limit.clamp(1, MAX_LIST_LIMIT),
    )
}

pub(crate) fn conversation_execution_snapshot_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<ConversationExecutionSnapshot>, ConversationInputStoreError> {
    let Ok(task_id) = Uuid::parse_str(task_id).map(|task_id| task_id.to_string()) else {
        return Ok(None);
    };
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    execution_snapshot_for_task_in_db(&db, &task_id)
}

pub(crate) fn conversation_presentation_snapshot_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<ConversationExecutionSnapshot>, ConversationInputStoreError> {
    let Ok(task_id) = Uuid::parse_str(task_id).map(|task_id| task_id.to_string()) else {
        return Ok(None);
    };
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    db.query_row(
        "SELECT instruction_revision, execution_epoch
         FROM conversation_input_scopes WHERE focus_task_id = ?1
         UNION ALL
         SELECT instruction_revision, execution_epoch
         FROM conversation_terminal_boundaries WHERE task_id = ?1
         LIMIT 1",
        [&task_id],
        |row| {
            Ok(ConversationExecutionSnapshot {
                instruction_revision: from_i64(row.get(0)?)?,
                execution_epoch: from_i64(row.get(1)?)?,
            })
        },
    )
    .optional()
    .map_err(database_error)
}

pub(crate) fn claim_conversation_terminal_boundary(
    pool: &DbPool,
    task_id: &str,
    expected_instruction_revision: u64,
    expected_execution_epoch: u64,
) -> Result<ConversationTerminalBoundaryOutcome, ConversationInputStoreError> {
    let Ok(task_id) = Uuid::parse_str(task_id).map(|task_id| task_id.to_string()) else {
        return Ok(ConversationTerminalBoundaryOutcome::Untracked);
    };
    let outcome = crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || -> anyhow::Result<ConversationTerminalBoundaryOutcome> {
            let mut db = pool.get().context("conversation_input_db_pool_failed")?;
            ensure_conversation_input_schema(&db)?;
            Ok(claim_conversation_terminal_boundary_in_db(
                &mut db,
                &task_id,
                expected_instruction_revision,
                expected_execution_epoch,
            )?)
        },
    );
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => match error.downcast::<ConversationInputStoreError>() {
            Ok(error) => Err(error),
            Err(error) => Err(ConversationInputStoreError::Database(error)),
        },
    }
}

fn claim_conversation_terminal_boundary_in_db(
    db: &mut Connection,
    task_id: &str,
    expected_instruction_revision: u64,
    expected_execution_epoch: u64,
) -> Result<ConversationTerminalBoundaryOutcome, ConversationInputStoreError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let existing = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_terminal_boundaries WHERE task_id = ?1
             )",
            [&task_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?
        != 0;
    if existing {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationTerminalBoundaryOutcome::Claimed);
    }
    let Some(current) = execution_snapshot_for_task_in_db(&tx, &task_id)? else {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationTerminalBoundaryOutcome::Untracked);
    };
    let pending_input = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_inputs
                WHERE target_task_id = ?1 AND preparation_state = 'ready'
                  AND disposition = 'pending'
             )",
            [&task_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?
        != 0;
    if pending_input
        || current.instruction_revision != expected_instruction_revision
        || current.execution_epoch != expected_execution_epoch
    {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationTerminalBoundaryOutcome::PendingOrStale);
    }
    tx.execute(
        "INSERT INTO conversation_terminal_boundaries(
            task_id, instruction_revision, execution_epoch, claimed_at_ts
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            task_id,
            to_i64(expected_instruction_revision)?,
            to_i64(expected_execution_epoch)?,
            to_i64(crate::now_ts_u64())?,
        ],
    )
    .map_err(database_error)?;
    let changed = tx
        .execute(
            "UPDATE conversation_input_scopes
             SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
                 updated_at_ts = ?4
             WHERE focus_task_id = ?1 AND instruction_revision = ?2
               AND execution_epoch = ?3",
            params![
                task_id,
                to_i64(expected_instruction_revision)?,
                to_i64(expected_execution_epoch)?,
                to_i64(crate::now_ts_u64())?,
            ],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err(ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_terminal_boundary_scope_conflict"
        )));
    }
    tx.commit().map_err(database_error)?;
    Ok(ConversationTerminalBoundaryOutcome::Claimed)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn claim_conversation_action_dispatch(
    pool: &DbPool,
    task_id: &str,
    expected_instruction_revision: u64,
    expected_execution_epoch: u64,
    round_no: usize,
    global_step: usize,
    action_fingerprint: &str,
) -> Result<ConversationActionDispatchClaimOutcome, ConversationInputStoreError> {
    let Ok(task_id) = Uuid::parse_str(task_id).map(|task_id| task_id.to_string()) else {
        return Ok(ConversationActionDispatchClaimOutcome::Untracked);
    };
    let action_fingerprint = action_fingerprint.trim();
    if action_fingerprint.is_empty() || action_fingerprint.len() > 4096 {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let Some(current) = execution_snapshot_for_task_in_db(&tx, &task_id)? else {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationActionDispatchClaimOutcome::Untracked);
    };
    let pending_input = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_inputs
                WHERE target_task_id = ?1 AND preparation_state = 'ready'
                  AND disposition = 'pending'
             )",
            [&task_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?
        != 0;
    if pending_input
        || current.instruction_revision != expected_instruction_revision
        || current.execution_epoch != expected_execution_epoch
    {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationActionDispatchClaimOutcome::Stale {
            current,
            pending_input,
        });
    }
    let round_no = i64::try_from(round_no).map_err(|error| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_action_round_invalid:{error}"
        ))
    })?;
    let global_step = i64::try_from(global_step).map_err(|error| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_action_step_invalid:{error}"
        ))
    })?;
    let expected_execution_epoch = to_i64(expected_execution_epoch)?;
    let existing = tx
        .query_row(
            "SELECT claim_id, status FROM conversation_action_dispatch_claims
             WHERE task_id = ?1 AND execution_epoch = ?2 AND round_no = ?3
               AND global_step = ?4 AND action_fingerprint = ?5",
            params![
                task_id,
                expected_execution_epoch,
                round_no,
                global_step,
                action_fingerprint,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((claim_id, status)) = existing {
        tx.commit().map_err(database_error)?;
        return Ok(ConversationActionDispatchClaimOutcome::Existing {
            claim_id: Uuid::parse_str(&claim_id).map_err(|error| {
                ConversationInputStoreError::Database(anyhow::anyhow!(
                    "conversation_action_claim_id_invalid:{error}"
                ))
            })?,
            status,
        });
    }
    let claim_id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO conversation_action_dispatch_claims(
            claim_id, task_id, instruction_revision, execution_epoch, round_no,
            global_step, action_fingerprint, status, claimed_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'claimed', ?8)",
        params![
            claim_id.to_string(),
            task_id,
            to_i64(expected_instruction_revision)?,
            expected_execution_epoch,
            round_no,
            global_step,
            action_fingerprint,
            to_i64(crate::now_ts_u64())?,
        ],
    )
    .map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(ConversationActionDispatchClaimOutcome::Claimed { claim_id })
}

pub(crate) fn settle_conversation_action_dispatch(
    pool: &DbPool,
    claim_id: Uuid,
    succeeded: bool,
) -> Result<(), ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let changed = db
        .execute(
            "UPDATE conversation_action_dispatch_claims
             SET status = ?2, settled_at_ts = ?3
             WHERE claim_id = ?1 AND status = 'claimed'",
            params![
                claim_id.to_string(),
                if succeeded {
                    "settled_ok"
                } else {
                    "settled_error"
                },
                to_i64(crate::now_ts_u64())?,
            ],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    Ok(())
}

pub(crate) fn wake_task_for_pending_conversation_input(
    pool: &DbPool,
    task_id: Uuid,
    input_id: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let task_id = task_id.to_string();
    let input_id = input_id.to_string();
    let input_revision = tx
        .query_row(
            "SELECT instruction_revision
             FROM conversation_inputs
             WHERE input_id = ?1 AND target_task_id = ?2
               AND preparation_state = 'ready' AND disposition = 'pending'",
            params![input_id, task_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(database_error)?;
    let Some(input_revision) = input_revision else {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    };
    let raw_result = tx
        .query_row(
            "SELECT result_json FROM tasks
             WHERE task_id = ?1 AND status = 'running'",
            [&task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(database_error)?
        .flatten();
    let Some(raw_result) = raw_result else {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    };
    let mut result = match serde_json::from_str::<Value>(&raw_result) {
        Ok(result) if result.is_object() => result,
        _ => {
            tx.commit().map_err(database_error)?;
            return Ok(false);
        }
    };
    let now_ts = to_i64(crate::now_ts_u64())?;
    if !matches!(
        crate::task_lifecycle::paused_checkpoint_resume_readiness(&result, now_ts),
        crate::task_lifecycle::PausedCheckpointResumeReadiness::WaitingNotDue { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::Ready { .. }
    ) {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    }
    let Some(mut checkpoint) = crate::task_lifecycle::task_checkpoint_from_result_json(&result)
    else {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    };
    if !matches!(
        checkpoint.resume_entrypoint,
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound
            | crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput
    ) {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    }
    checkpoint.resume_entrypoint = crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound;
    let checkpoint_id = checkpoint.checkpoint_id.clone();
    let checkpoint_value = serde_json::to_value(&checkpoint).map_err(json_error)?;
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result), None);
    let Some(lifecycle_object) = lifecycle.as_object_mut() else {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    };
    lifecycle_object.insert("state".to_string(), json!("waiting"));
    lifecycle_object.insert("source".to_string(), json!("conversation_input"));
    lifecycle_object.insert("checkpoint_id".to_string(), json!(checkpoint_id));
    lifecycle_object.insert("next_check_after".to_string(), json!(now_ts));
    lifecycle_object.insert("resume_due".to_string(), json!(true));
    lifecycle_object.insert("resume_wait_seconds".to_string(), json!(0));
    lifecycle_object.insert(
        "resume_input".to_string(),
        json!({
            "schema_version": 1,
            "kind": "conversation_input",
            "input_id": input_id,
            "instruction_revision": input_revision,
            "resume_trigger": "user_followup",
        }),
    );
    result["task_checkpoint"] = checkpoint_value;
    result["task_lifecycle"] = lifecycle;
    let changed = tx
        .execute(
            "UPDATE tasks SET result_json = ?2, updated_at = ?3
             WHERE task_id = ?1 AND status = 'running' AND result_json = ?4",
            params![task_id, result.to_string(), now_ts.to_string(), raw_result],
        )
        .map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(changed == 1)
}

fn execution_snapshot_for_task_in_db(
    db: &Connection,
    task_id: &str,
) -> Result<Option<ConversationExecutionSnapshot>, ConversationInputStoreError> {
    db.query_row(
        "SELECT instruction_revision, execution_epoch
         FROM conversation_input_scopes
         WHERE focus_task_id = ?1",
        [task_id],
        |row| {
            Ok(ConversationExecutionSnapshot {
                instruction_revision: from_i64(row.get(0)?)?,
                execution_epoch: from_i64(row.get(1)?)?,
            })
        },
    )
    .optional()
    .map_err(database_error)
}

fn bind_conversation_input_to_task_in_tx(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    task_id: Uuid,
    binding: ConversationInputTaskBinding,
) -> Result<(), ConversationInputStoreError> {
    let task_id = task_id.to_string();
    let (task_status, task_owner) = tx
        .query_row(
            "SELECT status, principal_id FROM tasks WHERE task_id = ?1",
            [&task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or(ConversationInputStoreError::TargetConflict)?;
    if task_owner.as_deref() != Some(scope.owner_principal_id.as_str()) {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let task_active = matches!(task_status.as_str(), "queued" | "running");
    if matches!(
        binding,
        ConversationInputTaskBinding::ExistingActiveTask | ConversationInputTaskBinding::ActiveTask
    ) && !task_active
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let input_scope = scope_for_record(tx, &input_id)?;
    if &input_scope != scope {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let now_ts = to_i64(crate::now_ts_u64())?;
    let (current_focus, current_revision, current_epoch) = tx
        .query_row(
            "SELECT focus_task_id, instruction_revision, execution_epoch
             FROM conversation_input_scopes
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(database_error)?;
    let set_focus = task_active;
    let next_epoch = if set_focus && current_focus.as_deref() != Some(task_id.as_str()) {
        current_epoch.checked_add(1).ok_or_else(|| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_epoch_overflow"
            ))
        })?
    } else {
        current_epoch
    };
    let initial_payload = matches!(
        binding,
        ConversationInputTaskBinding::InitialTaskPayload
            | ConversationInputTaskBinding::ExistingActiveTask
    );
    let next_revision = if initial_payload {
        current_revision.checked_add(1).ok_or_else(|| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_revision_overflow"
            ))
        })?
    } else {
        current_revision
    };
    tx.execute(
        "UPDATE conversation_input_scopes
         SET focus_task_id = CASE WHEN ?6 = 1 THEN ?7 ELSE focus_task_id END,
             instruction_revision = ?8, execution_epoch = ?9, updated_at_ts = ?10
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            if set_focus { 1_i64 } else { 0_i64 },
            task_id,
            next_revision,
            next_epoch,
            now_ts,
        ],
    )
    .map_err(database_error)?;
    let (disposition, checkpoint_ref) = match binding {
        ConversationInputTaskBinding::InitialTaskPayload
        | ConversationInputTaskBinding::ExistingActiveTask => {
            ("applied", Some("initial_task_payload"))
        }
        ConversationInputTaskBinding::ActiveTask => ("pending", None),
    };
    let changed = tx
        .execute(
            "UPDATE conversation_inputs
             SET target_task_id = ?2, disposition = ?3, applied_checkpoint_ref = ?4,
                 instruction_revision = ?5, execution_epoch = ?6, updated_at_ts = ?7
             WHERE input_id = ?1 AND owner_principal_id = ?8
               AND delivery_mode = 'auto' AND preparation_state = 'ready'
               AND disposition = 'pending'
               AND (target_task_id IS NULL OR target_task_id = ?2)",
            params![
                input_id.to_string(),
                task_id,
                disposition,
                checkpoint_ref,
                next_revision,
                next_epoch,
                now_ts,
                scope.owner_principal_id,
            ],
        )
        .map_err(database_error)?;
    if changed != 1 {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    tx.execute(
        "DELETE FROM conversation_input_task_templates WHERE input_id = ?1",
        [input_id.to_string()],
    )
    .map_err(database_error)?;
    append_event(
        tx,
        scope,
        input_id,
        "task_bound",
        serde_json::json!({
            "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
            "input_id": input_id,
            "task_id": task_id,
            "binding": match binding {
                ConversationInputTaskBinding::InitialTaskPayload => "initial_task_payload",
                ConversationInputTaskBinding::ExistingActiveTask => "existing_active_task",
                ConversationInputTaskBinding::ActiveTask => "active_task",
            },
            "instruction_revision": next_revision,
            "execution_epoch": next_epoch,
        }),
        now_ts,
    )?;
    Ok(())
}

include!("conversation_inputs/storage_support.rs");
include!("conversation_inputs/attachment_storage.rs");
include!("conversation_inputs/authorization.rs");
include!("conversation_inputs/focus.rs");

#[cfg(test)]
#[path = "conversation_inputs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "conversation_inputs/attachment_storage_tests.rs"]
mod attachment_storage_tests;
