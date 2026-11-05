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
    match path.canonicalize() {
        Ok(resolved) => return Ok(resolved),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    let mut ancestor = path;
    let mut tail = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                if !resolved.is_dir() {
                    return Err(io::Error::other("configuration_parent_not_directory"));
                }
                for component in tail.iter().rev() {
                    match component {
                        Component::ParentDir => {
                            if !resolved.pop() {
                                return Err(io::Error::other("configuration_path_invalid"));
                            }
                        }
                        Component::Normal(part) => resolved.push(part),
                        _ => return Err(io::Error::other("configuration_path_invalid")),
                    }
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Resolve the existing prefix before collapsing missing tail
                // components. Lexically removing `..` first changes the
                // meaning of `directory-symlink/../missing-file`.
                tail.push(match ancestor.components().next_back() {
                    Some(component @ (Component::Normal(_) | Component::ParentDir)) => component,
                    _ => return Err(error),
                });
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("configuration_path_parent_missing"))?;
            }
            Err(error) => return Err(error),
        }
    }
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
