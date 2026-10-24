//! Process-local ownership of monitors that can still write a runner job.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn jobs() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static JOBS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) struct DurableJobMonitorGuard(PathBuf);
impl DurableJobMonitorGuard {
    pub(crate) fn register(path: &Path) -> Self {
        let path = path.to_path_buf();
        *jobs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(path.clone())
            .or_default() += 1;
        Self(path)
    }
}
impl Drop for DurableJobMonitorGuard {
    fn drop(&mut self) {
        let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = jobs.get_mut(&self.0) {
            *count -= 1;
            if *count == 0 {
                jobs.remove(&self.0);
            }
        }
    }
}
pub(crate) fn active(path: &Path) -> bool {
    jobs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(path)
}
