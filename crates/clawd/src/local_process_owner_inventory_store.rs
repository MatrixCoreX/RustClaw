use super::*;
use rustix::fs::Dir;

pub(crate) const MAX_PENDING_OWNERS: usize = 4096;

pub(crate) struct OwnerInventoryCounts {
    pub(crate) examined: u64,
    pub(crate) pending: u64,
}

// Fresh open descriptions: locking a clone could upgrade/unlock another
// operation's flock. Writers share this lock; enumeration owns it exclusively.
pub(super) struct RecordSetLock(File);

impl OwnerReferenceStore {
    pub(super) fn record_set_lock(
        &self,
        enumerate: bool,
    ) -> Result<RecordSetLock, OwnerReferenceFailure> {
        self.validate_directory()?;
        let descriptor = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let mode = if enumerate {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
            return Err(
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                    OwnerReferenceFailure::Busy
                } else {
                    OwnerReferenceFailure::StorageUnavailable
                },
            );
        }
        Ok(RecordSetLock(file))
    }

    // A complete, pinned-directory scan, not a public filename/PID inventory.
    // The callback is private transport only; it must never publish credentials.
    pub(crate) fn visit_pending_resources(
        &self,
        mut emit: impl FnMut(
            &DurableOwnerReference,
            &super::super::owner_resources::OwnerResourceBudget,
        ) -> Result<(), OwnerReferenceFailure>,
    ) -> Result<OwnerInventoryCounts, OwnerReferenceFailure> {
        let lock = self.record_set_lock(true)?;
        let mut counts = OwnerInventoryCounts {
            examined: 0,
            pending: 0,
        };
        for entry in
            Dir::read_from(&lock.0).map_err(|_| OwnerReferenceFailure::StorageUnavailable)?
        {
            let entry = entry.map_err(|_| OwnerReferenceFailure::StorageUnavailable)?;
            let name = entry.file_name().to_bytes();
            if matches!(name, b"." | b"..")
                || name == super::super::owner_authority::KEY_FILE.as_bytes()
            {
                continue;
            }
            // An unfinished temporary never authorizes a commit. Publication
            // cleanup also holds a writer lock, so these are retained leftovers.
            if name.starts_with(b".owner-ref-") && name.ends_with(b".tmp") {
                continue;
            }
            let text =
                std::str::from_utf8(name).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
            let (id, reference_entry) = if let Some(id) = text.strip_suffix(".owner") {
                (id, true)
            } else if let Some(id) = text.strip_suffix(".outcome") {
                (id, false)
            } else if let Some(id) = text.strip_suffix(".resources.json") {
                (id, false)
            } else {
                return Err(OwnerReferenceFailure::InvalidReference);
            };
            let job = Uuid::parse_str(id).map_err(|_| OwnerReferenceFailure::InvalidReference)?;
            let canonical = if text.ends_with(".resources.json") {
                job.to_string()
            } else {
                job.simple().to_string()
            };
            if id != canonical || job.is_nil() {
                return Err(OwnerReferenceFailure::InvalidReference);
            }
            let reference = self
                .load(job)?
                .ok_or(OwnerReferenceFailure::InvalidReference)?;
            if !reference_entry {
                // Reject orphan records rather than quietly omitting a job.
                continue;
            }
            counts.examined = counts
                .examined
                .checked_add(1)
                .ok_or(OwnerReferenceFailure::InventoryLimitExceeded)?;
            let budget = self.load_resources(&reference)?;
            if self.load_outcome(&reference)?.is_some() {
                continue;
            }
            let budget = budget.ok_or(OwnerReferenceFailure::MissingResourceBudget)?;
            if counts.pending >= MAX_PENDING_OWNERS as u64 {
                return Err(OwnerReferenceFailure::InventoryLimitExceeded);
            }
            emit(&reference, &budget)?;
            counts.pending += 1;
        }
        self.validate_directory()?;
        Ok(counts)
    }
}

#[cfg(test)]
#[path = "local_process_owner_inventory_store_tests.rs"]
mod tests;
