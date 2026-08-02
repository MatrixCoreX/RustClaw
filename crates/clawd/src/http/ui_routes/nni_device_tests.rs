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
                .uri("/v1/nni/rewards?page=1&per_page=10")
                .body(Body::empty())
                .expect("reward ledger request"),
        )
        .await
        .expect("reward ledger response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
    assert_eq!(initial.last_heartbeat_at_ts, None);
    assert_eq!(initial.heartbeat_request_count, 0);
    assert_eq!(initial.last_heartbeat_error, None);

    let response =
        write_nni_heartbeat_status(&state, Some(222), None, None, None, Some(10), Some(0))
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
    assert_eq!(persisted.schema_version, 1);
    assert_eq!(persisted.last_heartbeat_at_ts, Some(222));
    assert_eq!(persisted.heartbeat_request_count, 10);
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
        Some(444),
        Some("network unavailable"),
        Some(444),
        Some(true),
        Some(11),
        Some(3),
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
    assert_eq!(runtime_state.last_heartbeat_error_at_ts, None);
    assert_eq!(runtime_state.last_heartbeat_network_failures, 0);
}
