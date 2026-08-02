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
