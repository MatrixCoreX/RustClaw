use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::private_filesystem_policy::DirectoryExclusions;

#[path = "host_directory.rs"]
mod directory;
use directory::HostDirectory;
#[path = "host_mutation.rs"]
mod mutation;
pub(crate) use mutation::BoundedFileReadError;
#[path = "host_operation_lock.rs"]
mod operation_lock;

#[derive(Debug)]
struct PrivateAccessDenied;

impl std::fmt::Display for PrivateAccessDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("host_private_directory_access_denied")
    }
}

impl std::error::Error for PrivateAccessDenied {}

struct PrivateDirectory {
    path: PathBuf,
    descriptor: File,
    identity: (u64, u64),
}

// Host metadata only. Neither directory pinning nor these filesystem checks
// prove process/descriptor/survivor admission or authorize loading key material.
pub(crate) struct HostFilesystemPolicy {
    directories: Vec<PrivateDirectory>,
}

impl HostFilesystemPolicy {
    pub(crate) fn is_private_access_denied(error: &io::Error) -> bool {
        error
            .get_ref()
            .is_some_and(|cause| cause.is::<PrivateAccessDenied>())
    }

    pub(crate) fn without_private_authority() -> Self {
        Self {
            directories: Vec::new(),
        }
    }

    pub(crate) fn pin_directories(paths: &[PathBuf], execution_root: &Path) -> io::Result<Self> {
        let exclusions = DirectoryExclusions::admit(paths, execution_root)
            .map_err(|code| io::Error::new(io::ErrorKind::PermissionDenied, code))?;
        let mut directories = Vec::new();
        for path in exclusions.paths() {
            let descriptor = crate::secure_workspace_fs::open_workspace_directory_with_guard(
                Path::new("/"),
                path,
                |_| Ok(()),
            )?;
            let descriptor = File::from(descriptor);
            let metadata = descriptor.metadata()?;
            let current = std::fs::symlink_metadata(path)?;
            if !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o7777 != 0o700
                || metadata.dev() != current.dev()
                || metadata.ino() != current.ino()
                || current.file_type().is_symlink()
            {
                return Err(denied("host_private_directory_invalid"));
            }
            directories.push(PrivateDirectory {
                path: path.clone(),
                descriptor,
                identity: (metadata.dev(), metadata.ino()),
            });
        }
        Ok(Self { directories })
    }

    pub(crate) fn process_directories(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for private in &self.directories {
            let current = crate::secure_workspace_fs::open_workspace_directory_with_guard(
                Path::new("/"),
                &private.path,
                |_| Ok(()),
            )
            .map_err(|_| denied("host_private_directory_changed"))?;
            let metadata = File::from(current).metadata()?;
            let pinned = private.descriptor.metadata()?;
            if metadata.dev() != pinned.dev()
                || metadata.ino() != pinned.ino()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o7777 != 0o700
            {
                return Err(denied("host_private_directory_changed"));
            }
            paths.push(private.path.clone());
        }
        Ok(paths)
    }

    pub(crate) fn open_file(
        &self,
        workspace_root: Option<&Path>,
        path: &Path,
    ) -> io::Result<(File, PathBuf)> {
        self.check_declared_path(path)?;
        let canonical = path.canonicalize()?;
        self.check_declared_path(&canonical)?;
        let file = if self.directories.is_empty() {
            match workspace_root {
                Some(root) => crate::secure_workspace_fs::open_workspace_file(root, path)?,
                None => crate::secure_workspace_fs::open_workspace_file_with_guard(
                    Path::new("/"),
                    &canonical,
                    |_| Ok(()),
                )?,
            }
        } else {
            let (root, target) = match workspace_root {
                Some(root) => (root, path),
                None => (Path::new("/"), canonical.as_path()),
            };
            crate::secure_workspace_fs::open_workspace_file_with_guard(root, target, |directory| {
                self.check_directory(directory)
            })?
        };
        Ok((file, canonical))
    }

    pub(crate) fn open_directory(
        &self,
        workspace_root: Option<&Path>,
        path: &Path,
    ) -> io::Result<HostDirectory> {
        self.check_declared_path(path)?;
        let canonical = path.canonicalize()?;
        self.check_declared_path(&canonical)?;
        let descriptor = crate::secure_workspace_fs::open_workspace_directory_with_guard(
            workspace_root.unwrap_or_else(|| Path::new("/")),
            &canonical,
            |directory| self.check_directory(directory),
        )?;
        HostDirectory::from_descriptor(descriptor, canonical)
    }

    pub(crate) fn atomic_write_file(
        &self,
        workspace_root: &Path,
        path: &Path,
        bytes: &[u8],
    ) -> io::Result<()> {
        self.atomic_write_file_with_options(Some(workspace_root), path, bytes, true)
    }

    fn check_declared_path(&self, path: &Path) -> io::Result<()> {
        if self
            .directories
            .iter()
            .any(|private| path.starts_with(&private.path))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                PrivateAccessDenied,
            ));
        }
        Ok(())
    }

    fn check_directory(&self, directory: &OwnedFd) -> io::Result<()> {
        let metadata = File::from(directory.try_clone()?).metadata()?;
        let identity = (metadata.dev(), metadata.ino());
        if self
            .directories
            .iter()
            .any(|private| private.identity == identity)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                PrivateAccessDenied,
            ));
        }
        Ok(())
    }
}

fn denied(code: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, code)
}

#[cfg(test)]
#[path = "host_filesystem_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "host_mutation_tests.rs"]
mod mutation_tests;
