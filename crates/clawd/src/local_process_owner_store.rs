use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use uuid::Uuid;

use super::owner_handshake::NoCommitEvidence;
use super::owner_no_commit::{DurableOwnerNoCommit, OwnerNoCommitRecord};
use super::owner_outcome::{
    DurableOwnerOutcome, DurableOwnerStartupFailure, OwnerOutcomeRecord, OwnerStartupFailureRecord,
};
use super::owner_protocol::OwnerCredential;
use super::owner_reference::{
    AuthorityFingerprint, OwnerAuthorityKey, OwnerReference, OwnerReferenceFailure,
    MAX_REFERENCE_BYTES,
};
use super::owner_runtime::TerminalEvidence;
use super::owner_start::StartupFailureEvidence;
use super::owner_terminal::{DurableOwnerTerminal, OwnerTerminalRecord};

#[path = "local_process_owner_resource_store.rs"]
mod resources;

#[path = "local_process_owner_inventory_store.rs"]
pub(super) mod inventory;

// Host-owned authority, not a task's writable output directory. Keep the
// admitted directory pinned; path replacement cannot redirect publication.
pub(crate) struct OwnerReferenceStore {
    directory: OwnedFd,
    key: OwnerAuthorityKey,
}

// Only strict publication/revalidation can construct this token. It contains
// a private control credential but never serializes/debugs it or writes it out.
pub(crate) struct DurableOwnerReference {
    reference: OwnerReference,
    credential: OwnerCredential,
}

impl DurableOwnerReference {
    pub(super) fn record(&self) -> &OwnerReference {
        &self.reference
    }

    /// # Safety
    /// Only the pinned protected authority helper's validated private response,
    /// exact request binding and confirmed successful exit establish this proof.
    /// Arbitrary decoded JSON or a public acknowledgment never does.
    pub(super) unsafe fn from_protected_reference(
        credential: OwnerCredential,
        binding_digest: [u8; 32],
    ) -> Result<Self, OwnerReferenceFailure> {
        credential
            .validate()
            .map_err(|_| OwnerReferenceFailure::InvalidReference)?;
        let reference =
            OwnerReference::new(credential.job_id, credential.owner.clone(), binding_digest)?;
        Ok(Self {
            reference,
            credential,
        })
    }

    pub(crate) fn credential(&self) -> OwnerCredential {
        self.credential.clone()
    }

    pub(crate) fn authorizes(
        &self,
        credential: &OwnerCredential,
        binding_digest: &[u8; 32],
    ) -> bool {
        &self.credential == credential && &self.reference.binding_digest == binding_digest
    }
}

impl OwnerReferenceStore {
    pub(crate) fn authority_fingerprint(
        &self,
    ) -> Result<AuthorityFingerprint, OwnerReferenceFailure> {
        self.key.fingerprint()
    }

    pub(crate) fn credential_for(
        &self,
        job_id: Uuid,
        owner: super::instance::ProcessBirthIdentity,
        binding_digest: [u8; 32],
    ) -> Result<OwnerCredential, OwnerReferenceFailure> {
        self.key.credential_for(job_id, owner, binding_digest)
    }

    pub(crate) fn from_admitted_directory(
        directory: File,
        key: OwnerAuthorityKey,
    ) -> Result<Self, OwnerReferenceFailure> {
        let store = Self {
            directory: directory.into(),
            key,
        };
        store.validate_directory()?;
        Ok(store)
    }

    pub(crate) fn publish(
        &self,
        credential: &OwnerCredential,
        binding_digest: [u8; 32],
    ) -> Result<DurableOwnerReference, OwnerReferenceFailure> {
        let reference =
            OwnerReference::new(credential.job_id, credential.owner.clone(), binding_digest)?;
        if self.key.credential(&reference)? != *credential {
            return Err(OwnerReferenceFailure::AuthenticationFailed);
        }
        self.validate_directory()?;
        let bytes = self.key.sign(reference.clone())?;
        let name = reference_name(credential.job_id)?;
        self.publish_record(&name, &bytes)?;
        let durable = self
            .load(credential.job_id)?
            .ok_or(OwnerReferenceFailure::StorageUnavailable)?;
        if durable.reference != reference {
            return Err(OwnerReferenceFailure::Conflict);
        }
        Ok(durable)
    }

    pub(crate) fn publish_terminal(
        &self,
        reference: &DurableOwnerReference,
        evidence: &TerminalEvidence,
    ) -> Result<DurableOwnerTerminal, OwnerReferenceFailure> {
        if !reference.authorizes(evidence.observed().0, &reference.reference.binding_digest) {
            return Err(OwnerReferenceFailure::AuthenticationFailed);
        }
        self.require_reference(reference)?;
        let record = OwnerTerminalRecord::from_evidence(reference.reference.clone(), evidence)?;
        let bytes = self.key.sign_terminal(record.clone())?;
        self.publish_record(&outcome_name(reference.credential.job_id)?, &bytes)?;
        match self.load_outcome(reference)? {
            Some(DurableOwnerOutcome::Terminal(durable))
                if durable.snapshot() == record.snapshot() =>
            {
                Ok(durable)
            }
            Some(_) => Err(OwnerReferenceFailure::Conflict),
            None => Err(OwnerReferenceFailure::StorageUnavailable),
        }
    }

    pub(crate) fn publish_startup_failure(
        &self,
        reference: &DurableOwnerReference,
        evidence: &StartupFailureEvidence,
    ) -> Result<DurableOwnerStartupFailure, OwnerReferenceFailure> {
        let (credential, binding, _) = evidence.observed();
        if !reference.authorizes(credential, binding) {
            return Err(OwnerReferenceFailure::AuthenticationFailed);
        }
        self.require_reference(reference)?;
        let record =
            OwnerStartupFailureRecord::from_evidence(reference.reference.clone(), evidence)?;
        let bytes = self.key.sign_startup_failure(record.clone())?;
        self.publish_record(&outcome_name(credential.job_id)?, &bytes)?;
        match self.load_outcome(reference)? {
            Some(DurableOwnerOutcome::StartupFailed(durable))
                if durable.error_code() == record.error_code() =>
            {
                Ok(durable)
            }
            Some(_) => Err(OwnerReferenceFailure::Conflict),
            None => Err(OwnerReferenceFailure::StorageUnavailable),
        }
    }

    #[cfg(test)]
    pub(crate) fn load_terminal(
        &self,
        reference: &DurableOwnerReference,
    ) -> Result<Option<DurableOwnerTerminal>, OwnerReferenceFailure> {
        match self.load_outcome(reference)? {
            Some(DurableOwnerOutcome::Terminal(terminal)) => Ok(Some(terminal)),
            Some(DurableOwnerOutcome::StartupFailed(_) | DurableOwnerOutcome::NotCommitted(_)) => {
                Err(OwnerReferenceFailure::Conflict)
            }
            None => Ok(None),
        }
    }

    pub(crate) fn publish_no_commit(
        &self,
        reference: &DurableOwnerReference,
        evidence: &NoCommitEvidence,
    ) -> Result<DurableOwnerNoCommit, OwnerReferenceFailure> {
        let (credential, binding) = evidence.observed();
        if !reference.authorizes(credential, binding) {
            return Err(OwnerReferenceFailure::AuthenticationFailed);
        }
        self.require_reference(reference)?;
        let record = OwnerNoCommitRecord::from_evidence(reference.reference.clone(), evidence)?;
        let bytes = self.key.sign_no_commit(record)?;
        self.publish_record(&outcome_name(credential.job_id)?, &bytes)?;
        match self.load_outcome(reference)? {
            Some(DurableOwnerOutcome::NotCommitted(durable)) => Ok(durable),
            Some(_) => Err(OwnerReferenceFailure::Conflict),
            None => Err(OwnerReferenceFailure::StorageUnavailable),
        }
    }

    fn require_reference(
        &self,
        expected: &DurableOwnerReference,
    ) -> Result<(), OwnerReferenceFailure> {
        let actual = self
            .load(expected.credential.job_id)?
            .ok_or(OwnerReferenceFailure::InvalidReference)?;
        if actual.reference != expected.reference || actual.credential != expected.credential {
            return Err(OwnerReferenceFailure::Conflict);
        }
        Ok(())
    }

    pub(crate) fn load_outcome(
        &self,
        reference: &DurableOwnerReference,
    ) -> Result<Option<DurableOwnerOutcome>, OwnerReferenceFailure> {
        self.require_reference(reference)?;
        let Some(bytes) = self.read_record(&outcome_name(reference.credential.job_id)?)? else {
            return Ok(None);
        };
        let record = self.key.verify_outcome(&bytes)?;
        if record.reference() != &reference.reference {
            return Err(OwnerReferenceFailure::Conflict);
        }
        Ok(Some(match record {
            OwnerOutcomeRecord::NotCommitted(record) => DurableOwnerOutcome::NotCommitted(
                DurableOwnerNoCommit::verified(record, reference.credential()),
            ),
            OwnerOutcomeRecord::Terminal(record) => DurableOwnerOutcome::Terminal(
                DurableOwnerTerminal::verified(record, reference.credential()),
            ),
            OwnerOutcomeRecord::StartupFailed(record) => DurableOwnerOutcome::StartupFailed(
                DurableOwnerStartupFailure::verified(record, reference.credential()),
            ),
        }))
    }

    fn publish_record(&self, name: &CString, bytes: &[u8]) -> Result<(), OwnerReferenceFailure> {
        self.validate_directory()?;
        let _record_set = self.record_set_lock(false)?;
        let mut temporary = TemporaryReference::create(&self.directory)?;
        validate_file(&temporary.file)?;
        temporary.file.write_all(bytes).map_err(storage)?;
        temporary.file.sync_all().map_err(storage)?;
        match crate::secure_workspace_fs::rename_without_replacement(
            &self.directory,
            &temporary.name,
            name,
        ) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(storage(error)),
        }
        Ok(())
    }

    pub(crate) fn load(
        &self,
        job_id: Uuid,
    ) -> Result<Option<DurableOwnerReference>, OwnerReferenceFailure> {
        let name = reference_name(job_id)?;
        let Some(bytes) = self.read_record(&name)? else {
            return Ok(None);
        };
        let reference = self.key.verify(&bytes)?;
        if reference.job_id != job_id {
            return Err(OwnerReferenceFailure::Conflict);
        }
        let credential = self.key.credential(&reference)?;
        Ok(Some(DurableOwnerReference {
            reference,
            credential,
        }))
    }

    fn read_record(&self, name: &CStr) -> Result<Option<Vec<u8>>, OwnerReferenceFailure> {
        self.validate_directory()?;
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(storage(error))
            };
        }
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        validate_file(&file)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_REFERENCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        // File and directory synchronization are strict here. Unlike ordinary
        // workspace writes, EINVAL/ENOTSUP never produce a start permission.
        file.sync_all().map_err(storage)?;
        if unsafe { libc::fsync(self.directory.as_raw_fd()) } != 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        Ok(Some(bytes))
    }

    fn validate_directory(&self) -> Result<(), OwnerReferenceFailure> {
        let descriptor = unsafe { libc::fcntl(self.directory.as_raw_fd(), libc::F_GETFD) };
        let file = File::from(self.directory.try_clone().map_err(storage)?);
        let metadata = file.metadata().map_err(storage)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7777 != 0o700
            || descriptor < 0
            || descriptor & libc::FD_CLOEXEC == 0
        {
            return Err(OwnerReferenceFailure::InvalidDirectory);
        }
        Ok(())
    }
}

struct TemporaryReference<'a> {
    directory: &'a OwnedFd,
    name: CString,
    file: File,
}
impl<'a> TemporaryReference<'a> {
    fn create(directory: &'a OwnedFd) -> Result<Self, OwnerReferenceFailure> {
        let name = CString::new(format!(".owner-ref-{}.tmp", Uuid::new_v4().simple())).unwrap();
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if descriptor < 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        Ok(Self {
            directory,
            name,
            file: unsafe { File::from_raw_fd(descriptor) },
        })
    }
}
impl Drop for TemporaryReference<'_> {
    fn drop(&mut self) {
        // Only the exclusively created temporary entry, never the job claim.
        unsafe {
            libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}

fn validate_file(file: &File) -> Result<(), OwnerReferenceFailure> {
    let metadata = file.metadata().map_err(storage)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(OwnerReferenceFailure::InvalidReference);
    }
    Ok(())
}

fn reference_name(job_id: Uuid) -> Result<CString, OwnerReferenceFailure> {
    if job_id.is_nil() {
        return Err(OwnerReferenceFailure::InvalidReference);
    }
    Ok(CString::new(format!("{}.owner", job_id.simple())).unwrap())
}

fn outcome_name(job_id: Uuid) -> Result<CString, OwnerReferenceFailure> {
    if job_id.is_nil() {
        return Err(OwnerReferenceFailure::InvalidReference);
    }
    Ok(CString::new(format!("{}.outcome", job_id.simple())).unwrap())
}

#[cfg(test)]
#[path = "local_process_owner_outcome_store_tests.rs"]
mod outcome_tests;

fn storage(_: std::io::Error) -> OwnerReferenceFailure {
    OwnerReferenceFailure::StorageUnavailable
}

#[cfg(test)]
#[path = "local_process_owner_store_tests.rs"]
pub(super) mod tests;
