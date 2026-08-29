use super::*;

fn config_with_separate_nodes() -> NniConfigResponse {
    NniConfigResponse {
        remote_nodes: vec![
            "https://node-a.example.test".to_string(),
            "https://node-b.example.test".to_string(),
        ],
        selected_node_url: Some("https://node-a.example.test".to_string()),
        bancor_service_node_url: Some("https://node-a.example.test".to_string()),
        asset_service_node_url: Some("https://node-b.example.test".to_string()),
        joined: true,
        asset_owner_pubkey: None,
        heartbeat_interval_seconds: 590,
        heartbeat_network_retry_limit: 3,
        heartbeat_request_count: 0,
        last_heartbeat_at_ts: None,
        last_heartbeat_error: None,
        last_heartbeat_error_code: None,
        last_heartbeat_error_at_ts: None,
        last_heartbeat_network_failures: 0,
        last_heartbeat_attempt_at_ts: None,
        consecutive_heartbeat_failures: 0,
        last_success_node_host: None,
        network_authorization: "unknown".to_string(),
        heartbeat_state: "active".to_string(),
        next_heartbeat_due_at_ts: None,
        worker_running: true,
        config_path: "/tmp/nni-runtime-config.json".to_string(),
    }
}

#[test]
fn asset_service_priority_is_independent_from_heartbeat_priority() {
    let config = config_with_separate_nodes();

    assert_eq!(
        nni_selected_remote_nodes(&config)
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "https://node-a.example.test",
            "https://node-b.example.test",
        ],
    );
    assert_eq!(
        nni_asset_service_remote_nodes(&config)
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "https://node-b.example.test",
            "https://node-a.example.test",
        ],
    );
}

#[test]
fn bancor_and_asset_priorities_can_be_selected_independently() {
    let mut config = config_with_separate_nodes();
    config.bancor_service_node_url = Some("https://node-b.example.test".to_string());
    config.asset_service_node_url = Some("https://node-a.example.test".to_string());

    assert_eq!(
        nni_bancor_service_remote_node(&config).map(String::as_str),
        Some("https://node-b.example.test"),
    );
    assert_eq!(
        nni_asset_service_remote_node(&config).map(String::as_str),
        Some("https://node-a.example.test"),
    );
    assert_eq!(
        nni_selected_remote_node(&config).map(String::as_str),
        Some("https://node-a.example.test"),
    );
}

#[test]
fn asset_service_falls_back_to_the_heartbeat_node_for_old_config() {
    let mut config = config_with_separate_nodes();
    config.asset_service_node_url = None;

    assert_eq!(
        nni_asset_service_remote_node(&config).map(String::as_str),
        Some("https://node-a.example.test"),
    );
}
