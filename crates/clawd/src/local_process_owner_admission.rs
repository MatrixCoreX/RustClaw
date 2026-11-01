use rusqlite::{Connection, ErrorCode};
use serde::{Deserialize, Serialize};

use super::owner_authority::{AuthorityFailure, OwnerAuthorityBroker};
use super::owner_reference::OwnerReferenceFailure;
use super::owner_store::OwnerReferenceStore;

#[path = "local_process_owner_admission_journal.rs"]
mod journal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionStage {
    Configure,
    Reserve,
    CommitReady,
    AdmitExisting,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionFailure {
    InvalidJournal,
    UnsupportedSchema,
    MissingJournal,
    IncompleteInitialization,
    AlreadyInitialized,
    ClaimLost,
    DurabilityUnavailable,
    SequenceExhausted,
    AuthorityMismatch,
    EntropyUnavailable,
    Sqlite {
        stage: AdmissionStage,
        sqlite_code: Option<ErrorCode>,
        sqlite_extended_code: Option<i32>,
    },
    Authority(AuthorityFailure),
    Reference(OwnerReferenceFailure),
}

// Dedicated connection, not a borrowed ordinary pool connection. Changing this
// connection's sync policy must never change the existing runtime pool policy.
// Journal/key coherence is not a policy grant, job liveness or terminal proof.
pub(crate) struct OwnerAuthorityJournal {
    db: Connection,
}

impl OwnerAuthorityJournal {
    pub(crate) fn from_connection(db: Connection) -> Result<Self, AdmissionFailure> {
        if !db.is_autocommit() {
            return Err(AdmissionFailure::InvalidJournal);
        }
        let mode: String = db
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|error| sqlite_failure(AdmissionStage::Configure, error))?;
        if mode != "wal" || db.path().is_none_or(str::is_empty) {
            return Err(AdmissionFailure::DurabilityUnavailable);
        }
        db.pragma_update(None, "synchronous", "FULL")
            .map_err(|error| sqlite_failure(AdmissionStage::Configure, error))?;
        #[cfg(target_os = "macos")]
        db.pragma_update(None, "fullfsync", true)
            .map_err(|error| sqlite_failure(AdmissionStage::Configure, error))?;
        journal::validate_durability(&db, AdmissionStage::Configure)?;
        Ok(Self { db })
    }

    // Explicit host first-install admission. Never fall back here from resume.
    // No DB write lock is held while the broker does filesystem work.
    pub(crate) fn initialize(
        &mut self,
        broker: &OwnerAuthorityBroker,
    ) -> Result<OwnerReferenceStore, AdmissionFailure> {
        let reservation = journal::reserve(&mut self.db)?;
        let store = broker
            .initialize_new()
            .map_err(AdmissionFailure::Authority)?;
        let fingerprint = store
            .authority_fingerprint()
            .map_err(AdmissionFailure::Reference)?;
        journal::commit_ready(&mut self.db, reservation, fingerprint)?;
        Ok(store)
    }

    pub(crate) fn resume(
        &mut self,
        broker: &OwnerAuthorityBroker,
        expected: &super::owner_reference::AuthorityFingerprint,
    ) -> Result<OwnerReferenceStore, AdmissionFailure> {
        let fingerprint = journal::admit_existing(&mut self.db, expected)?;
        broker
            .load_expected(&fingerprint)
            .map_err(AdmissionFailure::Authority)
    }

    pub(super) fn close(self) -> Result<(), AdmissionFailure> {
        self.db
            .close()
            .map_err(|(_, error)| sqlite_failure(AdmissionStage::Close, error))
    }
}

fn sqlite_failure(stage: AdmissionStage, error: rusqlite::Error) -> AdmissionFailure {
    AdmissionFailure::Sqlite {
        stage,
        sqlite_code: error.sqlite_error_code(),
        sqlite_extended_code: error.sqlite_error().map(|error| error.extended_code),
    }
}

#[cfg(test)]
#[path = "local_process_owner_admission_tests.rs"]
pub(super) mod tests;
