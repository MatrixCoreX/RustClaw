use std::time::Duration;

use r2d2::Builder;
use r2d2_sqlite::SqliteConnectionManager;

const LOW_MEMORY_MAX_MIB: u64 = 2048;

fn low_memory_host(memory_mib: Option<u64>) -> bool {
    memory_mib.is_some_and(|value| value > 0 && value <= LOW_MEMORY_MAX_MIB)
}

pub(crate) fn sqlite_pool_builder(max_size: u32) -> Builder<SqliteConnectionManager> {
    sqlite_pool_builder_for_host(max_size, crate::resource_scheduler::total_memory_mib())
}

fn sqlite_pool_builder_for_host(
    max_size: u32,
    memory_mib: Option<u64>,
) -> Builder<SqliteConnectionManager> {
    let builder = r2d2::Pool::builder().max_size(max_size.max(2));
    if low_memory_host(memory_mib) {
        // Keep the configured burst capacity without permanently retaining its page caches.
        builder
            .min_idle(Some(0))
            .idle_timeout(Some(Duration::from_secs(60)))
    } else {
        builder
    }
}

#[derive(Debug, Default)]
pub(crate) struct AllocatorTuning {
    pub(crate) attempted: bool,
    pub(crate) applied: bool,
}

/// Must be called from main before creating any runtime or background threads.
pub(crate) unsafe fn configure_allocator_at_startup() -> AllocatorTuning {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let overridden = [
            "MALLOC_ARENA_MAX",
            "MALLOC_ARENA_TEST",
            "MALLOC_MMAP_THRESHOLD_",
            "MALLOC_TRIM_THRESHOLD_",
            "MALLOC_TOP_PAD_",
            "MALLOC_MMAP_MAX_",
        ]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
            || std::env::var("GLIBC_TUNABLES").is_ok_and(|value| {
                value
                    .split(':')
                    .any(|part| part.starts_with("glibc.malloc."))
            });
        if !allocator_tuning_enabled(crate::resource_scheduler::total_memory_mib(), overridden) {
            return AllocatorTuning::default();
        }
        // Large transient registry/JSON buffers should return to the OS on free.
        // mallopt changes process-global settings and is only used before threads start.
        let arena = unsafe { libc::mallopt(libc::M_ARENA_MAX, 2) };
        let mmap = unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, 1024 * 1024) };
        let trim = unsafe { libc::mallopt(libc::M_TRIM_THRESHOLD, 1024 * 1024) };
        AllocatorTuning {
            attempted: true,
            applied: arena != 0 && mmap != 0 && trim != 0,
        }
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    AllocatorTuning::default()
}

#[cfg(any(test, all(target_os = "linux", target_env = "gnu")))]
fn allocator_tuning_enabled(memory_mib: Option<u64>, overridden: bool) -> bool {
    low_memory_host(memory_mib) && !overridden
}

pub(crate) fn spawn_allocator_reclaimer(tuning: &AllocatorTuning) {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if tuning.applied {
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                // Trimming can scan arenas; keep it off the asynchronous reactor threads.
                match tokio::task::spawn_blocking(reclaim_free_pages).await {
                    Ok(released) => tracing::debug!(released, "runtime_allocator_reclaim"),
                    Err(error) => tracing::warn!(%error, "runtime_allocator_reclaim_failed"),
                }
            }
        });
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    let _ = tuning;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn reclaim_free_pages() -> bool {
    // Unlike mallopt, malloc_trim is thread-safe; it releases only allocator-free pages,
    // including holes between live allocations, and never invalidates application data.
    unsafe { libc::malloc_trim(0) != 0 }
}

#[cfg(test)]
#[path = "runtime_memory_tests.rs"]
mod tests;
