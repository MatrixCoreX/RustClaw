use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::owner_reference::{AuthorityFingerprint, OwnerAuthorityKey, OwnerReferenceFailure};
use super::owner_store::OwnerReferenceStore;

pub(super) const DIRECTORY: &str = "process-owners-v1";
pub(super) const KEY_FILE: &str = "authority.key";
const KEY_HEADER: &[u8; 8] = b"OWNKEY1\0";
const KEY_BYTES: usize = KEY_HEADER.len() + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthorityOperation {
    AdmitParent,
    CreateDirectory,
    OpenDirectory,
    CreateKey,
    GenerateKey,
    OpenKey,
    ReadKey,
    WriteKey,
    SyncKey,
    SyncDirectory,
    SyncParent,
    OpenReferenceStore,
    VerifyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthorityFailureCode {
    InvalidDirectory,
    AlreadyInitialized,
    Missing,
    InvalidKey,
    EntropyUnavailable,
    StorageUnavailable,
    AuthorityMismatch,
}

// Machine diagnostics only; never retain a private path, key bytes or IO prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthorityFailure {
    pub(crate) operation: AuthorityOperation,
    pub(crate) error_code: AuthorityFailureCode,
    pub(crate) os_error: Option<i32>,
}

// This closed host-only broker is deliberately not a general SecretsBroker.
// The caller admits/fences the parent from every payload before using it.
pub(crate) struct OwnerAuthorityBroker {
    parent: File,
}

impl OwnerAuthorityBroker {
    pub(crate) fn from_admitted_directory(parent: File) -> Result<Self, AuthorityFailure> {
        validate_directory(&parent, AuthorityOperation::AdmitParent)?;
        Ok(Self { parent })
    }

    // Caller must prove first installation from the host journal, never use
    // this as a fallback when load_existing fails or expected state is missing.
    // Failed initialization retains its claim; neither method repairs it.
    pub(super) fn initialize_new(&self) -> Result<OwnerReferenceStore, AuthorityFailure> {
        validate_directory(&self.parent, AuthorityOperation::AdmitParent)?;
        let name = CString::new(DIRECTORY).unwrap();
        if unsafe { libc::mkdirat(self.parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(AuthorityFailure {
                    operation: AuthorityOperation::CreateDirectory,
                    error_code: AuthorityFailureCode::AlreadyInitialized,
                    os_error: error.raw_os_error(),
                });
            }
            return Err(io_failure(AuthorityOperation::CreateDirectory, error));
        }
        let directory = self.open_directory()?;
        let mut bytes = Zeroizing::new([0_u8; KEY_BYTES]);
        bytes[..KEY_HEADER.len()].copy_from_slice(KEY_HEADER);
        OsRng
            .try_fill_bytes(&mut bytes[KEY_HEADER.len()..])
            .map_err(|_| {
                failure(
                    AuthorityOperation::GenerateKey,
                    AuthorityFailureCode::EntropyUnavailable,
                )
            })?;
        let key = decode_key(&bytes[..], AuthorityOperation::GenerateKey)?;
        let mut file = open_at(
            &directory,
            KEY_FILE,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            AuthorityOperation::CreateKey,
        )?;
        validate_key_file(&file, AuthorityOperation::CreateKey)?;
        file.write_all(&bytes[..])
            .map_err(|error| io_failure(AuthorityOperation::WriteKey, error))?;
        validate_key_file(&file, AuthorityOperation::WriteKey)?;
        file.sync_all()
            .map_err(|error| io_failure(AuthorityOperation::SyncKey, error))?;
        self.finish(directory, key)
    }

    pub(super) fn load_expected(
        &self,
        expected: &AuthorityFingerprint,
    ) -> Result<OwnerReferenceStore, AuthorityFailure> {
        let store = self.load_existing()?;
        let actual = store.authority_fingerprint().map_err(|_| {
            failure(
                AuthorityOperation::VerifyIdentity,
                AuthorityFailureCode::InvalidKey,
            )
        })?;
        if &actual != expected {
            return Err(failure(
                AuthorityOperation::VerifyIdentity,
                AuthorityFailureCode::AuthorityMismatch,
            ));
        }
        Ok(store)
    }

    fn load_existing(&self) -> Result<OwnerReferenceStore, AuthorityFailure> {
        validate_directory(&self.parent, AuthorityOperation::AdmitParent)?;
        let directory = self.open_directory()?;
        let mut file = open_at(
            &directory,
            KEY_FILE,
            libc::O_RDWR,
            AuthorityOperation::OpenKey,
        )?;
        validate_key_file(&file, AuthorityOperation::ReadKey)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_BYTES + 1));
        (&mut file)
            .take(KEY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| io_failure(AuthorityOperation::ReadKey, error))?;
        let key = decode_key(&bytes, AuthorityOperation::ReadKey)?;
        validate_key_file(&file, AuthorityOperation::ReadKey)?;
        file.sync_all()
            .map_err(|error| io_failure(AuthorityOperation::SyncKey, error))?;
        self.finish(directory, key)
    }

    fn open_directory(&self) -> Result<File, AuthorityFailure> {
        let directory = open_at(
            &self.parent,
            DIRECTORY,
            libc::O_RDONLY | libc::O_DIRECTORY,
            AuthorityOperation::OpenDirectory,
        )?;
        validate_directory(&directory, AuthorityOperation::OpenDirectory)?;
        Ok(directory)
    }

    fn finish(
        &self,
        directory: File,
        key: OwnerAuthorityKey,
    ) -> Result<OwnerReferenceStore, AuthorityFailure> {
        directory
            .sync_all()
            .map_err(|error| io_failure(AuthorityOperation::SyncDirectory, error))?;
        self.parent
            .sync_all()
            .map_err(|error| io_failure(AuthorityOperation::SyncParent, error))?;
        OwnerReferenceStore::from_admitted_directory(directory, key).map_err(|error| {
            failure(
                AuthorityOperation::OpenReferenceStore,
                if error == OwnerReferenceFailure::InvalidDirectory {
                    AuthorityFailureCode::InvalidDirectory
                } else {
                    AuthorityFailureCode::StorageUnavailable
                },
            )
        })
    }
}

fn decode_key(
    bytes: &[u8],
    operation: AuthorityOperation,
) -> Result<OwnerAuthorityKey, AuthorityFailure> {
    if bytes.len() != KEY_BYTES || &bytes[..KEY_HEADER.len()] != KEY_HEADER {
        return Err(failure(operation, AuthorityFailureCode::InvalidKey));
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    key.copy_from_slice(&bytes[KEY_HEADER.len()..]);
    OwnerAuthorityKey::from_zeroizing(key)
        .map_err(|_| failure(operation, AuthorityFailureCode::InvalidKey))
}

fn open_at(
    parent: &File,
    name: &str,
    flags: i32,
    operation: AuthorityOperation,
) -> Result<File, AuthorityFailure> {
    let name = CString::new(name).unwrap();
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(io_failure(operation, std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

pub(super) fn validate_directory(
    file: &File,
    operation: AuthorityOperation,
) -> Result<(), AuthorityFailure> {
    let metadata = file
        .metadata()
        .map_err(|error| io_failure(operation, error))?;
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o700
        || flags < 0
        || flags & libc::FD_CLOEXEC == 0
    {
        return Err(failure(operation, AuthorityFailureCode::InvalidDirectory));
    }
    Ok(())
}

fn validate_key_file(file: &File, operation: AuthorityOperation) -> Result<(), AuthorityFailure> {
    let metadata = file
        .metadata()
        .map_err(|error| io_failure(operation, error))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(failure(operation, AuthorityFailureCode::InvalidKey));
    }
    Ok(())
}

fn failure(operation: AuthorityOperation, error_code: AuthorityFailureCode) -> AuthorityFailure {
    AuthorityFailure {
        operation,
        error_code,
        os_error: None,
    }
}

fn io_failure(operation: AuthorityOperation, error: std::io::Error) -> AuthorityFailure {
    AuthorityFailure {
        operation,
        error_code: if error.kind() == std::io::ErrorKind::NotFound {
            AuthorityFailureCode::Missing
        } else {
            AuthorityFailureCode::StorageUnavailable
        },
        os_error: error.raw_os_error(),
    }
}

#[cfg(test)]
#[path = "local_process_owner_authority_tests.rs"]
pub(super) mod tests;
