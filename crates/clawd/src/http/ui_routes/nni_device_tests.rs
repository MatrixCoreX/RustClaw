use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

#[test]
fn nni_hardware_detection_uses_a_twelve_second_retry_window() {
    assert_eq!(NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS, 12);
    assert_eq!(
        nni_detection_retry_delay(Duration::from_secs(5)),
        Duration::from_secs(1)
    );
    assert_eq!(
        nni_detection_retry_delay(Duration::from_millis(250)),
        Duration::from_millis(250)
    );
}

#[test]
fn nni_agent_sends_heartbeats_every_eight_minutes() {
    assert_eq!(NNI_HEARTBEAT_INTERVAL_SECONDS, 8 * 60);
}

#[test]
fn nni_next_heartbeat_due_time_is_stable_for_success_and_retry_states() {
    let active = NniHeartbeatRuntimeState {
        last_heartbeat_at_ts: Some(1_000),
        last_heartbeat_attempt_at_ts: Some(1_000),
        ..NniHeartbeatRuntimeState::default()
    };
    assert_eq!(
        nni_next_heartbeat_due_at_ts(true, "active", &active, 1_200),
        Some(1_000 + NNI_HEARTBEAT_INTERVAL_SECONDS)
    );

    let waiting = NniHeartbeatRuntimeState {
        last_heartbeat_attempt_at_ts: Some(2_000),
        ..NniHeartbeatRuntimeState::default()
    };
    assert_eq!(
        nni_next_heartbeat_due_at_ts(true, "waiting_network", &waiting, 9_999),
        Some(2_000 + NNI_HEARTBEAT_POLL_SECONDS)
    );
    assert_eq!(
        nni_next_heartbeat_due_at_ts(false, "disabled", &waiting, 9_999),
        None
    );
}

#[test]
fn nni_remote_error_compatibility_never_treats_prose_as_a_machine_token() {
    let prose = ApiResponse::<Value> {
        ok: false,
        data: None,
        error: Some("localized provider detail".to_string()),
    };
    assert_eq!(
        nni_remote_api_error_code(&prose, "nni_remote_request_failed"),
        "nni_remote_request_failed"
    );

    let legacy_machine_token = ApiResponse::<Value> {
        ok: false,
        data: None,
        error: Some("nni_pubkey_not_allowlisted".to_string()),
    };
    assert_eq!(
        nni_remote_api_error_code(&legacy_machine_token, "nni_remote_request_failed"),
        "nni_pubkey_not_allowlisted"
    );

    let canonical = ApiResponse::<Value> {
        ok: false,
        data: Some(json!({"error_code": "nni_structured_rejection"})),
        error: Some("ignored detail".to_string()),
    };
    assert_eq!(
        nni_remote_api_error_code(&canonical, "nni_remote_request_failed"),
        "nni_structured_rejection"
    );
}

#[test]
fn nni_simulation_controls_are_accepted_but_not_advertised_as_chip_operations() {
    assert!(nni_accepted_actions().contains(&NNI_SIMULATION_ENABLE_ACTION));
    assert!(nni_accepted_actions().contains(&NNI_SIMULATION_DISABLE_ACTION));
    assert!(!nni_supported_actions().contains(&NNI_SIMULATION_ENABLE_ACTION));
    assert!(!nni_supported_actions().contains(&NNI_SIMULATION_DISABLE_ACTION));
}

#[test]
fn nni_simulator_state_is_scoped_to_the_workspace_data_directory() {
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = PathBuf::from("workspace-fixture");
    assert_eq!(
        nni_signature_simulator_state_path(&state),
        PathBuf::from("workspace-fixture/data/nni/signature-simulator.json")
    );
}

#[test]
fn nni_helper_metadata_preserves_simulation_identity() {
    let metadata = nni_helper_payload_meta(&json!({
        "slot": 0,
        "i2c_address": "virtual",
        "simulated": true,
        "device_kind": "simulated",
    }));
    assert_eq!(metadata["simulated"], Value::Bool(true));
    assert_eq!(
        metadata["device_kind"],
        Value::String("simulated".to_string())
    );
    assert_eq!(
        metadata["i2c_address"],
        Value::String("virtual".to_string())
    );
}

#[test]
fn nni_simulation_actions_have_stable_message_keys() {
    assert_eq!(
        nni_action_message_key(NNI_SIMULATION_ENABLE_ACTION),
        "nni.device_action.simulation_enabled"
    );
    assert_eq!(
        nni_action_message_key(NNI_SIMULATION_DISABLE_ACTION),
        "nni.device_action.simulation_disabled"
    );
    assert_eq!(
        nni_action_message_key("sign_timestamp"),
        "nni.device_action.completed"
    );
}

#[tokio::test]
async fn nni_reward_ledger_requires_ui_authentication() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/v1/nni/rewards?page=1&per_page=100")
                .body(Body::empty())
                .expect("reward ledger request"),
        )
        .await
        .expect("reward ledger response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nni_bancor_routes_require_ui_authentication() {
    for (method, uri, body) in [
        ("GET", "/v1/nni/bancor/market", ""),
        (
            "GET",
            "/v1/nni/bancor/candles?interval_seconds=3600&limit=120",
            "",
        ),
        ("GET", "/v1/nni/bancor/trades", ""),
        ("GET", "/v1/nni/bancor/account?page=1&per_page=20", ""),
        (
            "POST",
            "/v1/nni/bancor/quote",
            r#"{"side":"sell","input_amount":"1.0000"}"#,
        ),
        (
            "POST",
            "/v1/nni/bancor/trade",
            r#"{"side":"sell","input_amount":"1.0000","min_output":"0.0001"}"#,
        ),
    ] {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let mut builder = Request::builder().method(method).uri(uri);
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        let response = axum::Router::new()
            .nest("/v1", build_ui_router())
            .with_state(state)
            .oneshot(builder.body(Body::from(body)).expect("Bancor request"))
            .await
            .expect("Bancor response");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri}"
        );
    }
}

#[test]
fn nni_bancor_market_trades_are_sanitized_and_limited_to_the_latest_hundred() {
    let pubkey = concat!(
        "2b9c9d84fa15f4e178ce58d0a40a9f5e150e9c502e689a24d0c0f221337870c",
        "726f0e463d730a75401c425bfde0db0c442e314027d83885a84c535eaa35460a0"
    );
    assert_eq!(
        compact_bancor_device_pubkey(pubkey).as_deref(),
        Some("ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C"),
    );
    assert_eq!(
        compact_bancor_device_pubkey("AiucnYT6FfTheM5Y0KQKn14VDpxQLmiaJNDA8iEzeHDH").as_deref(),
        Some("ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C"),
    );

    let mut trades = vec![
        json!({"trade_id": "raw", "device_pubkey": pubkey}),
        json!({"trade_id": "mislabelled", "device_pubkey_masked": pubkey}),
        json!({"trade_id": "masked", "device_pubkey_masked": "a2c887498554••••••••331016eb"}),
    ];
    trades.extend((3..105).map(|index| {
        json!({
            "trade_id": format!("trade-{index}"),
            "device_pubkey_compact": "ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C",
        })
    }));
    let mut payload = json!({
        "page": 1,
        "per_page": 105,
        "total": 500,
        "total_pages": 5,
        "trades": trades,
    });
    normalize_bancor_market_trades(&mut payload);
    let serialized = payload.to_string();
    assert!(!serialized.contains(pubkey));
    assert_eq!(payload["limit"], Value::Number(100.into()));
    assert_eq!(payload["trades"].as_array().map(Vec::len), Some(100));
    for removed in ["page", "per_page", "total", "total_pages"] {
        assert!(
            payload.get(removed).is_none(),
            "{removed} must not expose pagination"
        );
    }
    assert_eq!(
        payload["trades"][0]["device_pubkey_compact"],
        Value::String("ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C".to_string()),
    );
    assert_eq!(
        payload["trades"][1]["device_pubkey_compact"],
        Value::String("ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C".to_string()),
    );
    assert_eq!(
        payload["trades"][2]["device_pubkey_compact"],
        Value::String("a2c887498554••••••••331016eb".to_string()),
    );
    assert!(payload["trades"]
        .as_array()
        .expect("market trades")
        .iter()
        .all(|trade| trade.get("device_pubkey").is_none()
            && trade.get("device_pubkey_masked").is_none()));
}

struct NniRuntimeStateTestWorkspace {
    root: PathBuf,
}

impl NniRuntimeStateTestWorkspace {
    fn new(label: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-nni-{label}-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("configs")).expect("create NNI test workspace");
        Self { root }
    }

    fn state(&self) -> AppState {
        let mut state = AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = self.root.clone();
        state
    }

    fn write_config(&self, raw: &str) {
        std::fs::write(self.root.join("configs/config.toml"), raw).expect("write NNI test config");
    }

    fn read_config(&self) -> String {
        std::fs::read_to_string(self.root.join("configs/config.toml"))
            .expect("read NNI test config")
    }
}

impl Drop for NniRuntimeStateTestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn nni_heartbeat_status_uses_data_root_without_mutating_config() {
    let workspace = NniRuntimeStateTestWorkspace::new("heartbeat-state");
    let config = concat!(
        "[nni]\n",
        "remote_nodes = [\"https://nni.example.test\"]\n",
        "joined = true\n",
        "last_heartbeat_at_ts = 111\n",
        "heartbeat_request_count = 9\n",
        "last_heartbeat_error = \"legacy error\"\n",
    );
    workspace.write_config(config);
    let state = workspace.state();

    let initial = read_nni_config(&state).expect("read initial NNI config");
    assert_eq!(initial.remote_nodes, vec!["https://nni.example.test"]);
    assert_eq!(
        initial.selected_node_url.as_deref(),
        Some("https://nni.example.test")
    );
    assert!(initial.joined);
    assert_eq!(
        initial.config_path,
        workspace
            .root
            .join("data/nni/runtime-config.json")
            .display()
            .to_string()
    );
    assert_eq!(initial.last_heartbeat_at_ts, None);
    assert_eq!(initial.heartbeat_request_count, 0);
    assert_eq!(initial.last_heartbeat_error, None);

    let response = write_nni_heartbeat_status(
        &state,
        NniHeartbeatStatusUpdate {
            heartbeat_at_ts: Some(222),
            attempt_at_ts: Some(222),
            error: None,
            error_code: None,
            error_at_ts: None,
            error_network: false,
            request_count: Some(10),
            network_failures: Some(0),
            success_node_url: Some("https://nni.example.test"),
            network_authorization: Some("authorized"),
        },
    )
    .expect("persist NNI heartbeat runtime state");

    assert_eq!(workspace.read_config(), config);
    assert_eq!(response.last_heartbeat_at_ts, Some(222));
    assert_eq!(response.heartbeat_request_count, 10);
    assert_eq!(response.last_heartbeat_network_failures, 0);
    let runtime_state_path = nni_heartbeat_runtime_state_path(&state);
    assert_eq!(
        runtime_state_path,
        workspace.root.join("data/nni/heartbeat-state.json")
    );
    let persisted: NniHeartbeatRuntimeState = serde_json::from_str(
        &std::fs::read_to_string(runtime_state_path).expect("read NNI heartbeat state"),
    )
    .expect("parse NNI heartbeat state");
    assert_eq!(persisted.schema_version, 2);
    assert_eq!(persisted.last_heartbeat_at_ts, Some(222));
    assert_eq!(persisted.heartbeat_request_count, 10);
    let migrated: NniRuntimeConfig = serde_json::from_str(
        &std::fs::read_to_string(nni_runtime_config_path(&state))
            .expect("read migrated NNI runtime config"),
    )
    .expect("parse migrated NNI runtime config");
    assert_eq!(migrated.schema_version, 1);
    assert_eq!(migrated.remote_nodes, vec!["https://nni.example.test"]);
    assert!(migrated.joined);
}

#[test]
fn nni_settings_use_data_root_without_mutating_main_config() {
    let workspace = NniRuntimeStateTestWorkspace::new("runtime-config");
    let main_config = "[llm]\nselected_vendor = \"minimax\"\n";
    workspace.write_config(main_config);
    let state = workspace.state();
    let nodes = vec!["https://nni.example.test/v1".to_string()];

    let response =
        write_nni_config(&state, Some(&nodes), Some(true)).expect("persist NNI runtime config");

    assert_eq!(workspace.read_config(), main_config);
    assert_eq!(response.remote_nodes, vec!["https://nni.example.test"]);
    assert_eq!(
        response.selected_node_url.as_deref(),
        Some("https://nni.example.test")
    );
    assert!(response.joined);
    assert_eq!(
        response.config_path,
        workspace
            .root
            .join("data/nni/runtime-config.json")
            .display()
            .to_string()
    );
    let persisted: NniRuntimeConfig = serde_json::from_str(
        &std::fs::read_to_string(nni_runtime_config_path(&state)).expect("read NNI runtime config"),
    )
    .expect("parse NNI runtime config");
    assert_eq!(persisted.remote_nodes, vec!["https://nni.example.test"]);
    assert_eq!(
        persisted.selected_node_url.as_deref(),
        Some("https://nni.example.test")
    );
    assert!(persisted.joined);
}

#[test]
fn nni_settings_keep_one_explicit_active_node() {
    let workspace = NniRuntimeStateTestWorkspace::new("selected-node");
    workspace.write_config("[llm]\nselected_vendor = \"minimax\"\n");
    let state = workspace.state();
    let nodes = vec![
        "https://node-a.example.test".to_string(),
        "https://node-b.example.test/v1".to_string(),
    ];

    let response = write_nni_config_with_selected_node(
        &state,
        Some(&nodes),
        Some("https://node-b.example.test"),
        Some(true),
    )
    .expect("persist selected NNI node");

    assert_eq!(response.remote_nodes.len(), 2);
    assert_eq!(
        response.selected_node_url.as_deref(),
        Some("https://node-b.example.test")
    );
    assert_eq!(
        nni_selected_remote_node(&response).map(String::as_str),
        Some("https://node-b.example.test")
    );

    let switch_error = write_nni_config_with_selected_node(
        &state,
        None,
        Some("https://node-a.example.test"),
        None,
    )
    .expect_err("active NNI must stop before switching nodes");
    assert_eq!(
        switch_error.to_string(),
        "nni_selected_node_change_requires_stop"
    );

    let stopped = write_nni_config_with_selected_node(
        &state,
        None,
        Some("https://node-a.example.test"),
        Some(false),
    )
    .expect("stop NNI and select another bound node");
    assert!(!stopped.joined);
    assert_eq!(
        stopped.selected_node_url.as_deref(),
        Some("https://node-a.example.test")
    );
}

#[test]
fn nni_history_clear_operations_do_not_mutate_config() {
    let workspace = NniRuntimeStateTestWorkspace::new("history-clear");
    let config = concat!(
        "[nni]\n",
        "remote_nodes = [\"https://nni.example.test\"]\n",
        "joined = true\n",
    );
    workspace.write_config(config);
    let state = workspace.state();

    let mut request_record = nni_request_record("nni_heartbeat", "accepted");
    request_record.created_at_ts = Some(333);
    write_nni_request_record(&state, request_record).expect("write NNI request record");
    clear_nni_request_records(&state).expect("clear NNI request records");
    assert_eq!(workspace.read_config(), config);
    assert!(read_nni_request_records(&state)
        .expect("read cleared NNI request records")
        .is_empty());

    write_nni_heartbeat_status(
        &state,
        NniHeartbeatStatusUpdate {
            heartbeat_at_ts: Some(444),
            attempt_at_ts: Some(444),
            error: Some("network unavailable"),
            error_code: Some("heartbeat_request_network_failed"),
            error_at_ts: Some(444),
            error_network: true,
            request_count: Some(11),
            network_failures: Some(3),
            success_node_url: None,
            network_authorization: None,
        },
    )
    .expect("write NNI heartbeat error state");
    assert_eq!(
        read_nni_heartbeat_error_records(&state)
            .expect("read NNI heartbeat errors")
            .len(),
        1
    );
    clear_nni_heartbeat_error_records(&state).expect("clear NNI heartbeat errors");
    assert_eq!(workspace.read_config(), config);
    assert!(read_nni_heartbeat_error_records(&state)
        .expect("read cleared NNI heartbeat errors")
        .is_empty());
    let runtime_state =
        read_nni_heartbeat_runtime_state(&state).expect("read cleared NNI heartbeat state");
    assert_eq!(runtime_state.heartbeat_request_count, 11);
    assert_eq!(runtime_state.last_heartbeat_at_ts, Some(444));
    assert_eq!(runtime_state.last_heartbeat_error, None);
    assert_eq!(runtime_state.last_heartbeat_error_code, None);
    assert_eq!(runtime_state.last_heartbeat_error_at_ts, None);
    assert_eq!(runtime_state.last_heartbeat_network_failures, 0);
}
