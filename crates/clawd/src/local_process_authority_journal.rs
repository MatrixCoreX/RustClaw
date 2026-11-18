use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::time::Duration;

use super::super::owner_admission::OwnerAuthorityJournal;
use super::super::owner_authority::{validate_directory, AuthorityOperation, OwnerAuthorityBroker};
use super::super::owner_outcome::OwnerObservationRecord;
use super::super::owner_reference::{AuthorityFingerprint, OwnerReferenceFailure};
use super::protocol::{
    AuthorityAction, AuthorityAdmissionCode as Code, AuthorityAdmissionError, AuthorityResponse,
};

pub(super) const JOURNAL_NAME: &str = "authority.sqlite";

fn invalid_journal() -> AuthorityAdmissionError {
    AuthorityAdmissionError::code(Code::InvalidJournal)
}

fn validate_record(metadata: &fs::Metadata) -> Result<(), AuthorityAdmissionError> {
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(invalid_journal());
    }
    Ok(())
}

fn validate_records() -> Result<(), AuthorityAdmissionError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        match fs::symlink_metadata(format!("{JOURNAL_NAME}{suffix}")) {
            Ok(metadata) => validate_record(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AuthorityAdmissionError::io(Code::StorageUnavailable, error)),
        }
    }
    Ok(())
}

fn same_file(file: &File) -> Result<(), AuthorityAdmissionError> {
    let admitted = file
        .metadata()
        .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
    let current = fs::symlink_metadata(JOURNAL_NAME)
        .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
    validate_record(&current)?;
    if (admitted.dev(), admitted.ino()) != (current.dev(), current.ino()) {
        return Err(invalid_journal());
    }
    Ok(())
}

fn open_journal(create: bool) -> Result<(OwnerAuthorityJournal, File), AuthorityAdmissionError> {
    validate_records()?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(JOURNAL_NAME)
        .map_err(|error| {
            let code = match error.kind() {
                std::io::ErrorKind::AlreadyExists if create => Code::AlreadyInitialized,
                std::io::ErrorKind::NotFound if !create => Code::MissingJournal,
                _ => Code::StorageUnavailable,
            };
            AuthorityAdmissionError::io(code, error)
        })?;
    validate_record(
        &file
            .metadata()
            .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?,
    )?;
    if create {
        file.sync_all()
            .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
        File::open(".")
            .and_then(|directory| directory.sync_all())
            .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
    } else {
        // A partial first-create claim is retained, never promoted into a new
        // database just because SQLite accepts an existing zero-length file.
        let mut header = [0; 16];
        if file
            .metadata()
            .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?
            .len()
            < 100
            || file.read_exact(&mut header).is_err()
            || &header != b"SQLite format 3\0"
        {
            return Err(invalid_journal());
        }
    }
    let db = rusqlite::Connection::open_with_flags(
        JOURNAL_NAME,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(AuthorityAdmissionError::sqlite)?;
    same_file(&file)?;
    db.busy_timeout(Duration::from_secs(8))
        .map_err(AuthorityAdmissionError::sqlite)?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(AuthorityAdmissionError::sqlite)?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(AuthorityAdmissionError::sqlite)?;
    validate_records()?;
    let journal = OwnerAuthorityJournal::from_connection(db)?;
    Ok((journal, file))
}

/// # Safety
/// Only the independently execed, protected, one-operation authority helper may
/// call this. Cwd/umask are process-wide and never restored. The host must have
/// separately fenced both admitted directories from all managed payloads.
unsafe fn with_store<T>(
    key_parent: File,
    journal_parent: File,
    action: &AuthorityAction,
    apply: impl FnOnce(
        &super::super::owner_store::OwnerReferenceStore,
    ) -> Result<T, AuthorityAdmissionError>,
) -> Result<T, AuthorityAdmissionError> {
    validate_directory(&key_parent, AuthorityOperation::AdmitParent)?;
    validate_directory(&journal_parent, AuthorityOperation::AdmitParent)?;
    for parent in [&key_parent, &journal_parent] {
        parent
            .sync_all()
            .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
    }
    unsafe {
        libc::umask(0o077);
    }
    if unsafe { libc::fchdir(journal_parent.as_raw_fd()) } != 0 {
        return Err(AuthorityAdmissionError::io(
            Code::InvalidDirectory,
            std::io::Error::last_os_error(),
        ));
    }
    let actual = fs::metadata(".")
        .map_err(|error| AuthorityAdmissionError::io(Code::InvalidDirectory, error))?;
    let expected = journal_parent
        .metadata()
        .map_err(|error| AuthorityAdmissionError::io(Code::InvalidDirectory, error))?;
    if (actual.dev(), actual.ino()) != (expected.dev(), expected.ino()) {
        return Err(AuthorityAdmissionError::code(Code::InvalidDirectory));
    }
    let broker = OwnerAuthorityBroker::from_admitted_directory(key_parent)?;
    let (mut journal, file) = open_journal(matches!(action, AuthorityAction::Initialize {}))?;
    let store = match action {
        AuthorityAction::Initialize {} => journal.initialize(&broker)?,
        AuthorityAction::Resume {
            authority_fingerprint,
        }
        | AuthorityAction::InventoryResources {
            authority_fingerprint,
            ..
        }
        | AuthorityAction::PublishReference {
            authority_fingerprint,
            ..
        }
        | AuthorityAction::ReadOutcome {
            authority_fingerprint,
            ..
        }
        | AuthorityAction::LoadReference {
            authority_fingerprint,
            ..
        }
        | AuthorityAction::HoldResources {
            authority_fingerprint,
            ..
        }
        | AuthorityAction::ReadResources {
            authority_fingerprint,
            ..
        } => {
            let expected = AuthorityFingerprint::from_slice(authority_fingerprint)
                .ok_or_else(|| AuthorityAdmissionError::code(Code::InvalidRequest))?;
            journal.resume(&broker, &expected)?
        }
    };
    let response = apply(&store)?;
    drop(store);
    journal.close()?;
    same_file(&file)?;
    validate_records()?;
    journal_parent
        .sync_all()
        .map_err(|error| AuthorityAdmissionError::io(Code::StorageUnavailable, error))?;
    Ok(response)
}

pub(super) unsafe fn admit(
    key_parent: File,
    journal_parent: File,
    action: &AuthorityAction,
) -> Result<AuthorityResponse, AuthorityAdmissionError> {
    unsafe {
        with_store(key_parent, journal_parent, action, |store| {
            let fingerprint = store.authority_fingerprint()?;
            let response = match action {
                AuthorityAction::PublishReference {
                    job_id,
                    owner,
                    binding_digest,
                    ..
                } => {
                    let credential =
                        store.credential_for(*job_id, owner.clone(), *binding_digest)?;
                    let reference = match store.load(*job_id)? {
                        Some(reference) => {
                            if !reference.authorizes(&credential, binding_digest) {
                                return Err(
                                    super::super::owner_reference::OwnerReferenceFailure::Conflict
                                        .into(),
                                );
                            }
                            reference
                        }
                        None => {
                            let observed = super::super::instance::observe_process(owner.pid)
                                .map_err(|_| {
                                    AuthorityAdmissionError::code(Code::OwnerUnavailable)
                                })?;
                            if observed.birth != *owner {
                                return Err(AuthorityAdmissionError::code(
                                    Code::OwnerIdentityMismatch,
                                ));
                            }
                            if observed.state != super::super::instance::ProcessRunState::Active {
                                return Err(AuthorityAdmissionError::code(Code::OwnerUnavailable));
                            }
                            store.publish(&credential, *binding_digest)?
                        }
                    };
                    AuthorityResponse::Reference {
                        schema_version: 1,
                        authority_fingerprint: *fingerprint.as_bytes(),
                        credential: reference.credential(),
                        binding_digest: *binding_digest,
                    }
                }
                AuthorityAction::LoadReference {
                    job_id,
                    owner,
                    binding_digest,
                    ..
                } => {
                    let reference = store
                        .load(*job_id)?
                        .ok_or(OwnerReferenceFailure::InvalidReference)?;
                    let expected = store.credential_for(*job_id, owner.clone(), *binding_digest)?;
                    if !reference.authorizes(&expected, binding_digest) {
                        return Err(OwnerReferenceFailure::Conflict.into());
                    }
                    AuthorityResponse::Reference {
                        schema_version: 1,
                        authority_fingerprint: *fingerprint.as_bytes(),
                        credential: reference.credential(),
                        binding_digest: *binding_digest,
                    }
                }
                AuthorityAction::Initialize {} | AuthorityAction::Resume { .. } => {
                    AuthorityResponse::from_result(Ok(fingerprint))
                }
                AuthorityAction::InventoryResources { .. } => {
                    return Err(AuthorityAdmissionError::code(Code::InvalidRequest))
                }
                AuthorityAction::HoldResources {
                    credential,
                    binding_digest,
                    ..
                }
                | AuthorityAction::ReadResources {
                    credential,
                    binding_digest,
                    ..
                }
                | AuthorityAction::ReadOutcome {
                    credential,
                    binding_digest,
                    ..
                } => {
                    let reference = store
                        .load(credential.job_id)?
                        .ok_or(OwnerReferenceFailure::InvalidReference)?;
                    if !reference.authorizes(credential, binding_digest) {
                        return Err(OwnerReferenceFailure::Conflict.into());
                    }
                    if let AuthorityAction::HoldResources { budget, .. } = action {
                        store.retain_resources(&reference, budget.clone())?;
                    }
                    let outcome = store.load_outcome(&reference)?;
                    if matches!(action, AuthorityAction::ReadOutcome { .. }) {
                        AuthorityResponse::Outcome {
                            schema_version: 1,
                            authority_fingerprint: *fingerprint.as_bytes(),
                            reference: reference.record().clone(),
                            observation: OwnerObservationRecord::from_verified_outcome(
                                outcome.as_ref(),
                            ),
                        }
                    } else {
                        AuthorityResponse::Resources {
                            schema_version: 1,
                            authority_fingerprint: *fingerprint.as_bytes(),
                            reference: reference.record().clone(),
                            budget: store.load_resources(&reference)?,
                            observation: OwnerObservationRecord::from_verified_outcome(
                                outcome.as_ref(),
                            ),
                        }
                    }
                }
            };
            Ok(response)
        })
    }
}

pub(super) unsafe fn inventory(
    key_parent: File,
    journal_parent: File,
    action: &AuthorityAction,
    emit: impl FnMut(
        &super::super::owner_store::DurableOwnerReference,
        &super::super::owner_resources::OwnerResourceBudget,
    ) -> Result<(), OwnerReferenceFailure>,
) -> Result<super::super::owner_store::inventory::OwnerInventoryCounts, AuthorityAdmissionError> {
    unsafe {
        with_store(key_parent, journal_parent, action, |store| {
            store.visit_pending_resources(emit).map_err(Into::into)
        })
    }
}
