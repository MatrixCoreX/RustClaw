use argon2::password_hash::rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use super::super::owner_reference::AuthorityFingerprint;
use super::{sqlite_failure, AdmissionFailure, AdmissionStage};

pub(super) const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS local_process_authority_bootstrap (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    reservation_id BLOB NOT NULL CHECK (typeof(reservation_id) = 'blob' AND length(reservation_id) = 16),
    phase TEXT NOT NULL CHECK (phase IN ('initializing', 'ready')),
    fingerprint BLOB,
    admission_sequence INTEGER NOT NULL CHECK (typeof(admission_sequence) = 'integer' AND admission_sequence >= 0),
    CHECK (
        (phase = 'initializing' AND fingerprint IS NULL AND admission_sequence = 0)
        OR (phase = 'ready' AND typeof(fingerprint) = 'blob' AND length(fingerprint) = 32
            AND fingerprint != zeroblob(32) AND admission_sequence > 0)
    )
) WITHOUT ROWID;
"#;

pub(super) struct Reservation {
    id: Uuid,
}

struct Record {
    schema_version: i64,
    reservation_id: Option<Vec<u8>>,
    phase: i64,
    fingerprint: Option<Vec<u8>>,
    fingerprint_absent: bool,
    sequence: i64,
}

pub(super) fn validate_durability(
    db: &Connection,
    stage: AdmissionStage,
) -> Result<(), AdmissionFailure> {
    if !db.is_autocommit() {
        return Err(AdmissionFailure::InvalidJournal);
    }
    let mode: String = db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| sqlite_failure(stage, error))?;
    let synchronous: i64 = db
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .map_err(|error| sqlite_failure(stage, error))?;
    if mode != "wal" || !matches!(synchronous, 2 | 3) || db.path().is_none_or(str::is_empty) {
        return Err(AdmissionFailure::DurabilityUnavailable);
    }
    #[cfg(target_os = "macos")]
    {
        let fullfsync: bool = db
            .pragma_query_value(None, "fullfsync", |row| row.get(0))
            .map_err(|error| sqlite_failure(stage, error))?;
        if !fullfsync {
            return Err(AdmissionFailure::DurabilityUnavailable);
        }
    }
    Ok(())
}

pub(super) fn reserve(db: &mut Connection) -> Result<Reservation, AdmissionFailure> {
    let stage = AdmissionStage::Reserve;
    validate_durability(db, stage)?;
    let mut bytes = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| AdmissionFailure::EntropyUnavailable)?;
    let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_failure(stage, error))?;
    tx.execute_batch(INIT_SQL)
        .map_err(|error| sqlite_failure(stage, error))?;
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM local_process_authority_bootstrap",
            [],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_failure(stage, error))?;
    if count != 0 {
        return Err(AdmissionFailure::AlreadyInitialized);
    }
    tx.execute(
        "INSERT INTO local_process_authority_bootstrap(
        singleton, schema_version, reservation_id, phase, fingerprint, admission_sequence
        ) VALUES (1, 1, ?1, 'initializing', NULL, 0)",
        [id.as_bytes().as_slice()],
    )
    .map_err(|error| sqlite_failure(stage, error))?;
    tx.commit().map_err(|error| sqlite_failure(stage, error))?;
    Ok(Reservation { id })
}

pub(super) fn commit_ready(
    db: &mut Connection,
    reservation: Reservation,
    fingerprint: AuthorityFingerprint,
) -> Result<(), AdmissionFailure> {
    let stage = AdmissionStage::CommitReady;
    validate_durability(db, stage)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_failure(stage, error))?;
    let changed = tx
        .execute(
            "UPDATE local_process_authority_bootstrap
        SET phase = 'ready', fingerprint = ?2, admission_sequence = 1
        WHERE singleton = 1 AND schema_version = 1 AND phase = 'initializing'
          AND reservation_id = ?1 AND fingerprint IS NULL AND admission_sequence = 0",
            params![
                reservation.id.as_bytes().as_slice(),
                fingerprint.as_bytes().as_slice()
            ],
        )
        .map_err(|error| sqlite_failure(stage, error))?;
    if changed != 1 {
        return Err(AdmissionFailure::ClaimLost);
    }
    tx.commit().map_err(|error| sqlite_failure(stage, error))?;
    Ok(())
}

pub(super) fn admit_existing(
    db: &mut Connection,
    expected: &AuthorityFingerprint,
) -> Result<AuthorityFingerprint, AdmissionFailure> {
    let stage = AdmissionStage::AdmitExisting;
    validate_durability(db, stage)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_failure(stage, error))?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
        WHERE type = 'table' AND name = 'local_process_authority_bootstrap')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_failure(stage, error))?;
    if !exists {
        return Err(AdmissionFailure::MissingJournal);
    }
    let row = tx.query_row("SELECT schema_version,
        CASE WHEN typeof(reservation_id) = 'blob' AND length(reservation_id) = 16 THEN reservation_id END,
        CASE phase WHEN 'initializing' THEN 0 WHEN 'ready' THEN 1 ELSE -1 END,
        CASE WHEN typeof(fingerprint) = 'blob' AND length(fingerprint) = 32 THEN fingerprint END,
        admission_sequence, fingerprint IS NULL
        FROM local_process_authority_bootstrap WHERE singleton = 1", [], |row| Ok(Record {
            schema_version: row.get(0)?, reservation_id: row.get(1)?, phase: row.get(2)?,
            fingerprint: row.get(3)?, sequence: row.get(4)?, fingerprint_absent: row.get(5)?,
        })).optional().map_err(|error| sqlite_failure(stage, error))?
        .ok_or(AdmissionFailure::MissingJournal)?;
    if row.schema_version != 1 {
        return Err(AdmissionFailure::UnsupportedSchema);
    }
    let id = Uuid::from_slice(
        row.reservation_id
            .as_deref()
            .ok_or(AdmissionFailure::InvalidJournal)?,
    )
    .map_err(|_| AdmissionFailure::InvalidJournal)?;
    if id.get_version_num() != 4 || id.get_variant() != uuid::Variant::RFC4122 {
        return Err(AdmissionFailure::InvalidJournal);
    }
    if row.phase == 0 && row.fingerprint_absent && row.sequence == 0 {
        return Err(AdmissionFailure::IncompleteInitialization);
    }
    if row.phase != 1 || row.sequence <= 0 {
        return Err(AdmissionFailure::InvalidJournal);
    }
    let fingerprint = row
        .fingerprint
        .as_deref()
        .and_then(AuthorityFingerprint::from_slice)
        .ok_or(AdmissionFailure::InvalidJournal)?;
    if &fingerprint != expected {
        return Err(AdmissionFailure::AuthorityMismatch);
    }
    if row.sequence == i64::MAX {
        return Err(AdmissionFailure::SequenceExhausted);
    }
    // A real write + FULL commit is a durable admission barrier, including
    // recovery after an earlier commit whose reply/outcome was uncertain.
    let changed = tx
        .execute(
            "UPDATE local_process_authority_bootstrap
        SET admission_sequence = admission_sequence + 1
        WHERE singleton = 1 AND schema_version = 1 AND phase = 'ready'
          AND fingerprint = ?1 AND admission_sequence = ?2",
            params![fingerprint.as_bytes().as_slice(), row.sequence],
        )
        .map_err(|error| sqlite_failure(stage, error))?;
    if changed != 1 {
        return Err(AdmissionFailure::ClaimLost);
    }
    tx.commit().map_err(|error| sqlite_failure(stage, error))?;
    Ok(fingerprint)
}
