use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

use super::HostFilesystemPolicy;

pub(crate) struct HostPathMetadata {
    pub(crate) file_type: FileType,
    pub(crate) size_bytes: u64,
}

#[derive(Debug)]
pub(crate) enum BoundedFileReadError {
    Io(io::Error),
    LimitExceeded {
        limit_bytes: usize,
        observed_bytes_at_least: usize,
    },
}

impl HostFilesystemPolicy {
    pub(crate) fn read_file(&self, root: Option<&Path>, path: &Path) -> io::Result<Vec<u8>> {
        let (mut file, _) = self.open_file(root, path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn read_file_bounded(
        &self,
        root: Option<&Path>,
        path: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, BoundedFileReadError> {
        let read_limit = max_bytes.checked_add(1).ok_or_else(|| {
            BoundedFileReadError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host_file_read_limit_overflow",
            ))
        })?;
        let (file, _) = self
            .open_file(root, path)
            .map_err(BoundedFileReadError::Io)?;
        let mut bytes = Vec::new();
        file.take(read_limit as u64)
            .read_to_end(&mut bytes)
            .map_err(BoundedFileReadError::Io)?;
        if bytes.len() > max_bytes {
            return Err(BoundedFileReadError::LimitExceeded {
                limit_bytes: max_bytes,
                observed_bytes_at_least: bytes.len(),
            });
        }
        Ok(bytes)
    }

    pub(crate) fn metadata(
        &self,
        root: Option<&Path>,
        path: &Path,
    ) -> io::Result<HostPathMetadata> {
        if root.is_some_and(|root| path == root) {
            self.check_declared_path(path)?;
            let directory = crate::secure_workspace_fs::open_workspace_directory_with_guard(
                root.expect("workspace root checked"),
                path,
                |directory| self.check_directory(directory),
            )?;
            let metadata = File::from(directory).metadata()?;
            return Ok(HostPathMetadata {
                file_type: FileType::Directory,
                size_bytes: metadata.len(),
            });
        }
        let (parent, name) = self.mutation_parent(root, path, false)?;
        self.entry_metadata(&parent, &name)
    }

    pub(crate) fn set_directory_permissions(
        &self,
        root: Option<&Path>,
        path: &Path,
        permissions: std::fs::Permissions,
    ) -> io::Result<()> {
        let target = self.mutation_target(root, path)?;
        let descriptor = crate::secure_workspace_fs::open_workspace_directory_with_guard(
            root.unwrap_or_else(|| Path::new("/")),
            &target,
            |directory| self.check_directory(directory),
        )?;
        let readable = rustix::fs::openat(
            descriptor,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        self.check_directory(&readable)?;
        File::from(readable).set_permissions(permissions)
    }

    pub(crate) fn atomic_write_file_with_options(
        &self,
        root: Option<&Path>,
        path: &Path,
        bytes: &[u8],
        create_parents: bool,
    ) -> io::Result<()> {
        let target = self.mutation_target(root, path)?;
        crate::secure_workspace_fs::atomic_write_workspace_file_with_options_and_guard(
            root.unwrap_or_else(|| Path::new("/")),
            &target,
            bytes,
            create_parents,
            |directory| self.check_directory(directory),
        )
    }

    pub(crate) fn atomic_create_file(
        &self,
        root: Option<&Path>,
        path: &Path,
        bytes: &[u8],
    ) -> io::Result<()> {
        let target = self.mutation_target(root, path)?;
        crate::secure_workspace_fs::atomic_create_workspace_file_with_guard(
            root.unwrap_or_else(|| Path::new("/")),
            &target,
            bytes,
            |directory| self.check_directory(directory),
        )
    }

    pub(crate) fn create_directory(
        &self,
        root: Option<&Path>,
        path: &Path,
        recursive: bool,
    ) -> io::Result<()> {
        let (parent, name) = self.mutation_parent(root, path, recursive)?;
        match rustix::fs::mkdirat(&parent, name.as_c_str(), Mode::from_raw_mode(0o777)) {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) if recursive => {}
            Err(error) => return Err(error.into()),
        }
        let directory = rustix::fs::openat(
            parent,
            name.as_c_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        self.check_directory(&directory)
    }

    pub(crate) fn remove_path(
        &self,
        root: Option<&Path>,
        path: &Path,
        recursive: bool,
    ) -> io::Result<()> {
        let (parent, name) = self.mutation_parent(root, path, false)?;
        // Refuse a known private descendant before deleting any public siblings.
        self.admit_removal(&parent, &name, recursive, 0)?;
        self.remove_entry(&parent, &name, recursive, 0)
    }

    fn mutation_target(&self, root: Option<&Path>, path: &Path) -> io::Result<PathBuf> {
        self.check_declared_path(path)?;
        if root.is_some() {
            return Ok(path.to_path_buf());
        }
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "host_mutation_filename_missing",
            )
        })?;
        let mut ancestor = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "host_mutation_parent_missing")
        })?;
        let mut missing = Vec::new();
        let mut canonical = loop {
            self.check_declared_path(ancestor)?;
            match ancestor.canonicalize() {
                Ok(path) => break path,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing.push(ancestor.file_name().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "host_mutation_parent_missing")
                    })?);
                    ancestor = ancestor.parent().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "host_mutation_parent_missing")
                    })?;
                }
                Err(error) => return Err(error),
            }
        };
        self.check_declared_path(&canonical)?;
        for component in missing.into_iter().rev() {
            canonical.push(component);
        }
        canonical.push(name);
        self.check_declared_path(&canonical)?;
        Ok(canonical)
    }

    pub(super) fn mutation_parent(
        &self,
        root: Option<&Path>,
        path: &Path,
        create_parents: bool,
    ) -> io::Result<(OwnedFd, CString)> {
        let target = self.mutation_target(root, path)?;
        crate::secure_workspace_fs::open_workspace_parent_with_guard(
            root.unwrap_or_else(|| Path::new("/")),
            &target,
            create_parents,
            |directory| self.check_directory(directory),
        )
    }

    fn entry_metadata(&self, parent: &OwnedFd, name: &CString) -> io::Result<HostPathMetadata> {
        self.check_directory(parent)?;
        let metadata = rustix::fs::statat(parent, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode as _);
        if file_type == FileType::Directory
            && self
                .directories
                .iter()
                .any(|private| private.identity == (metadata.st_dev as u64, metadata.st_ino as u64))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                super::PrivateAccessDenied,
            ));
        }
        Ok(HostPathMetadata {
            file_type,
            size_bytes: u64::try_from(metadata.st_size).unwrap_or_default(),
        })
    }

    fn open_removal_directory(&self, parent: &OwnedFd, name: &CString) -> io::Result<OwnedFd> {
        let directory = rustix::fs::openat(
            parent,
            name.as_c_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        self.check_directory(&directory)?;
        Ok(directory)
    }

    fn admit_removal(
        &self,
        parent: &OwnedFd,
        name: &CString,
        recursive: bool,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host_mutation_depth_exceeded",
            ));
        }
        if self.entry_metadata(parent, name)?.file_type == FileType::Directory && recursive {
            let directory = self.open_removal_directory(parent, name)?;
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                if matches!(entry.file_name().to_bytes(), b"." | b"..") {
                    continue;
                }
                self.admit_removal(&directory, &entry.file_name().to_owned(), true, depth + 1)?;
            }
        }
        Ok(())
    }

    fn remove_entry(
        &self,
        parent: &OwnedFd,
        name: &CString,
        recursive: bool,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host_mutation_depth_exceeded",
            ));
        }
        let kind = self.entry_metadata(parent, name)?.file_type;
        if kind == FileType::Directory && recursive {
            let directory = self.open_removal_directory(parent, name)?;
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                if matches!(entry.file_name().to_bytes(), b"." | b"..") {
                    continue;
                }
                self.remove_entry(&directory, &entry.file_name().to_owned(), true, depth + 1)?;
            }
        }
        self.entry_metadata(parent, name)?;
        rustix::fs::unlinkat(
            parent,
            name.as_c_str(),
            if kind == FileType::Directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            },
        )?;
        Ok(())
    }
}
