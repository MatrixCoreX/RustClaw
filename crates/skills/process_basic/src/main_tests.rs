use super::*;

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
fn command_output_filter_keeps_exit_and_matching_rows() {
    let text =
        "exit=0\nLISTEN 0 128 0.0.0.0:8787 users:((\"clawd\",pid=1))\nLISTEN 0 128 0.0.0.0:5432";

    let filtered = filter_command_output(text, Some("8787"));

    assert!(filtered.starts_with("exit=0"));
    assert!(filtered.contains("8787"));
    assert!(!filtered.contains("5432"));
}

#[test]
fn ss_listener_parser_extracts_scope_port_and_process() {
    let line = "LISTEN 0 4096 0.0.0.0:8787 0.0.0.0:* users:((\"clawd\",pid=4097222,fd=31))";

    let listener = parse_ss_listener_line(line).expect("ss listener row");

    assert_eq!(listener.local_address, "0.0.0.0");
    assert_eq!(listener.port, "8787");
    assert_eq!(listener.bind_scope, "all_interfaces");
    assert!(listener.is_wildcard);
    assert_eq!(listener.process_name.as_deref(), Some("clawd"));
    assert_eq!(listener.pid, Some(4097222));
}

#[test]
fn port_list_extra_keeps_public_ports_as_structured_evidence() {
    let text = concat!(
        "exit=0\n",
        "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\n",
        "LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n",
        "LISTEN 0 128 127.0.0.1:46225 0.0.0.0:* users:((\"cursorsandbox\",pid=10,fd=12))\n",
        "LISTEN 0 4096 0.0.0.0:8787 0.0.0.0:* users:((\"clawd\",pid=20,fd=31))\n",
        "LISTEN 0 4096 0.0.0.0:22 0.0.0.0:*\n",
        "LISTEN 0 511 [::]:80 [::]:*\n"
    );

    let extra = port_list_extra("ss", text, None);
    let public_ports = extra
        .get("public_ports")
        .and_then(Value::as_array)
        .expect("public ports");
    let listeners = extra
        .get("listeners")
        .and_then(Value::as_array)
        .expect("listeners");

    assert_eq!(extra.get("listener_count").and_then(Value::as_u64), Some(5));
    assert_eq!(
        extra.get("public_listener_count").and_then(Value::as_u64),
        Some(3)
    );
    assert!(public_ports.iter().any(|port| port.as_str() == Some("22")));
    assert!(public_ports.iter().any(|port| port.as_str() == Some("80")));
    assert!(public_ports
        .iter()
        .any(|port| port.as_str() == Some("8787")));
    assert_eq!(
        listeners[0].get("bind_scope").and_then(Value::as_str),
        Some("all_interfaces")
    );
}
