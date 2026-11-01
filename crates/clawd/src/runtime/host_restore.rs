use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{mkdirat, openat, Mode, OFlags};

use super::{HostAccessMode, HostDirectory, HostFilesystemPolicy};

fn child_name(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.contains(&b'/') || matches!(bytes, b"." | b"..") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "host_restore_child_invalid",
        ));
    }
    CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "host_restore_child_invalid"))
}

impl HostFilesystemPolicy {
    pub(crate) fn create_restore_directory_at(
        &self,
        parent: &HostDirectory,
        name: &OsStr,
    ) -> io::Result<HostDirectory> {
        let name_c = child_name(name)?;
        let parent_descriptor = parent.file.try_clone()?.into();
        self.check_directory(&parent_descriptor)?;
        mkdirat(&parent.file, name_c.as_c_str(), Mode::from_raw_mode(0o700))?;
        let descriptor = openat(
            &parent.file,
            name_c.as_c_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        self.check_directory(&descriptor)?;
        let directory =
            HostDirectory::from_descriptor(descriptor, parent.canonical_path().join(name))?;
        HostAccessMode::try_from(0o700)
            .expect("ordinary staging mode")
            .apply(&directory.file)?;
        Ok(directory)
    }

    pub(crate) fn restore_file_at(
        &self,
        parent: &HostDirectory,
        name: &OsStr,
        bytes: &[u8],
        mode: Option<HostAccessMode>,
    ) -> io::Result<()> {
        let name = child_name(name)?;
        let descriptor = parent.file.try_clone()?.into();
        self.check_directory(&descriptor)?;
        crate::secure_workspace_fs::atomic_restore_file_in_directory(
            descriptor,
            name,
            bytes,
            mode.map(u32::from),
        )
    }

    pub(crate) fn finish_restore_directory(
        &self,
        directory: &HostDirectory,
        mode: Option<HostAccessMode>,
    ) -> io::Result<()> {
        let descriptor = directory.file.try_clone()?.into();
        self.check_directory(&descriptor)?;
        if let Some(mode) = mode {
            mode.apply(&directory.file)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "host_restore_tests.rs"]
mod tests;
