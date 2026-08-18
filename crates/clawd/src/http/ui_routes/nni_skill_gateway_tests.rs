use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request};
use tower::ServiceExt;

struct NniTempDir(std::path::PathBuf);

impl NniTempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("nni-skill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create NNI test workspace");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for NniTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn internal_nni_token(skill_name: &str) -> String {
    claw_core::secrets::issue_secret_token_value(
        &claw_core::secrets::SecretValue::new(
            json!({
                "task_id": "internal-nni-test",
                "user_id": 1,
                "chat_id": 2,
                "user_key": "test-user-key",
                "channel": "ui",
                "external_user_id": null,
                "external_chat_id": null,
                "kind": "run_skill",
                "payload_json": "{}",
                "skill_name": skill_name,
            })
            .to_string(),
        ),
        Duration::from_secs(60),
    )
    .expect("issue internal NNI token")
}

fn internal_nni_context() -> InternalSkillTokenContext {
    InternalSkillTokenContext {
        task_id: "internal-nni-test".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-user-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: "{}".to_string(),
        skill_name: "nni".to_string(),
    }
}

fn internal_nni_request(action: InternalNniAction) -> InternalNniActionRequest {
    InternalNniActionRequest {
        action,
        limit: None,
        interval: None,
        end_time_ts: None,
        side: None,
        pay_asset: None,
        pay_amount: None,
        slippage_bps: None,
    }
}

fn isolated_nni_state(root: &std::path::Path) -> AppState {
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = root.to_path_buf();
    state
}

async fn call_internal_nni(token: &str, action: &str) -> (StatusCode, Value) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/internal/nni/action")
                .header("content-type", "application/json")
                .header(
                    claw_core::product_identity::INTERNAL_SKILL_TOKEN_HEADER,
                    token,
                )
                .body(Body::from(json!({"action": action}).to_string()))
                .expect("internal NNI request"),
        )
        .await
        .expect("internal NNI response");
    let status = response.status();
    let payload = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read internal NNI response"),
    )
    .expect("parse internal NNI response");
    (status, payload)
}

#[tokio::test]
async fn internal_nni_gateway_is_skill_scoped_and_one_time() {
    let token = internal_nni_token("nni");
    let (status, payload) = call_internal_nni(&token, "heartbeat_status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["source_skill"], "nni");
    assert_eq!(payload["data"]["action"], "heartbeat_status");

    let (replay_status, replay_payload) = call_internal_nni(&token, "heartbeat_status").await;
    assert_eq!(replay_status, StatusCode::UNAUTHORIZED);
    assert_eq!(replay_payload["ok"], false);

    let wrong_skill_token = internal_nni_token("extension_manager");
    let (wrong_status, wrong_payload) =
        call_internal_nni(&wrong_skill_token, "heartbeat_status").await;
    assert_eq!(wrong_status, StatusCode::FORBIDDEN);
    assert_eq!(
        wrong_payload["data"]["error_code"],
        "nni_internal_gateway_unauthorized"
    );
}

#[tokio::test]
async fn heartbeat_enable_without_a_signer_fails_without_changing_intent() {
    let root = NniTempDir::new();
    let state = isolated_nni_state(root.path());
    assert!(!read_nni_config(&state).expect("initial NNI state").joined);

    let error = execute_internal_nni_action(
        &state,
        &internal_nni_context(),
        &internal_nni_request(InternalNniAction::HeartbeatEnable),
    )
    .await
    .expect_err("missing signer must block heartbeat enable");

    assert_eq!(error.error_code, "nni_signature_helper_unavailable");
    assert!(!read_nni_config(&state).expect("final NNI state").joined);
}

#[tokio::test]
async fn heartbeat_disable_is_idempotent_and_preserves_history_owner_state() {
    let root = NniTempDir::new();
    let state = isolated_nni_state(root.path());
    write_nni_config(&state, None, Some(true)).expect("enable heartbeat intent fixture");

    let first = execute_internal_nni_action(
        &state,
        &internal_nni_context(),
        &internal_nni_request(InternalNniAction::HeartbeatDisable),
    )
    .await
    .expect("first disable");
    let second = execute_internal_nni_action(
        &state,
        &internal_nni_context(),
        &internal_nni_request(InternalNniAction::HeartbeatDisable),
    )
    .await
    .expect("second disable");

    assert_eq!(first["changed"], true);
    assert_eq!(second["changed"], false);
    assert_eq!(second["heartbeat"]["desired_enabled"], false);
}

#[tokio::test]
async fn public_market_query_is_not_blocked_by_missing_local_signer() {
    let root = NniTempDir::new();
    let state = isolated_nni_state(root.path());

    let error = execute_internal_nni_action(
        &state,
        &internal_nni_context(),
        &internal_nni_request(InternalNniAction::BancorMarket),
    )
    .await
    .expect_err("fixture has no configured remote node");

    assert_eq!(error.error_code, "nni_remote_node_unconfigured");
}

#[tokio::test]
async fn network_stats_requires_the_signed_rewards_contract() {
    let root = NniTempDir::new();
    let state = isolated_nni_state(root.path());

    let error = execute_internal_nni_action(
        &state,
        &internal_nni_context(),
        &internal_nni_request(InternalNniAction::NetworkStats),
    )
    .await
    .expect_err("network statistics are returned by the signed rewards endpoint");

    assert_eq!(error.error_code, "nni_signature_helper_unavailable");
}

#[test]
fn nni_gateway_sanitizes_sensitive_fields_recursively() {
    let sanitized = sanitize_nni_skill_data(json!({
        "node_url": "https://secret.example.test/private/path",
        "device_pubkey": "abcdef0123456789abcdef0123456789",
        "nested": {
            "signature": "private-signature",
            "challenge": "private-challenge",
            "token": "private-token",
            "device_pubkey_compact": "1234567890abcdef12345678",
        }
    }));
    assert!(sanitized.get("node_url").is_none());
    assert!(sanitized.get("device_pubkey").is_none());
    assert!(sanitized["nested"].get("signature").is_none());
    assert!(sanitized["nested"].get("challenge").is_none());
    assert!(sanitized["nested"].get("token").is_none());
    assert!(sanitized["nested"].get("device_pubkey_compact").is_none());
    assert_eq!(
        sanitized["nested"]["device_pubkey_preview"],
        "1234567890...345678"
    );
}

#[test]
fn nni_gateway_uses_persisted_error_code_without_parsing_error_text() {
    let projection = nni_skill_heartbeat_projection(&NniConfigResponse {
        remote_nodes: vec!["https://nni.example.test".to_string()],
        selected_node_url: Some("https://nni.example.test".to_string()),
        joined: true,
        asset_owner_pubkey: None,
        heartbeat_interval_seconds: 590,
        heartbeat_network_retry_limit: 3,
        heartbeat_request_count: 4,
        last_heartbeat_at_ts: Some(100),
        last_heartbeat_error: Some("localized or provider detail".to_string()),
        last_heartbeat_error_code: Some("heartbeat_request_network_failed".to_string()),
        last_heartbeat_error_at_ts: Some(200),
        last_heartbeat_network_failures: 3,
        last_heartbeat_attempt_at_ts: Some(200),
        consecutive_heartbeat_failures: 1,
        last_success_node_host: Some("nni.example.test".to_string()),
        network_authorization: "authorized".to_string(),
        heartbeat_state: "waiting_network".to_string(),
        next_heartbeat_due_at_ts: Some(690),
        worker_running: true,
        config_path: "ignored".to_string(),
    });
    assert_eq!(
        projection["last_error_code"],
        "heartbeat_request_network_failed"
    );
    assert_eq!(projection["effective_state"], "waiting_network");
}

#[test]
fn nni_gateway_errors_use_canonical_contract() {
    let (status, Json(response)) = nni_skill_error_response(
        InternalNniAction::HeartbeatEnable,
        NniSkillDomainError::new(
            StatusCode::PRECONDITION_FAILED,
            "nni_signature_device_unavailable",
            false,
            json!({"signer_kind": "unavailable"}),
        ),
    );
    let data = response.data.expect("error envelope");
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(data["error_code"], "nni_signature_device_unavailable");
    assert_eq!(data["retryable"], false);
    assert_eq!(data["failure_phase"], "execution_no_effect");
    assert_eq!(data["side_effect_applied"], false);
    assert!(data.get("error_kind").is_none());
}

#[test]
fn nni_gateway_distinguishes_rejected_and_uncertain_mutations() {
    let rejected = NniSkillDomainError::new(
        StatusCode::PRECONDITION_FAILED,
        "nni_device_not_authorized",
        false,
        json!({}),
    )
    .provider_rejected();
    assert_eq!(rejected.failure_phase, Some("provider_rejected"));
    assert_eq!(rejected.side_effect_applied, Some(false));

    let uncertain = NniSkillDomainError::new(
        StatusCode::BAD_GATEWAY,
        "nni_heartbeat_network_unavailable",
        true,
        json!({}),
    )
    .uncertain();
    assert_eq!(uncertain.failure_phase, None);
    assert_eq!(uncertain.side_effect_applied, None);
    assert_eq!(uncertain.recovery_action, Some("reconcile_before_retry"));
}

#[test]
fn nni_gateway_recognizes_current_authorization_rejection_tokens() {
    for token in [
        "nni_device_not_authorized",
        "nni_public_key_not_allowed",
        "nni_pubkey_not_allowlisted",
    ] {
        assert!(nni_heartbeat_error_is_authorization_rejection(token));
    }
    assert!(!nni_heartbeat_error_is_authorization_rejection(
        "heartbeat_verify_network_failed"
    ));
}

#[test]
fn nni_gateway_rejects_fields_owned_by_another_action() {
    let request: InternalNniActionRequest = serde_json::from_value(json!({
        "action": "status",
        "pay_amount": "10"
    }))
    .expect("decode known request fields");
    let error = validate_internal_nni_action_request(&request)
        .expect_err("status must reject quote fields");
    assert_eq!(error.error_code, "nni_argument_invalid");
    assert_eq!(error.details["invalid_fields"], json!(["pay_amount"]));
}

#[test]
fn nni_gateway_projects_hardware_simulated_and_unavailable_signers_without_ambiguity() {
    let hardware = nni_skill_device_projection(&json!({
        "helper_available": true,
        "hardware_chip_present": true,
        "signer_available": true,
        "local_participation_eligible": true,
        "signer_kind": "hardware",
    }));
    assert_eq!(hardware["hardware_chip_present"], true);
    assert_eq!(hardware["signer_kind"], "hardware");

    let simulated = nni_skill_device_projection(&json!({
        "helper_available": true,
        "hardware_chip_present": false,
        "signer_available": true,
        "local_participation_eligible": true,
        "signer_kind": "simulated",
    }));
    assert_eq!(simulated["hardware_chip_present"], false);
    assert_eq!(simulated["signer_available"], true);
    assert_eq!(simulated["signer_kind"], "simulated");
    assert_eq!(simulated["simulation_enabled"], true);
    assert_eq!(simulated["simulation_enable_available"], false);

    let unavailable = nni_skill_device_projection(&json!({
        "helper_available": true,
        "hardware_chip_present": false,
        "signer_available": false,
        "local_participation_eligible": false,
        "signer_kind": "unavailable",
        "simulation_available": true,
    }));
    assert_eq!(unavailable["signer_available"], false);
    assert_eq!(unavailable["local_participation_eligible"], false);
    assert_eq!(unavailable["simulation_enabled"], false);
    assert_eq!(unavailable["simulation_enable_available"], true);
}

#[test]
fn nni_gateway_retryability_uses_machine_tokens_and_status_codes() {
    assert!(nni_skill_attempt_is_retryable(&json!({
        "error_code": "nni_remote_query_network_failed"
    })));
    assert!(nni_skill_attempt_is_retryable(&json!({
        "http_status": 503,
        "error_code": "localized-provider-error"
    })));
    assert!(!nni_skill_attempt_is_retryable(&json!({
        "error_code": "the network wording appears only in an unregistered token"
    })));
}

#[test]
fn nni_gateway_never_promotes_remote_prose_to_an_error_code() {
    let prose = ApiResponse::<Value> {
        ok: false,
        data: None,
        error: Some("localized provider detail".to_string()),
    };
    assert_eq!(
        nni_skill_remote_error_code(&prose),
        "nni_remote_query_failed"
    );

    let authorized_machine_token = ApiResponse::<Value> {
        ok: false,
        data: None,
        error: Some("nni_pubkey_not_allowlisted".to_string()),
    };
    assert_eq!(
        nni_skill_remote_error_code(&authorized_machine_token),
        "nni_pubkey_not_allowlisted"
    );

    let structured = ApiResponse::<Value> {
        ok: false,
        data: Some(json!({"error_code": "nni_provider_contract_error"})),
        error: Some("ignored detail".to_string()),
    };
    assert_eq!(
        nni_skill_remote_error_code(&structured),
        "nni_provider_contract_error"
    );
}

#[test]
fn nni_gateway_collapses_uniform_authorization_rejections() {
    let error = nni_skill_attempts_error(
        "nni_rewards_query_failed",
        vec![
            json!({"error_code": "nni_pubkey_not_allowlisted", "http_status": 403}),
            json!({"error_code": "nni_device_not_authorized", "http_status": 403}),
        ],
    );
    assert_eq!(error.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(error.error_code, "nni_device_not_authorized");
    assert_eq!(error.failure_phase, Some("provider_rejected"));
    assert_eq!(error.side_effect_applied, Some(false));

    let mixed = nni_skill_attempts_error(
        "nni_rewards_query_failed",
        vec![
            json!({"error_code": "nni_pubkey_not_allowlisted", "http_status": 403}),
            json!({"error_code": "nni_response_contract_invalid", "http_status": 502}),
        ],
    );
    assert_eq!(mixed.status, StatusCode::BAD_GATEWAY);
    assert_eq!(mixed.error_code, "nni_rewards_query_failed");
}

#[test]
fn nni_gateway_adds_deterministic_utc_companions_to_machine_timestamps() {
    let mut data = json!({
        "updated_at_unix": 1_786_703_857_i64,
        "candles": [{
            "bucket_start_unix": 1_786_693_500_i64,
            "bucket_end_unix": 1_786_694_400_i64,
        }],
        "interval_seconds": 900,
        "unknown_at_ts": -1,
    });
    nni_skill_enrich_utc_timestamps(&mut data);
    assert_eq!(data["updated_at_utc"], "2026-08-14T10:37:37Z");
    assert_eq!(
        data["candles"][0]["bucket_start_utc"],
        "2026-08-14T07:45:00Z"
    );
    assert_eq!(data["candles"][0]["bucket_end_utc"], "2026-08-14T08:00:00Z");
    assert!(data.get("interval_utc").is_none());
    assert!(data.get("unknown_at_utc").is_none());
}

#[test]
fn nni_gateway_enforces_query_bounds_and_supported_candle_periods() {
    assert_eq!(nni_skill_limit(None, 20, 100).expect("default limit"), 20);
    assert_eq!(
        nni_skill_limit(Some(100), 20, 100).expect("upper limit"),
        100
    );
    assert_eq!(
        nni_skill_limit(Some(0), 20, 100)
            .expect_err("zero limit")
            .error_code,
        "nni_argument_invalid"
    );
    assert_eq!(
        nni_skill_limit(Some(101), 20, 100)
            .expect_err("oversized limit")
            .error_code,
        "nni_argument_invalid"
    );

    for (interval, seconds) in [
        ("1m", 60),
        ("5m", 300),
        ("15m", 900),
        ("1h", 3_600),
        ("4h", 14_400),
        ("1d", 86_400),
        ("1w", 604_800),
        ("1y", 31_536_000),
    ] {
        assert_eq!(
            nni_skill_interval_seconds(Some(interval)).expect("supported interval"),
            seconds
        );
    }
    assert_eq!(
        nni_skill_interval_seconds(Some("30m"))
            .expect_err("unsupported interval")
            .error_code,
        "nni_argument_invalid"
    );
}

#[test]
fn nni_network_stats_requires_structured_network_sections() {
    let data = nni_skill_network_stats(&json!({
        "network_devices": {"registered": 10, "active": 8},
        "reward_policy": {"window_reward": "5000"},
        "network_rewards": {"cumulative_output": "10000"},
    }))
    .expect("network statistics");
    assert_eq!(data["network_devices"]["active"], 8);
    assert_eq!(data["reward_policy"]["window_reward"], "5000");

    assert_eq!(
        nni_skill_network_stats(&json!({}))
            .expect_err("missing network statistics")
            .error_code,
        "nni_response_contract_invalid"
    );
}
