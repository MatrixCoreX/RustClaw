use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{statat, AtFlags, Dir, FileType, Mode, OFlags};

pub(crate) struct HostDirectory {
    pub(super) file: File,
    canonical_path: PathBuf,
}

pub(crate) struct HostDirectoryEntry {
    pub(crate) name: OsString,
    pub(crate) file_type: FileType,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) modified_ts: Option<u64>,
    pub(crate) modified_ns: u64,
}

impl HostDirectory {
    pub(super) fn from_descriptor(directory: OwnedFd, canonical_path: PathBuf) -> io::Result<Self> {
        // Upgrade the admitted search handle without resolving its pathname again.
        let readable = rustix::fs::openat(
            directory,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        Ok(Self {
            file: File::from(readable),
            canonical_path,
        })
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub(crate) fn access_mode(&self) -> io::Result<super::HostAccessMode> {
        super::HostAccessMode::capture(&self.file.metadata()?)
    }

    pub(crate) fn verify_descriptor_headroom(&self, count: usize) -> io::Result<()> {
        let mut probes = Vec::with_capacity(count);
        for _ in 0..count {
            probes.push(self.file.try_clone()?);
        }
        Ok(())
    }

    pub(crate) fn entries(&self) -> io::Result<Vec<HostDirectoryEntry>> {
        self.entries_iter()?.collect()
    }

    pub(crate) fn entries_iter(
        &self,
    ) -> io::Result<impl Iterator<Item = io::Result<HostDirectoryEntry>> + '_> {
        Ok(
            Dir::read_from(&self.file)?.filter_map(move |entry| match entry {
                Err(error) => Some(Err(error.into())),
                Ok(entry) if matches!(entry.file_name().to_bytes(), b"." | b"..") => None,
                Ok(entry) => Some(self.describe_entry(entry.file_name())),
            }),
        )
    }

    fn describe_entry(&self, name: &std::ffi::CStr) -> io::Result<HostDirectoryEntry> {
        // Enumeration and lstat stay on the held directory even after rename.
        let metadata = statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode as _);
        Ok(HostDirectoryEntry {
            name: std::ffi::OsStr::from_bytes(name.to_bytes()).to_owned(),
            file_type,
            size_bytes: (file_type == FileType::RegularFile)
                .then(|| u64::try_from(metadata.st_size).ok())
                .flatten(),
            modified_ts: u64::try_from(metadata.st_mtime).ok(),
            modified_ns: u64::try_from(metadata.st_mtime)
                .ok()
                .map(|seconds| {
                    seconds
                        .saturating_mul(1_000_000_000)
                        .saturating_add(u64::try_from(metadata.st_mtime_nsec).unwrap_or_default())
                })
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
#[path = "host_directory_tests.rs"]
mod tests;
