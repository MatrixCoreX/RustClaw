use super::*;

#[test]
fn small_host_policy_has_explicit_boundaries_and_respects_allocator_overrides() {
    for size in [Some(1), Some(991), Some(2048)] {
        assert!(allocator_tuning_enabled(size, false));
        assert!(!allocator_tuning_enabled(size, true));
    }
    for size in [None, Some(0), Some(2049), Some(16384)] {
        assert!(!allocator_tuning_enabled(size, false));
    }
}

#[test]
fn small_host_pool_opens_on_demand_without_reducing_burst_capacity() {
    let root = std::env::temp_dir().join(format!("runtime-pool-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let pool = sqlite_pool_builder_for_host(8, Some(991))
        .build(SqliteConnectionManager::file(root.join("test.db")))
        .unwrap();
    assert_eq!(pool.max_size(), 8);
    assert_eq!(pool.min_idle(), Some(0));
    assert_eq!(pool.idle_timeout(), Some(Duration::from_secs(60)));
    assert_eq!(pool.state().connections, 0);
    let connections: Vec<_> = (0..8).map(|_| pool.get().unwrap()).collect();
    assert_eq!(pool.state().connections, 8);
    assert_eq!(pool.state().idle_connections, 0);
    connections[0]
        .execute_batch("CREATE TABLE retained (value TEXT); INSERT INTO retained VALUES ('saved');")
        .unwrap();
    drop(connections);
    assert_eq!(pool.state().idle_connections, 8);
    let value: String = pool
        .get()
        .unwrap()
        .query_row("SELECT value FROM retained", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "saved");
    drop(pool);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn other_hosts_keep_pool_defaults_and_configured_limits() {
    for memory in [None, Some(0), Some(2049), Some(8192)] {
        let pool = sqlite_pool_builder_for_host(3, memory)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        assert_eq!(pool.max_size(), 3);
        assert_eq!(pool.min_idle(), None);
        assert_eq!(pool.idle_timeout(), Some(Duration::from_secs(600)));
        assert_eq!(pool.state().connections, 3);
    }
}

#[test]
fn malformed_pool_size_keeps_existing_two_connection_floor() {
    for size in [0, 1] {
        let pool = sqlite_pool_builder_for_host(size, Some(1024))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        assert_eq!(pool.max_size(), 2);
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn reclaiming_fragmented_pages_preserves_live_buffers_and_database() {
    let mut buffers: Vec<_> = (0..128)
        .map(|index| Some(vec![index as u8; 64 * 1024]))
        .collect();
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE retained (value TEXT); INSERT INTO retained VALUES ('saved');")
        .unwrap();
    for index in (1..buffers.len()).step_by(2) {
        buffers[index] = None;
    }
    reclaim_free_pages();
    for (index, buffer) in buffers.iter().enumerate() {
        if let Some(buffer) = buffer {
            assert!(buffer.iter().all(|byte| *byte == index as u8));
        }
    }
    let value: String = db
        .query_row("SELECT value FROM retained", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "saved");
}

#[test]
fn allocator_override_does_not_start_maintenance_or_require_a_runtime() {
    spawn_allocator_reclaimer(&AllocatorTuning::default());
}
