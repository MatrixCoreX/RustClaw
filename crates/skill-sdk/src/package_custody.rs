//! Private package retention for an independently owned job, not a permission grant.
use std::fs::File;
use std::io;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const MAX_RECORD_BYTES: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageCustodyBinding {
    pub job_id: Uuid,
    pub execution_binding_digest: [u8; 32],
    pub owner_reference_digest: [u8; 32],
}

impl PackageCustodyBinding {
    pub fn validate(&self) -> Result<(), PackageCustodyFailure> {
        if self.job_id.is_nil()
            || self.execution_binding_digest == [0; 32]
            || self.owner_reference_digest == [0; 32]
        {
            return Err(PackageCustodyFailure::InvalidBinding);
        }
        Ok(())
    }

    pub(crate) fn filename(&self) -> Result<String, PackageCustodyFailure> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| PackageCustodyFailure::InvalidRecord)?;
        Ok(format!("{}.json", hex::encode(Sha256::digest(bytes))))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageCustodyFailure {
    InvalidBinding,
    InvalidDescriptor,
    InvalidRecord,
    BindingMismatch,
    Released,
    Io(Option<i32>),
}

impl From<io::Error> for PackageCustodyFailure {
    fn from(error: io::Error) -> Self {
        Self::Io(error.raw_os_error())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackageCustodyRecord {
    pub(crate) schema_version: u32,
    pub(crate) binding: PackageCustodyBinding,
    pub(crate) skill_name: String,
    pub(crate) install_dir: String,
    pub(crate) lease_device: u64,
    pub(crate) lease_inode: u64,
    pub(crate) released: bool,
}

impl PackageCustodyRecord {
    pub(crate) fn validate(&self) -> Result<(), PackageCustodyFailure> {
        self.binding.validate()?;
        if self.schema_version != 1
            || self.skill_name.is_empty()
            || self.install_dir.is_empty()
            || self.lease_inode == 0
        {
            return Err(PackageCustodyFailure::InvalidRecord);
        }
        Ok(())
    }
}

// No Debug/serialization: the handles are private custody, never payload IO.
pub struct PackageVersionCustody {
    lease: File,
    record: File,
    binding: PackageCustodyBinding,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl PackageVersionCustody {
    pub fn from_process_handles(
        handles: [File; 2],
        expected: PackageCustodyBinding,
    ) -> Result<Self, PackageCustodyFailure> {
        expected.validate()?;
        let [lease, record] = handles;
        validate_handle(&lease, false)?;
        validate_handle(&record, true)?;
        let value = read_record(&record)?;
        validate_pair(&lease, &value, expected)?;
        if value.released {
            return Err(PackageCustodyFailure::Released);
        }
        Ok(Self {
            lease,
            record,
            binding: expected,
        })
    }

    pub fn binding(&self) -> PackageCustodyBinding {
        self.binding
    }

    pub fn into_process_handles(self) -> Result<[File; 2], PackageCustodyFailure> {
        validate_handle(&self.lease, false)?;
        validate_handle(&self.record, true)?;
        let value = read_record(&self.record)?;
        validate_pair(&self.lease, &value, self.binding)?;
        if value.released {
            return Err(PackageCustodyFailure::Released);
        }
        Ok([self.lease, self.record])
    }

    pub(crate) fn sync_for_transfer(&self) -> Result<(), PackageCustodyFailure> {
        sync_record(&self.record)
    }

    /// The owner calls this only after exact-bound terminal/failure persistence,
    /// or after its consuming startup handshake proves no commit occurred.
    /// The host may also release its exact-bound, never-transferred capsule;
    /// that guard must be disarmed before entering any transfer attempt.
    /// Dropping the handles alone must never clear an uncertain job's record.
    pub fn release(&self) -> Result<(), PackageCustodyFailure> {
        validate_handle(&self.lease, false)?;
        validate_handle(&self.record, true)?;
        let mut value = read_record(&self.record)?;
        validate_pair(&self.lease, &value, self.binding)?;
        if !value.released {
            value.released = true;
            write_record(&self.record, &value)?;
        } else {
            sync_record(&self.record)?;
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_pair(
    lease: &File,
    record: &PackageCustodyRecord,
    expected: PackageCustodyBinding,
) -> Result<(), PackageCustodyFailure> {
    use std::os::unix::fs::MetadataExt;
    record.validate()?;
    let metadata = lease.metadata()?;
    if record.binding != expected
        || record.lease_device != metadata.dev()
        || record.lease_inode != metadata.ino()
    {
        return Err(PackageCustodyFailure::BindingMismatch);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_handle(file: &File, private_record: bool) -> Result<(), PackageCustodyFailure> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    let descriptor_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    let metadata = file.metadata()?;
    if flags < 0
        || flags & libc::O_ACCMODE != libc::O_RDWR
        || flags & libc::O_APPEND != 0
        || descriptor_flags < 0
        || descriptor_flags & libc::FD_CLOEXEC == 0
        || !metadata.is_file()
        || metadata.nlink() != 1
        || (private_record
            && (metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0))
    {
        return Err(PackageCustodyFailure::InvalidDescriptor);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn read_record(file: &File) -> Result<PackageCustodyRecord, PackageCustodyFailure> {
    use std::os::unix::fs::FileExt;
    let length = file.metadata()?.len();
    if length == 0 || length > MAX_RECORD_BYTES as u64 {
        return Err(PackageCustodyFailure::InvalidRecord);
    }
    let mut bytes = vec![0; length as usize];
    file.read_exact_at(&mut bytes, 0)?;
    let record: PackageCustodyRecord =
        serde_json::from_slice(&bytes).map_err(|_| PackageCustodyFailure::InvalidRecord)?;
    record.validate()?;
    Ok(record)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn write_record(
    file: &File,
    record: &PackageCustodyRecord,
) -> Result<(), PackageCustodyFailure> {
    use std::os::unix::fs::FileExt;
    record.validate()?;
    let bytes = serde_json::to_vec(record).map_err(|_| PackageCustodyFailure::InvalidRecord)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(PackageCustodyFailure::InvalidRecord);
    }
    file.write_all_at(&bytes, 0)?;
    file.set_len(bytes.len() as u64)?;
    sync_record(file)?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn sync_record(file: &File) -> Result<(), PackageCustodyFailure> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
