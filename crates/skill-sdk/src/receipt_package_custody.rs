use super::*;
use crate::package_custody::{PackageCustodyBinding, PackageCustodyRecord, PackageVersionCustody};

const DIRECTORY: &str = "owner-leases";

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn custody_error(error: crate::package_custody::PackageCustodyFailure) -> SkillSdkError {
    SkillSdkError::new("skill_owner_lease_invalid", format!("{error:?}"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_child(parent: &File, name: &str) -> std::io::Result<File> {
    use rustix::fs::{openat, Mode, OFlags};
    let file = File::from(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::from_raw_os_error(libc::EACCES));
    }
    Ok(file)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_child(parent: &File, name: &str) -> std::io::Result<File> {
    match rustix::fs::mkdirat(parent, name, rustix::fs::Mode::RWXU) {
        Ok(()) => {}
        Err(rustix::io::Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let child = open_child(parent, name)?;
    parent.sync_all()?;
    Ok(child)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn skill_directory(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SkillVersionLease {
    pub fn retain_owner_custody(
        &self,
        binding: PackageCustodyBinding,
    ) -> SkillSdkResult<PackageVersionCustody> {
        use rustix::fs::{openat, Mode, OFlags};
        use std::os::unix::fs::MetadataExt;
        binding.validate().map_err(custody_error)?;
        let lease = self.retain_process_handle()?;
        let metadata = lease.metadata()?;
        let skill = skill_directory(&self.store.skill_root(&self.skill_name)?)?;
        let owners = create_child(&skill, DIRECTORY)?;
        let directory = create_child(&owners, &self.install_dir)?;
        let filename = binding.filename().map_err(custody_error)?;
        let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
        let record = match openat(
            &directory,
            filename.as_str(),
            flags | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(file) => {
                let record = File::from(file);
                fs2::FileExt::try_lock_exclusive(&record).map_err(initializing_error)?;
                let value = PackageCustodyRecord {
                    schema_version: 1,
                    binding,
                    skill_name: self.skill_name.clone(),
                    install_dir: self.install_dir.clone(),
                    lease_device: metadata.dev(),
                    lease_inode: metadata.ino(),
                    released: false,
                };
                crate::package_custody::write_record(&record, &value).map_err(custody_error)?;
                directory.sync_all()?;
                crate::package_custody::sync_record(&record).map_err(custody_error)?;
                record
            }
            Err(rustix::io::Errno::EXIST) => File::from(
                openat(&directory, filename.as_str(), flags, Mode::empty())
                    .map_err(std::io::Error::from)?,
            ),
            Err(error) => return Err(std::io::Error::from(error).into()),
        };
        // A failed/ongoing publisher cannot lend an uncommitted record. An
        // existing open repeats the barriers in case its creator lost the ACK.
        fs2::FileExt::try_lock_shared(&record).map_err(initializing_error)?;
        let custody = PackageVersionCustody::from_process_handles([lease, record], binding)
            .map_err(custody_error)?;
        directory.sync_all()?;
        custody.sync_for_transfer().map_err(custody_error)?;
        Ok(custody)
    }
}

impl InstallReceiptStore {
    // Called while the collector holds the exact version's exclusive lock.
    // Writers keep their shared handle until the durable record is published.
    pub(super) fn version_has_active_owner_custody(
        &self,
        skill_name: &str,
        install_dir: &str,
        lease: &File,
    ) -> SkillSdkResult<bool> {
        let root = self.skill_root(skill_name)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use rustix::fs::{openat, unlinkat, AtFlags, Dir, Mode, OFlags};
            use std::os::unix::fs::MetadataExt;
            let skill = skill_directory(&root)?;
            let owners = match open_child(&skill, DIRECTORY) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(_) => return Ok(true),
            };
            let directory = match open_child(&owners, install_dir) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(_) => return Ok(true),
            };
            let expected = lease.metadata()?;
            let mut active = false;
            let mut entries = Dir::read_from(&directory).map_err(std::io::Error::from)?;
            while let Some(entry) = entries.read() {
                let entry = entry.map_err(std::io::Error::from)?;
                let name = entry.file_name();
                if matches!(name.to_bytes(), b"." | b"..") {
                    continue;
                }
                let record = openat(
                    &directory,
                    name,
                    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .ok()
                .and_then(|fd| {
                    let file = File::from(fd);
                    let metadata = file.metadata().ok()?;
                    if !metadata.is_file()
                        || metadata.nlink() != 1
                        || metadata.uid() != unsafe { libc::geteuid() }
                        || metadata.mode() & 0o077 != 0
                    {
                        return None;
                    }
                    crate::package_custody::read_record(&file).ok()
                });
                let releasable = record.is_some_and(|record| {
                    record.released
                        && record.skill_name == skill_name
                        && record.install_dir == install_dir
                        && record.lease_device == expected.dev()
                        && record.lease_inode == expected.ino()
                        && record.binding.filename().ok().as_deref().map(str::as_bytes)
                            == Some(name.to_bytes())
                });
                if releasable {
                    unlinkat(&directory, name, AtFlags::empty()).map_err(std::io::Error::from)?;
                } else {
                    active = true;
                }
            }
            if !active {
                unlinkat(&owners, install_dir, AtFlags::REMOVEDIR).map_err(std::io::Error::from)?;
                owners.sync_all()?;
            }
            Ok(active)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = lease;
            presence_without_following(&root.join(DIRECTORY))
        }
    }

    pub(super) fn has_remaining_owner_custody(&self, skill_name: &str) -> SkillSdkResult<bool> {
        let root = self.skill_root(skill_name)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let skill = skill_directory(&root)?;
            let owners = match open_child(&skill, DIRECTORY) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(_) => return Ok(true),
            };
            let mut entries = rustix::fs::Dir::read_from(&owners).map_err(std::io::Error::from)?;
            while let Some(entry) = entries.read() {
                let entry = entry.map_err(std::io::Error::from)?;
                if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            presence_without_following(&root.join(DIRECTORY))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn presence_without_following(path: &Path) -> SkillSdkResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn initializing_error(error: std::io::Error) -> SkillSdkError {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        SkillSdkError::new(
            "skill_owner_lease_initializing",
            "owner_lease_publication_pending",
        )
    } else {
        error.into()
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "receipt_package_custody_tests.rs"]
mod tests;
