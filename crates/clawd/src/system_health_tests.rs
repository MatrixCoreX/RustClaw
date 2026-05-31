use super::{collect_matching_pids, process_basename, process_name_matches, ProcessSnapshot};

#[test]
fn process_name_matches_binary_and_cargo_run_forms() {
    let direct = ProcessSnapshot {
        pid: 1,
        rss_bytes: Some(1024),
        comm: "feishud".to_string(),
        args: "/tmp/rustclaw-workspace/target/release/feishud".to_string(),
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
        process_basename("\"/Applications/RustClaw/feishud\""),
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
            args: "/tmp/rustclaw-workspace/target/release/telegramd".to_string(),
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
            args: "/tmp/rustclaw-workspace/target/release/telegramd".to_string(),
        },
    ];

    let pids = collect_matching_pids(&processes, "telegramd", 42);
    assert_eq!(pids, vec![41, 43]);
}
