use super::{
    active_running_task_count, active_running_task_count_for_user, collect_matching_pids,
    oldest_running_task_age_seconds, oldest_running_task_age_seconds_for_user, process_basename,
    process_name_matches, DaemonProcessSnapshot, ProcessSnapshot,
};

#[test]
fn one_snapshot_preserves_all_daemon_counts_and_rss_without_rescanning() {
    let names = [
        "telegramd",
        "channel-gateway",
        "whatsappd",
        "whatsapp_webd",
        "webd",
        "wechatd",
        "feishud",
        "larkd",
    ];
    let processes = names
        .iter()
        .enumerate()
        .flat_map(|(index, name)| {
            [
                ProcessSnapshot {
                    pid: index as u32 * 2 + 1,
                    rss_bytes: Some(1024),
                    comm: name.to_string(),
                    args: format!("/opt/bin/{name}"),
                },
                ProcessSnapshot {
                    pid: index as u32 * 2 + 2,
                    rss_bytes: None,
                    comm: "cargo".into(),
                    args: format!("cargo run -p {name}"),
                },
            ]
        })
        .collect();
    let snapshot = DaemonProcessSnapshot {
        processes: Some(processes),
    };
    for _ in 0..2 {
        for name in names {
            assert_eq!(snapshot.stats(name), Some((2, 1024)));
        }
        assert_eq!(snapshot.stats("absent-service"), Some((0, 0)));
    }
}

#[test]
fn unavailable_snapshot_is_not_a_zero_process_observation() {
    let unknown = DaemonProcessSnapshot::default();
    assert_eq!(unknown.stats("webd"), None);
    assert_eq!(unknown.current_rss_bytes(), None);
    let empty = DaemonProcessSnapshot {
        processes: Some(Vec::new()),
    };
    assert_eq!(empty.stats("webd"), Some((0, 0)));
    assert_eq!(empty.current_rss_bytes(), None);
}

#[test]
fn process_snapshot_preserves_saturating_rss_and_current_pid_selection() {
    let snapshot = DaemonProcessSnapshot {
        processes: Some(vec![
            ProcessSnapshot {
                pid: std::process::id(),
                rss_bytes: Some(u64::MAX),
                comm: "webd".into(),
                args: String::new(),
            },
            ProcessSnapshot {
                pid: 0,
                rss_bytes: Some(1024),
                comm: "webd".into(),
                args: String::new(),
            },
        ]),
    };
    assert_eq!(snapshot.stats("webd"), Some((2, u64::MAX)));
    assert_eq!(snapshot.current_rss_bytes(), Some(u64::MAX));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_current_rss_reads_own_status_and_handles_missing_or_malformed_data() {
    assert!(super::current_rss_bytes().is_some_and(|rss| rss > 0));
    let root = crate::secure_workspace_fs::tests::TestWorkspace::new("rss");
    let path = root.path().join("status");
    assert_eq!(
        super::current_rss_bytes_from_status_path(path.to_str().unwrap()),
        None
    );
    std::fs::write(&path, "Name:\tfixture\nVmRSS:\t123 kB\n").unwrap();
    assert_eq!(
        super::current_rss_bytes_from_status_path(path.to_str().unwrap()),
        Some(123 * 1024)
    );
    std::fs::write(&path, "VmRSS:\tunknown kB\n").unwrap();
    assert_eq!(
        super::current_rss_bytes_from_status_path(path.to_str().unwrap()),
        None
    );
}

#[test]
fn process_name_matches_binary_and_cargo_run_forms() {
    let direct = ProcessSnapshot {
        pid: 1,
        rss_bytes: Some(1024),
        comm: "feishud".to_string(),
        args: "/tmp/agent-runtime-workspace/target/release/feishud".to_string(),
    };
    assert!(process_name_matches(&direct, "feishud"));

    let cargo = ProcessSnapshot {
        pid: 2,
        rss_bytes: Some(1024),
        comm: "cargo".to_string(),
        args: "cargo run -p feishud -- --config configs/channels/feishu.toml".to_string(),
    };
    assert!(process_name_matches(&cargo, "feishud"));
}

#[test]
fn process_basename_handles_paths_and_quotes() {
    assert_eq!(process_basename("/usr/local/bin/clawd"), "clawd");
    assert_eq!(
        process_basename("\"/Applications/Agent Runtime/feishud\""),
        "feishud"
    );
}

#[test]
fn collect_matching_pids_filters_self_and_matches_cross_platform_forms() {
    let processes = vec![
        ProcessSnapshot {
            pid: 41,
            rss_bytes: Some(1024),
            comm: "telegramd".to_string(),
            args: "/tmp/agent-runtime-workspace/target/release/telegramd".to_string(),
        },
        ProcessSnapshot {
            pid: 42,
            rss_bytes: Some(1024),
            comm: "bash".to_string(),
            args: "cargo run -p telegramd -- --config configs/channels/telegram.toml".to_string(),
        },
        ProcessSnapshot {
            pid: 43,
            rss_bytes: Some(1024),
            comm: "telegramd".to_string(),
            args: "/tmp/agent-runtime-workspace/target/release/telegramd".to_string(),
        },
    ];

    let pids = collect_matching_pids(&processes, "telegramd", 42);
    assert_eq!(pids, vec![41, 43]);
}

#[test]
fn running_task_age_supports_system_and_user_scopes() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let now = crate::now_ts_u64() as i64;
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create task health fixture");
    db.execute(
        "INSERT INTO tasks
            (task_id, user_id, status, created_at, lease_owner, lease_expires_at)
         VALUES ('owner-task', 42, 'running', ?1, 'worker:owner', ?5),
                ('other-task', 99, 'running', ?2, 'worker:other', ?5),
                ('waiting-user', 42, 'running', ?3, NULL, 0),
                ('expired-lease', 42, 'running', ?4, 'worker:expired', ?6)",
        rusqlite::params![
            (now - 20).to_string(),
            (now - 120).to_string(),
            (now - 3_600).to_string(),
            (now - 7_200).to_string(),
            now + 60,
            now - 1
        ],
    )
    .expect("insert task health fixtures");
    drop(db);

    let system_age = oldest_running_task_age_seconds(&state).expect("system age");
    let owner_age = oldest_running_task_age_seconds_for_user(&state, 42).expect("owner age");
    let system_count = active_running_task_count(&state).expect("system count");
    let owner_count = active_running_task_count_for_user(&state, 42).expect("owner count");

    assert!((119..=121).contains(&system_age));
    assert!((19..=21).contains(&owner_age));
    assert_eq!(system_count, 2);
    assert_eq!(owner_count, 1);
}
