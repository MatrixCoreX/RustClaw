//! Serialize retirement against in-flight host producers of task-owned files.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

type Key = (PathBuf, String);
fn gate(state: &crate::AppState, task_id: &str) -> Arc<RwLock<()>> {
    static GATES: OnceLock<Mutex<HashMap<Key, Weak<RwLock<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if gates.len() > 4096 {
        gates.retain(|_, lock| lock.strong_count() > 0);
    }
    let key = (state.skill_rt.workspace_root.clone(), task_id.to_string());
    if let Some(lock) = gates.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(RwLock::new(()));
    gates.insert(key, Arc::downgrade(&lock));
    lock
}

#[derive(Debug)]
pub(crate) struct RetiredResult;
impl std::fmt::Display for RetiredResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("work_result_retired")
    }
}
impl std::error::Error for RetiredResult {}

pub(crate) async fn begin_writing(
    state: &crate::AppState,
    task_id: &str,
) -> anyhow::Result<Option<OwnedRwLockReadGuard<()>>> {
    if super::read_enrollment(&state.skill_rt.workspace_root, task_id)
        .ok()
        .flatten()
        .is_none()
    {
        return Ok(None);
    }
    let guard = gate(state, task_id).read_owned().await;
    if super::is_retired(state, &*state.core.db.get()?, task_id)? {
        return Err(RetiredResult.into());
    }
    Ok(Some(guard))
}

pub(crate) fn begin_retiring(
    state: &crate::AppState,
    task_id: &str,
) -> anyhow::Result<OwnedRwLockWriteGuard<()>> {
    gate(state, task_id)
        .try_write_owned()
        .map_err(|_| anyhow::anyhow!("work_cleanup_producer_active"))
}

pub(crate) fn try_begin_writing(
    state: &crate::AppState,
    task_id: &str,
) -> anyhow::Result<Option<OwnedRwLockReadGuard<()>>> {
    if super::read_enrollment(&state.skill_rt.workspace_root, task_id)
        .ok()
        .flatten()
        .is_none()
    {
        return Ok(None);
    }
    let guard = gate(state, task_id)
        .try_read_owned()
        .map_err(|_| anyhow::anyhow!("work_cleanup_in_progress"))?;
    if super::is_retired(state, &*state.core.db.get()?, task_id)? {
        return Err(RetiredResult.into());
    }
    Ok(Some(guard))
}
