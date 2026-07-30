use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert!(extra.get("error_kind").is_none());
    assert_eq!(extra["message_key"], "skill.process_basic.execution_failed");
    assert_eq!(extra["retryable"], false);
}

#[test]
fn kill_signal_contract_is_finite_and_normalized() {
    assert_eq!(normalize_signal(None).unwrap(), "TERM");
    assert_eq!(normalize_signal(Some("sigint")).unwrap(), "INT");
    assert!(normalize_signal(Some("USR1")).is_err());
    assert!(validate_kill_target(1).is_err());
    assert!(validate_kill_target(i64::from(std::process::id())).is_err());
}

#[test]
fn tail_log_honors_runner_admin_path_authority() {
    let workspace = tempfile::tempdir().expect("workspace");
    let external = tempfile::NamedTempFile::new().expect("external log");
    std::fs::write(external.path(), "line one\nline two\n").expect("write external log");

    let denied = execute_with_root_and_context(
        json!({"action":"tail_log","path":external.path(),"n":1}),
        workspace.path(),
        None,
    );
    assert!(denied.unwrap_err().contains("path_outside_workspace"));

    let context = json!({
        "authority_scope": "unrestricted_admin",
        "permissions": {
            "unrestricted_admin": true,
            "allow_path_outside_workspace": true
        }
    });
    let (text, extra) = execute_with_root_and_context(
        json!({"action":"tail_log","path":external.path(),"n":1}),
        workspace.path(),
        Some(&context),
    )
    .expect("admin external tail");
    assert_eq!(text, "line two");
    assert_eq!(extra["authority_scope"], "unrestricted_admin");
}

#[test]
fn ps_filter_matches_command_case_insensitively() {
    let row = PsRow {
        pid: 42,
        ppid: 1,
        cpu: 0.0,
        mem: 0.0,
        comm: "clawd".to_string(),
    };

    assert!(ps_row_matches_filter(&row, Some("CLAWD")));
    assert!(!ps_row_matches_filter(&row, Some("telegramd")));
}

#[test]
fn pgrep_row_parser_preserves_full_command() {
    let row = parse_pgrep_row(
        "33030 /Users/xuhao/agent-runtime/target/release/clawd --config local.toml",
    )
    .expect("pgrep row");

    assert_eq!(row.pid, 33030);
    assert_eq!(row.ppid, 0);
    assert_eq!(
        row.comm,
        "/Users/xuhao/agent-runtime/target/release/clawd --config local.toml"
    );
}

#[test]
fn command_output_filter_keeps_exit_and_matching_rows() {
    let text =
        "exit=0\nLISTEN 0 128 0.0.0.0:8788 users:((\"webd\",pid=1))\nLISTEN 0 128 0.0.0.0:5432";

    let filtered = filter_command_output(text, Some("8788"));

    assert!(filtered.starts_with("exit=0"));
    assert!(filtered.contains("8788"));
    assert!(!filtered.contains("5432"));
}

#[test]
fn ps_extra_includes_structured_running_status() {
    let page = ProcessPage {
        text: "exit=0\nPID PPID %CPU %MEM COMM\n".to_string(),
        match_count: 0,
        cursor: 0,
        returned_count: 0,
        snapshot_sha256: "fixture".to_string(),
        continuation: None,
    };
    let extra = ps_extra(20, Some("telegramd".to_string()), &page);

    assert_eq!(extra.get("action").and_then(Value::as_str), Some("ps"));
    assert_eq!(
        extra.get("filter").and_then(Value::as_str),
        Some("telegramd")
    );
    assert_eq!(extra.get("match_count").and_then(Value::as_u64), Some(0));
    assert_eq!(extra.get("process_count").and_then(Value::as_u64), Some(0));
    assert_eq!(extra.get("running").and_then(Value::as_bool), Some(false));
    assert_eq!(
        extra.get("status").and_then(Value::as_str),
        Some("not_running")
    );
}

#[test]
fn process_pages_continue_without_silently_dropping_rows() {
    let lines = (1..=7)
        .map(|pid| format!("{pid} 1 0.0 0.0 process-{pid}"))
        .collect::<Vec<_>>();
    let first = process_page("PID PPID %CPU %MEM COMM", lines.clone(), 7, 3, None, None)
        .expect("first page");
    assert_eq!(first.returned_count, 3);
    let continuation = first.continuation.as_deref().expect("continuation");
    let second = process_page(
        "PID PPID %CPU %MEM COMM",
        lines.clone(),
        7,
        3,
        None,
        Some(continuation),
    )
    .expect("second page");
    assert_eq!(second.cursor, 3);
    assert!(second.text.contains("process-4"));
    assert!(!second.text.contains("process-1"));

    let mut changed = lines;
    changed.push("8 1 0.0 0.0 changed".to_string());
    let error = process_page(
        "PID PPID %CPU %MEM COMM",
        changed,
        8,
        3,
        None,
        Some(continuation),
    )
    .expect_err("changed snapshot is stale");
    assert_eq!(error, "stale_snapshot");
}

#[test]
fn ss_listener_parser_extracts_scope_port_and_process() {
    let line = "LISTEN 0 4096 0.0.0.0:8788 0.0.0.0:* users:((\"webd\",pid=4097222,fd=31))";

    let listener = parse_ss_listener_line(line).expect("ss listener row");

    assert_eq!(listener.local_address, "0.0.0.0");
    assert_eq!(listener.port, "8788");
    assert_eq!(listener.bind_scope, "all_interfaces");
    assert!(listener.is_wildcard);
    assert_eq!(listener.process_name.as_deref(), Some("webd"));
    assert_eq!(listener.pid, Some(4097222));
}

#[test]
fn port_list_extra_keeps_all_interface_ports_as_structured_evidence() {
    let text = concat!(
        "exit=0\n",
        "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\n",
        "LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n",
        "LISTEN 0 128 127.0.0.1:46225 0.0.0.0:* users:((\"cursorsandbox\",pid=10,fd=12))\n",
        "LISTEN 0 4096 0.0.0.0:8788 0.0.0.0:* users:((\"webd\",pid=20,fd=31))\n",
        "LISTEN 0 4096 0.0.0.0:22 0.0.0.0:*\n",
        "LISTEN 0 511 [::]:80 [::]:*\n"
    );

    let extra = port_list_extra("ss", text, None);
    let all_interface_ports = extra
        .get("all_interface_ports")
        .and_then(Value::as_array)
        .expect("all-interface ports");
    let listeners = extra
        .get("listeners")
        .and_then(Value::as_array)
        .expect("listeners");

    assert_eq!(extra.get("listener_count").and_then(Value::as_u64), Some(5));
    assert_eq!(
        extra
            .get("all_interface_listener_count")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        extra.get("internet_reachability").and_then(Value::as_str),
        Some("not_observed")
    );
    assert!(all_interface_ports
        .iter()
        .any(|port| port.as_str() == Some("22")));
    assert!(all_interface_ports
        .iter()
        .any(|port| port.as_str() == Some("80")));
    assert!(all_interface_ports
        .iter()
        .any(|port| port.as_str() == Some("8788")));
    assert_eq!(
        listeners[0].get("bind_scope").and_then(Value::as_str),
        Some("all_interfaces")
    );
}
