//! Request-owned configuration publication. Per-file replacement is atomic;
//! this deliberately does not claim crash-atomic multi-file transactions.
#[cfg(test)]
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod locks;
#[cfg(test)]
pub(crate) use locks::has_contender;
pub(crate) use locks::with_locked_files;

#[derive(Clone)]
pub(crate) struct Target {
    boundary: PathBuf,
    pub(crate) path: PathBuf,
}

impl Target {
    pub(crate) fn workspace(root: &Path, path: PathBuf) -> Self {
        Self {
            boundary: root.into(),
            path,
        }
    }

    // Only for explicit host-configured paths, not untrusted request paths.
    // A selected configuration/persistence file may live outside the workspace.
    pub(crate) fn configured(root: &Path, path: PathBuf) -> io::Result<Self> {
        // Host-selected paths may contain parent components. Resolve their
        // directory exactly as the loader does, but never follow a final-file
        // symlink. Untrusted workspace targets still use workspace() directly.
        let path = if path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        {
            let parent = path
                .parent()
                .ok_or_else(|| io::Error::other("configuration_parent_missing"))?;
            let filename = path
                .file_name()
                .ok_or_else(|| io::Error::other("configuration_filename_missing"))?;
            locks::key(parent)?.join(filename)
        } else {
            path
        };
        if path.starts_with(root) {
            return Ok(Self::workspace(root, path));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("configuration_parent_missing"))?;
        let mut ancestor = parent;
        while !ancestor.try_exists()? {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| io::Error::other("configuration_parent_missing"))?;
        }
        let boundary = ancestor.canonicalize()?;
        let path = boundary.join(path.strip_prefix(ancestor).map_err(io::Error::other)?);
        Ok(Self { boundary, path })
    }

    pub(crate) fn read(&self) -> io::Result<Option<Vec<u8>>> {
        match crate::secure_workspace_fs::open_workspace_file(&self.boundary, &self.path) {
            Ok(mut file) => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn publish(&self, bytes: &[u8]) -> io::Result<()> {
        crate::secure_workspace_fs::atomic_write_workspace_file(&self.boundary, &self.path, bytes)
    }

    pub(crate) fn same_destination(&self, other: &Self) -> io::Result<bool> {
        Ok(locks::key(&self.path)? == locks::key(&other.path)?)
    }

    fn remove(&self) -> io::Result<()> {
        crate::secure_workspace_fs::remove_workspace_file(&self.boundary, &self.path)
    }
}

pub(crate) struct Edit {
    pub(crate) target: Target,
    pub(crate) original: Option<Vec<u8>>,
    pub(crate) updated: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Changed,
    Write,
    Recovery,
}

#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) index: usize,
    pub(crate) kind: FailureKind,
}

fn restore(edits: &[Edit], publish: &mut impl FnMut(&Target, &[u8]) -> io::Result<()>) -> bool {
    let mut restored = true;
    for edit in edits.iter().rev() {
        match edit.target.read() {
            Ok(current) if current == edit.original => continue,
            Ok(Some(current)) if current == edit.updated => (),
            // Never restore over another writer or an unreadable/unknown file.
            _ => {
                restored = false;
                continue;
            }
        }
        let result = match edit.original.as_deref() {
            Some(original) => publish(&edit.target, original),
            None => edit.target.remove(),
        };
        restored &= result.is_ok();
    }
    restored
}

pub(crate) fn commit_with(
    edits: &[Edit],
    mut publish: impl FnMut(&Target, &[u8]) -> io::Result<()>,
) -> Result<(), Failure> {
    // No writes before all destinations (including mirrors) are readable and
    // unchanged. Secure reads reject symlinks, directories and special files.
    for (index, edit) in edits.iter().enumerate() {
        match edit.target.read() {
            Ok(current) if current == edit.original => (),
            Ok(_) => {
                return Err(Failure {
                    index,
                    kind: FailureKind::Changed,
                })
            }
            Err(_) => {
                return Err(Failure {
                    index,
                    kind: FailureKind::Write,
                })
            }
        }
    }
    for (index, edit) in edits.iter().enumerate() {
        let kind = match edit.target.read() {
            Ok(current) if current == edit.original => {
                if publish(&edit.target, &edit.updated).is_ok() {
                    continue;
                }
                FailureKind::Write
            }
            Ok(_) => FailureKind::Changed,
            Err(_) => FailureKind::Write,
        };
        // A publisher may fail after rename/fsync. Include the current target.
        let restored = restore(&edits[..=index], &mut publish);
        return Err(Failure {
            index,
            kind: if restored {
                kind
            } else {
                FailureKind::Recovery
            },
        });
    }
    Ok(())
}

// The caller owns the read-modify-write lock. Keeping locking outside this
// function avoids a reentrant lock when credentials join the same operation.
pub(crate) fn publish_copies(targets: Vec<Target>, bytes: &[u8]) -> io::Result<()> {
    let mut edits = Vec::<Edit>::new();
    for target in targets {
        let original = target.read()?;
        let key = locks::key(&target.path)?;
        if edits
            .iter()
            .any(|edit| locks::key(&edit.target.path).is_ok_and(|other| other == key))
        {
            continue;
        }
        edits.push(Edit {
            original,
            target,
            updated: bytes.into(),
        });
    }
    commit_with(&edits, |target, bytes| target.publish(bytes)).map_err(|failure| {
        io::Error::other(match failure.kind {
            FailureKind::Changed => "configuration_changed_during_save",
            FailureKind::Write => "configuration_write_failed",
            FailureKind::Recovery => "configuration_rollback_failed",
        })
    })
}

pub(crate) fn persistence_path(root: &Path) -> Option<PathBuf> {
    claw_core::product_identity::env_os("CONFIG_PERSIST_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
}

#[cfg(test)]
mod tests;
