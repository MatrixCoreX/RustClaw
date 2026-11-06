//! Single-file publication in a host-owned receipt directory.
//! No process-wide umask changes, target-link following, or multi-pointer transaction claim.
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{SkillSdkError, SkillSdkResult};

struct TemporaryFile {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        // Close first so cleanup also works on systems that cannot unlink an open file.
        drop(self.file.take());
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn target_metadata(path: &Path) -> SkillSdkResult<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata)),
        Ok(_) => Err(SkillSdkError::new(
            "atomic_write_target_invalid",
            path.display().to_string(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn same_target(before: &Option<Metadata>, after: &Option<Metadata>) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            if before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || before.permissions().readonly() != after.permissions().readonly()
            {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                (
                    before.dev(),
                    before.ino(),
                    before.uid(),
                    before.gid(),
                    before.mode(),
                    before.ctime(),
                    before.ctime_nsec(),
                ) == (
                    after.dev(),
                    after.ino(),
                    after.uid(),
                    after.gid(),
                    after.mode(),
                    after.ctime(),
                    after.ctime_nsec(),
                )
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

fn preserve_metadata(file: &File, existing: &Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        let staged = file.metadata()?;
        if (staged.uid(), staged.gid()) != (existing.uid(), existing.gid()) {
            // SAFETY: the exclusive temporary file descriptor is owned by this invocation.
            if unsafe { libc::fchown(file.as_raw_fd(), existing.uid(), existing.gid()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    // Writing/chown can clear mode bits. Restore only after both have completed.
    file.set_permissions(existing.permissions())
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> SkillSdkResult<()> {
    write_with(path, |file| file.write_all(bytes))
}

pub(super) fn write_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> SkillSdkResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SkillSdkError::new("atomic_write_parent_missing", path.display().to_string())
    })?;
    fs::create_dir_all(parent)?;
    let existing = target_metadata(path)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&temporary)?;
    let mut staged = TemporaryFile {
        path: temporary,
        file: Some(file),
        committed: false,
    };
    let file = staged.file.as_mut().expect("owned temporary file");
    // Capture the real creation policy (including umask) without reading/changing global state.
    let creation_permissions = file.metadata()?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    write(file)?;
    file.flush()?;
    match existing.as_ref() {
        Some(metadata) => preserve_metadata(file, metadata).map_err(|error| {
            SkillSdkError::new("atomic_write_metadata_failed", error.to_string())
        })?,
        None => file.set_permissions(creation_permissions)?,
    }
    file.sync_all()?;
    if !same_target(&existing, &target_metadata(path)?) {
        return Err(SkillSdkError::new(
            "atomic_write_target_changed",
            path.display().to_string(),
        ));
    }
    drop(staged.file.take());
    fs::rename(&staged.path, path).map_err(|error| {
        SkillSdkError::new(
            "atomic_write_commit_failed",
            format!("path={} error={error}", path.display()),
        )
    })?;
    staged.committed = true;
    // No new fallible operation after publication: installers must not delete a
    // successfully referenced package because a later directory sync reported failure.
    Ok(())
}
