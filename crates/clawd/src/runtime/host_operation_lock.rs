use super::HostFilesystemPolicy;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

impl HostFilesystemPolicy {
    // Cooperative operation exclusion, not host/permission admission. Callers
    // retain the returned descriptor for the entire synchronous operation.
    pub(crate) fn try_lock_operation_file(
        &self,
        root: Option<&Path>,
        path: &Path,
    ) -> io::Result<File> {
        let (parent, name) = self.mutation_parent(root, path, false)?;
        self.check_directory(&parent)?;
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_CLOEXEC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK,
                0o600 as libc::c_uint,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.len() != 0
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "host_operation_lock_invalid",
            ));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.check_directory(&parent)?;
        let current = rustix::fs::statat(
            &parent,
            name.as_c_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )?;
        if (metadata.dev(), metadata.ino()) != (current.st_dev as u64, current.st_ino as u64) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "host_operation_lock_changed",
            ));
        }
        Ok(file)
    }
}

#[cfg(test)]
#[path = "host_operation_lock_tests.rs"]
mod tests;
