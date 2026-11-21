//! Small bounded singleflight table, shared preview capacity and cancellation cleanup.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, SemaphorePermit};
type Flights = HashMap<PathBuf, Weak<AsyncMutex<()>>>;
pub(super) fn key_lock(path: &Path) -> Option<Arc<AsyncMutex<()>>> {
    static FLIGHTS: OnceLock<Mutex<Flights>> = OnceLock::new();
    let mut flights = FLIGHTS.get_or_init(Default::default).lock().ok()?;
    flights.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = flights.get(path).and_then(Weak::upgrade) {
        return Some(lock);
    }
    if flights.len() >= 64 {
        return None;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    flights.insert(path.into(), Arc::downgrade(&lock));
    Some(lock)
}
pub(super) struct PreviewLease {
    _gate: SemaphorePermit<'static>,
    _host: crate::resource_scheduler::ResourceLease,
}
pub(super) fn admit(
    state: &crate::AppState,
    path: &Path,
    reserved_bytes: u64,
    memory_mb: u64,
) -> Result<PreviewLease, ()> {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    let permit = GATE
        .get_or_init(|| Semaphore::new(1))
        .try_acquire()
        .map_err(|_| ())?;
    let parent = path.parent().ok_or(())?;
    if fs2::available_space(parent).unwrap_or(0) < 2 * 1024 * 1024 * 1024 + reserved_bytes {
        return Err(());
    }
    // Cache lifetime is independent of the original artifact. Only derived, old entries are eligible.
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten().take(128) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.starts_with(".video-browser-v1-") || name.starts_with(".video-poster-v1-"))
                && entry.metadata().is_ok_and(|meta| {
                    meta.is_file()
                        && meta
                            .modified()
                            .ok()
                            .and_then(|time| time.elapsed().ok())
                            .is_some_and(|age| age.as_secs() > 86400)
                })
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let request = claw_core::skill_registry::SkillResourceRequest {
        class: claw_core::skill_registry::SkillResourceClass::Cpu,
        cpu_cores: 1,
        memory_mb,
        disk_io_weight: 1,
        ..Default::default()
    };
    let resource = state
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .try_acquire(
            Some(&request),
            state.skill_rt.skill_global_max_concurrency.max(1),
        )
        .map_err(|_| ())?;
    Ok(PreviewLease {
        _gate: permit,
        _host: resource,
    })
}
pub(super) struct TempFile(pub PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[cfg(test)]
#[path = "task_media_guard_tests.rs"]
mod tests;

pub(super) async fn read_bounded_cache(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let metadata = tokio::fs::symlink_metadata(path).await.ok()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return None;
    }
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}
