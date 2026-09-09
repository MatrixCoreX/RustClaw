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
