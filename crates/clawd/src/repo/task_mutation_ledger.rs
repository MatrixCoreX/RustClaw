use anyhow::{anyhow, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db_init::DbPool;

const INIT_TASK_MUTATION_LEDGER_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_mutation_ledger (
    task_id            TEXT NOT NULL,
    fingerprint_hash   TEXT NOT NULL,
    action_ref         TEXT NOT NULL,
    phase              TEXT NOT NULL CHECK (phase IN (
        'intent_recorded',
        'attempt_started',
        'receipt_recorded',
        'verification_pending',
        'verified',
        'reconciliation_required',
        'reconciled',
        'committed'
    )),
    idempotency_key    TEXT NOT NULL,
    attempt_no         INTEGER NOT NULL DEFAULT 0,
    execution_token    TEXT NOT NULL,
    lease_owner        TEXT NOT NULL,
    claim_attempt      INTEGER NOT NULL,
    receipt_hash       TEXT,
    receipt_json       TEXT,
    verification_json  TEXT,
    reconciliation_json TEXT,
    started_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    receipt_at         INTEGER,
    verified_at        INTEGER,
    reconciled_at      INTEGER,
    committed_at       INTEGER,
    PRIMARY KEY (task_id, fingerprint_hash)
);
CREATE INDEX IF NOT EXISTS idx_task_mutation_ledger_phase_updated
    ON task_mutation_ledger(phase, updated_at);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskMutationPhase {
    IntentRecorded,
    AttemptStarted,
    ReceiptRecorded,
    VerificationPending,
    Verified,
    ReconciliationRequired,
    Reconciled,
    Committed,
}

impl TaskMutationPhase {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::IntentRecorded => "intent_recorded",
            Self::AttemptStarted => "attempt_started",
            Self::ReceiptRecorded => "receipt_recorded",
            Self::VerificationPending => "verification_pending",
            Self::Verified => "verified",
            Self::ReconciliationRequired => "reconciliation_required",
            Self::Reconciled => "reconciled",
            Self::Committed => "committed",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "intent_recorded" => Ok(Self::IntentRecorded),
            "attempt_started" => Ok(Self::AttemptStarted),
            "receipt_recorded" => Ok(Self::ReceiptRecorded),
            "verification_pending" => Ok(Self::VerificationPending),
            "verified" => Ok(Self::Verified),
            "reconciliation_required" => Ok(Self::ReconciliationRequired),
            "reconciled" => Ok(Self::Reconciled),
            "committed" => Ok(Self::Committed),
            other => Err(anyhow!("task_mutation_phase_unsupported:{other}")),
        }
    }

    pub(crate) fn suppresses_replay(self) -> bool {
        matches!(
            self,
            Self::ReceiptRecorded
                | Self::VerificationPending
                | Self::Verified
                | Self::Reconciled
                | Self::Committed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationRecord {
    pub(crate) task_id: String,
    pub(crate) fingerprint_hash: String,
    pub(crate) action_ref: String,
    pub(crate) phase: TaskMutationPhase,
    pub(crate) idempotency_key: String,
    pub(crate) attempt_no: i64,
    pub(crate) receipt: Option<Value>,
    pub(crate) verification: Option<Value>,
    pub(crate) reconciliation: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationLease {
    pub(crate) record: TaskMutationRecord,
    pub(crate) execution_token: String,
    pub(crate) lease_owner: String,
    pub(crate) claim_attempt: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationClaimRejected {
    pub(crate) status_code: &'static str,
    pub(crate) task_id: String,
    pub(crate) expected_lease_owner: String,
    pub(crate) expected_claim_attempt: i64,
    pub(crate) task_status: Option<String>,
    pub(crate) active_lease_owner: Option<String>,
    pub(crate) active_claim_attempt: Option<i64>,
}

impl std::fmt::Display for TaskMutationClaimRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "status_code={} task_id={} expected_lease_owner={} expected_claim_attempt={} task_status={} active_lease_owner={} active_claim_attempt={}",
            self.status_code,
            self.task_id,
            self.expected_lease_owner,
            self.expected_claim_attempt,
            self.task_status.as_deref().unwrap_or("missing"),
            self.active_lease_owner.as_deref().unwrap_or("none"),
            self.active_claim_attempt
                .map(|value| value.to_string())
                .as_deref()
                .unwrap_or("none")
        )
    }
}

impl std::error::Error for TaskMutationClaimRejected {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BeginTaskMutationOutcome {
    Acquired(TaskMutationLease),
    ReplaySuppressed(TaskMutationRecord),
    ReconciliationRequired(TaskMutationRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskMutationReconciliation {
    Applied,
    NotApplied,
    StillUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReconcileTaskMutationOutcome {
    RetryReady(TaskMutationLease),
    Reconciled(TaskMutationLease),
    ReplaySuppressed(TaskMutationRecord),
    Waiting(TaskMutationRecord),
}

pub(crate) fn begin_task_mutation(
    pool: &DbPool,
    lease_owner: &str,
    claim_attempt: i64,
    task_id: &str,
    action_fingerprint: &str,
    action_ref: &str,
) -> anyhow::Result<BeginTaskMutationOutcome> {
    let task_id = required_value(task_id, "task_id")?;
    let lease_owner = required_value(lease_owner, "lease_owner")?;
    let action_fingerprint = required_value(action_fingerprint, "action_fingerprint")?;
    let action_ref = required_value(action_ref, "action_ref")?;
    let fingerprint_hash = sha256_hex(action_fingerprint.as_bytes());
    let idempotency_key = mutation_idempotency_key(task_id, &fingerprint_hash);
    let execution_token = uuid::Uuid::new_v4().to_string();
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(&tx, task_id, lease_owner, claim_attempt)?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO task_mutation_ledger (
             task_id, fingerprint_hash, action_ref, phase, idempotency_key,
             attempt_no, execution_token, lease_owner, claim_attempt,
             receipt_hash, receipt_json, verification_json,
             reconciliation_json, started_at, updated_at, receipt_at,
             verified_at, reconciled_at, committed_at
         ) VALUES (
             ?1, ?2, ?3, 'intent_recorded', ?4, 0, ?5, ?6, ?7,
             NULL, NULL, NULL, NULL, ?8, ?8, NULL, NULL, NULL, NULL
         )",
        params![
            task_id,
            fingerprint_hash,
            action_ref,
            idempotency_key,
            execution_token,
            lease_owner,
            claim_attempt,
            now
        ],
    )?;
    let mut row = load_task_mutation_row(&tx, task_id, &fingerprint_hash)?
        .ok_or_else(|| anyhow!("task_mutation_insert_not_observable"))?;
    let acquired = if inserted == 1 {
        true
    } else if row.record.phase == TaskMutationPhase::IntentRecorded
        && (row.lease_owner != lease_owner || row.claim_attempt != claim_attempt)
    {
        let changed = tx.execute(
            "UPDATE task_mutation_ledger
             SET execution_token = ?3,
                 lease_owner = ?4,
                 claim_attempt = ?5,
                 updated_at = ?6
             WHERE task_id = ?1
               AND fingerprint_hash = ?2
               AND phase = 'intent_recorded'",
            params![
                task_id,
                fingerprint_hash,
                execution_token,
                lease_owner,
                claim_attempt,
                now
            ],
        )?;
        if changed == 1 {
            row.execution_token = execution_token.clone();
            row.lease_owner = lease_owner.to_string();
            row.claim_attempt = claim_attempt;
        }
        changed == 1
    } else {
        false
    };
    tx.commit()?;

    let record = row.record;
    if acquired {
        return Ok(BeginTaskMutationOutcome::Acquired(TaskMutationLease {
            record,
            execution_token: row.execution_token,
            lease_owner: row.lease_owner,
            claim_attempt: row.claim_attempt,
        }));
    }
    if record.phase.suppresses_replay() {
        Ok(BeginTaskMutationOutcome::ReplaySuppressed(record))
    } else {
        Ok(BeginTaskMutationOutcome::ReconciliationRequired(record))
    }
}

pub(crate) fn start_task_mutation_attempt(
    pool: &DbPool,
    lease: &mut TaskMutationLease,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET phase = 'attempt_started',
             attempt_no = attempt_no + 1,
             updated_at = ?4
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?5
           AND claim_attempt = ?6
           AND phase = 'intent_recorded'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("task_mutation_intent_not_startable"));
    }
    tx.commit()?;
    lease.record.phase = TaskMutationPhase::AttemptStarted;
    lease.record.attempt_no += 1;
    Ok(())
}

pub(crate) fn record_task_mutation_receipt(
    pool: &DbPool,
    lease: &TaskMutationLease,
    receipt_hash_source: &str,
    receipt_projection: Option<&Value>,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let receipt_hash = sha256_hex(receipt_hash_source.as_bytes());
    let receipt_json = serialize_optional_projection(receipt_projection, "mutation receipt")?;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET phase = 'receipt_recorded',
             receipt_hash = ?4,
             receipt_json = ?5,
             updated_at = ?6,
             receipt_at = ?6
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?7
           AND claim_attempt = ?8
           AND phase = 'attempt_started'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            receipt_hash,
            receipt_json,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("task_mutation_receipt_not_recordable"));
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn record_task_mutation_verification(
    pool: &DbPool,
    lease: &TaskMutationLease,
    verification_projection: &Value,
    verified: bool,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let verification_json = serde_json::to_string(verification_projection)
        .context("serialize mutation verification")?;
    let phase = if verified {
        TaskMutationPhase::Verified
    } else {
        TaskMutationPhase::VerificationPending
    };
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET phase = ?4,
             verification_json = ?5,
             updated_at = ?6,
             verified_at = CASE WHEN ?4 = 'verified' THEN ?6 ELSE NULL END
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?7
           AND claim_attempt = ?8
           AND phase = 'receipt_recorded'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            phase.as_token(),
            verification_json,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("task_mutation_verification_not_recordable"));
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn commit_task_mutation(pool: &DbPool, lease: &TaskMutationLease) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET phase = 'committed',
             updated_at = ?4,
             committed_at = ?4
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?5
           AND claim_attempt = ?6
           AND phase IN ('verification_pending', 'verified', 'reconciled')",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("task_mutation_not_committable"));
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn mark_task_mutation_uncertain(
    pool: &DbPool,
    lease: &TaskMutationLease,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET phase = 'reconciliation_required',
             updated_at = ?4
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?5
           AND claim_attempt = ?6
           AND phase = 'attempt_started'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("task_mutation_uncertainty_not_recordable"));
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn reconcile_task_mutation(
    pool: &DbPool,
    lease_owner: &str,
    claim_attempt: i64,
    task_id: &str,
    fingerprint_hash: &str,
    resolution: TaskMutationReconciliation,
    reconciliation_projection: &Value,
) -> anyhow::Result<ReconcileTaskMutationOutcome> {
    let task_id = required_value(task_id, "task_id")?;
    let lease_owner = required_value(lease_owner, "lease_owner")?;
    let fingerprint_hash = required_value(fingerprint_hash, "fingerprint_hash")?;
    let reconciliation_json = serde_json::to_string(reconciliation_projection)
        .context("serialize mutation reconciliation")?;
    let execution_token = uuid::Uuid::new_v4().to_string();
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task_mutation_ledger_db_pool")?;
    ensure_task_mutation_ledger_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(&tx, task_id, lease_owner, claim_attempt)?;
    let current = load_task_mutation_row(&tx, task_id, fingerprint_hash)?
        .ok_or_else(|| anyhow!("task_mutation_reconciliation_target_not_found"))?;
    if current.record.phase.suppresses_replay() {
        tx.commit()?;
        return Ok(ReconcileTaskMutationOutcome::ReplaySuppressed(
            current.record,
        ));
    }
    if current.record.phase != TaskMutationPhase::ReconciliationRequired
        && current.record.phase != TaskMutationPhase::AttemptStarted
    {
        return Err(anyhow!("task_mutation_not_reconcilable"));
    }

    match resolution {
        TaskMutationReconciliation::NotApplied => {
            tx.execute(
                "UPDATE task_mutation_ledger
                 SET phase = 'intent_recorded',
                     execution_token = ?3,
                     lease_owner = ?4,
                     claim_attempt = ?5,
                     reconciliation_json = ?6,
                     updated_at = ?7,
                     reconciled_at = ?7
                 WHERE task_id = ?1
                   AND fingerprint_hash = ?2
                   AND phase IN ('attempt_started', 'reconciliation_required')",
                params![
                    task_id,
                    fingerprint_hash,
                    execution_token,
                    lease_owner,
                    claim_attempt,
                    reconciliation_json,
                    now
                ],
            )?;
            let mut row = load_task_mutation_row(&tx, task_id, fingerprint_hash)?
                .ok_or_else(|| anyhow!("task_mutation_reconciled_not_observable"))?;
            row.record.reconciliation = Some(reconciliation_projection.clone());
            tx.commit()?;
            Ok(ReconcileTaskMutationOutcome::RetryReady(
                TaskMutationLease {
                    record: row.record,
                    execution_token: row.execution_token,
                    lease_owner: row.lease_owner,
                    claim_attempt: row.claim_attempt,
                },
            ))
        }
        TaskMutationReconciliation::Applied => {
            tx.execute(
                "UPDATE task_mutation_ledger
                 SET phase = 'reconciled',
                     execution_token = ?3,
                     lease_owner = ?4,
                     claim_attempt = ?5,
                     reconciliation_json = ?6,
                     updated_at = ?7,
                     reconciled_at = ?7
                 WHERE task_id = ?1
                   AND fingerprint_hash = ?2
                   AND phase IN ('attempt_started', 'reconciliation_required')",
                params![
                    task_id,
                    fingerprint_hash,
                    execution_token,
                    lease_owner,
                    claim_attempt,
                    reconciliation_json,
                    now
                ],
            )?;
            let row = load_task_mutation_row(&tx, task_id, fingerprint_hash)?
                .ok_or_else(|| anyhow!("task_mutation_reconciled_not_observable"))?;
            tx.commit()?;
            Ok(ReconcileTaskMutationOutcome::Reconciled(
                TaskMutationLease {
                    record: row.record,
                    execution_token: row.execution_token,
                    lease_owner: row.lease_owner,
                    claim_attempt: row.claim_attempt,
                },
            ))
        }
        TaskMutationReconciliation::StillUnknown => {
            tx.execute(
                "UPDATE task_mutation_ledger
                 SET phase = 'reconciliation_required',
                     lease_owner = ?3,
                     claim_attempt = ?4,
                     reconciliation_json = ?5,
                     updated_at = ?6
                 WHERE task_id = ?1
                   AND fingerprint_hash = ?2
                   AND phase IN ('attempt_started', 'reconciliation_required')",
                params![
                    task_id,
                    fingerprint_hash,
                    lease_owner,
                    claim_attempt,
                    reconciliation_json,
                    now
                ],
            )?;
            let row = load_task_mutation_row(&tx, task_id, fingerprint_hash)?
                .ok_or_else(|| anyhow!("task_mutation_reconciled_not_observable"))?;
            tx.commit()?;
            Ok(ReconcileTaskMutationOutcome::Waiting(row.record))
        }
    }
}

fn require_active_task_claim(
    db: &rusqlite::Connection,
    task_id: &str,
    expected_lease_owner: &str,
    expected_claim_attempt: i64,
) -> anyhow::Result<()> {
    let row = db
        .query_row(
            "SELECT status, lease_owner, COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let (task_status, active_lease_owner, active_claim_attempt) = match row {
        Some((status, owner, attempt)) => (Some(status), owner, Some(attempt)),
        None => (None, None, None),
    };
    if task_status.as_deref() == Some("running")
        && active_lease_owner.as_deref() == Some(expected_lease_owner)
        && active_claim_attempt == Some(expected_claim_attempt)
    {
        return Ok(());
    }
    Err(anyhow::Error::new(TaskMutationClaimRejected {
        status_code: crate::repo::WORKER_LEASE_LOST_STATUS_CODE,
        task_id: task_id.to_string(),
        expected_lease_owner: expected_lease_owner.to_string(),
        expected_claim_attempt,
        task_status,
        active_lease_owner,
        active_claim_attempt,
    }))
}

fn required_value<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("task_mutation_required_field_missing:{field}"));
    }
    Ok(value)
}

#[derive(Debug)]
struct TaskMutationRow {
    record: TaskMutationRecord,
    execution_token: String,
    lease_owner: String,
    claim_attempt: i64,
}

fn load_task_mutation_row(
    db: &rusqlite::Connection,
    task_id: &str,
    fingerprint_hash: &str,
) -> anyhow::Result<Option<TaskMutationRow>> {
    let row = db
        .query_row(
            "SELECT action_ref, phase, idempotency_key, attempt_no,
                    execution_token, lease_owner, claim_attempt,
                    receipt_json, verification_json, reconciliation_json
             FROM task_mutation_ledger
             WHERE task_id = ?1 AND fingerprint_hash = ?2",
            params![task_id, fingerprint_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        Ok(TaskMutationRow {
            record: TaskMutationRecord {
                task_id: task_id.to_string(),
                fingerprint_hash: fingerprint_hash.to_string(),
                action_ref: row.0,
                phase: TaskMutationPhase::parse(&row.1)?,
                idempotency_key: row.2,
                attempt_no: row.3,
                receipt: parse_projection_json(row.7.as_deref(), "mutation receipt")?,
                verification: parse_projection_json(row.8.as_deref(), "mutation verification")?,
                reconciliation: parse_projection_json(row.9.as_deref(), "mutation reconciliation")?,
            },
            execution_token: row.4,
            lease_owner: row.5,
            claim_attempt: row.6,
        })
    })
    .transpose()
}

fn ensure_task_mutation_ledger_schema(db: &mut rusqlite::Connection) -> anyhow::Result<()> {
    if !table_exists(db, "task_mutation_ledger")? {
        db.execute_batch(INIT_TASK_MUTATION_LEDGER_SQL)?;
        return Ok(());
    }
    if table_has_column(db, "task_mutation_ledger", "phase")? {
        db.execute_batch(INIT_TASK_MUTATION_LEDGER_SQL)?;
        return Ok(());
    }

    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "outcome_json",
        "ALTER TABLE task_mutation_ledger ADD COLUMN outcome_json TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "lease_owner",
        "ALTER TABLE task_mutation_ledger ADD COLUMN lease_owner TEXT NOT NULL DEFAULT ''",
    )?;
    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "claim_attempt",
        "ALTER TABLE task_mutation_ledger ADD COLUMN claim_attempt INTEGER NOT NULL DEFAULT 0",
    )?;

    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("ALTER TABLE task_mutation_ledger RENAME TO task_mutation_ledger_legacy;")?;
    tx.execute_batch(INIT_TASK_MUTATION_LEDGER_SQL)?;
    let legacy_rows = {
        let mut statement = tx.prepare(
            "SELECT task_id, fingerprint_hash, action_ref, status,
                    execution_token, lease_owner, claim_attempt,
                    outcome_hash, outcome_json, started_at, updated_at, completed_at
             FROM task_mutation_ledger_legacy",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for row in legacy_rows {
        let committed = row.3 == "completed";
        let phase = if committed {
            TaskMutationPhase::Committed
        } else {
            TaskMutationPhase::ReconciliationRequired
        };
        let idempotency_key = mutation_idempotency_key(&row.0, &row.1);
        tx.execute(
            "INSERT INTO task_mutation_ledger (
                 task_id, fingerprint_hash, action_ref, phase, idempotency_key,
                 attempt_no, execution_token, lease_owner, claim_attempt,
                 receipt_hash, receipt_json, verification_json,
                 reconciliation_json, started_at, updated_at, receipt_at,
                 verified_at, reconciled_at, committed_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, NULL, NULL,
                 ?11, ?12, ?13, ?13, NULL, ?13
             )",
            params![
                row.0,
                row.1,
                row.2,
                phase.as_token(),
                idempotency_key,
                row.4,
                row.5,
                row.6,
                row.7,
                row.8,
                row.9,
                row.10,
                row.11,
            ],
        )?;
    }
    tx.execute_batch("DROP TABLE task_mutation_ledger_legacy;")?;
    tx.commit()?;
    Ok(())
}

fn table_exists(db: &rusqlite::Connection, table: &str) -> anyhow::Result<bool> {
    db.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
        params![table],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn table_has_column(db: &rusqlite::Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns.iter().any(|candidate| candidate == column))
}

fn serialize_optional_projection(
    projection: Option<&Value>,
    label: &str,
) -> anyhow::Result<Option<String>> {
    projection
        .map(serde_json::to_string)
        .transpose()
        .with_context(|| format!("serialize {label} projection"))
}

fn parse_projection_json(raw: Option<&str>, label: &str) -> anyhow::Result<Option<Value>> {
    raw.map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("parse {label} projection"))
}

fn mutation_idempotency_key(task_id: &str, fingerprint_hash: &str) -> String {
    sha256_hex(format!("rustclaw:task-mutation:v2\0{task_id}\0{fingerprint_hash}").as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "task_mutation_ledger_tests.rs"]
mod tests;
