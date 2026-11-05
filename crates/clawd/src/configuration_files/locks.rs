use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

type FileLocks = BTreeMap<PathBuf, Weak<Mutex<()>>>;
static LOCKS: OnceLock<Mutex<FileLocks>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn has_contender(path: &Path) -> bool {
    let key = key(path).unwrap();
    LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .is_some_and(|lock| lock.strong_count() > 1)
}

// Canonicalize the existing prefix, including platform aliases such as /tmp on
// macOS. Missing directories must not be created just to acquire a lock.
pub(super) fn key(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(io::Error::other("configuration_path_not_absolute"));
    }
    if !path.components().any(|part| part == Component::ParentDir) {
        match path.canonicalize() {
            Ok(resolved) => return Ok(resolved),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
    }
    // Some platform canonicalizers collapse `file/..` without rejecting the
    // non-directory prefix. Walk directories before processing parent tokens;
    // this also resolves directory symlinks before their following `..`.
    let mut resolved = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => (),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                resolved.push(part);
                match resolved.canonicalize() {
                    Ok(physical) => {
                        if components.peek().is_some() && !physical.metadata()?.is_dir() {
                            return Err(io::Error::other("configuration_parent_not_directory"));
                        }
                        resolved = physical;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved)
}

pub(crate) fn with_locked_files<T>(
    paths: &[PathBuf],
    operation: impl FnOnce() -> T,
) -> io::Result<T> {
    let mut keys = paths
        .iter()
        .map(|path| key(path))
        .collect::<io::Result<Vec<_>>>()?;
    keys.sort();
    keys.dedup();
    let locks = {
        let mut registry = LOCKS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, value| value.strong_count() > 0);
        keys.into_iter()
            .map(|key| {
                if let Some(lock) = registry.get(&key).and_then(Weak::upgrade) {
                    lock
                } else {
                    let lock = Arc::new(Mutex::new(()));
                    registry.insert(key, Arc::downgrade(&lock));
                    lock
                }
            })
            .collect::<Vec<_>>()
    };
    // Poison does not represent protected application state: these mutexes only
    // coordinate fresh filesystem reads. A failed request must not brick saves.
    let _guards = locks
        .iter()
        .map(|lock| lock.lock().unwrap_or_else(|e| e.into_inner()))
        .collect::<Vec<_>>();
    Ok(operation())
}
