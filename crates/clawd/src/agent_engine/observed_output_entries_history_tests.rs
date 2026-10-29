use super::{
    finalization_observed_output_entries, has_observed_answer_candidates, observed_output_entries,
};
use crate::agent_engine::LoopState;
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use claw_core::capability_result::CapabilityResultEnvelope;
use serde_json::{json, Value};

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn catalog(entries: &[String]) -> Value {
    serde_json::from_str(
        entries[0]
            .strip_prefix("canonical_capability_evidence=")
            .unwrap(),
    )
    .unwrap()
}

fn step(id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    }
}

#[test]
fn final_observation_window_keeps_earlier_red_probe_evidence() {
    let mut state = LoopState::new();
    let red = json!({"schema_version":1,"source":"run_cmd","exit_code":0,
        "complete":true,"output_truncated":false,
        "command_output":"{\"phase\":\"red_phase\",\"returncode\":1}"});
    state
        .executed_step_results
        .push(step("red_probe", "run_cmd", &red.to_string()));
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "system.run_command",
        Some("exec".to_string()),
        red,
    ));
    for index in 1..=5 {
        state.executed_step_results.push(step(
            &format!("read_{index}"),
            "read_file",
            &format!("verified_current_content_{index}"),
        ));
        state.capability_results.push(CapabilityResultEnvelope::ok(
            "filesystem.read_text_range",
            Some("read_range".to_string()),
            json!({"content":format!("verified_current_content_{index}")}),
        ));
    }
    assert!(!observed_output_entries(&state)
        .join("\n")
        .contains("red_phase"));
    let app = crate::AppState::test_default_with_fixture_provider();
    let entries = finalization_observed_output_entries(&app, &task(), &state).unwrap();
    let value = catalog(&entries);
    assert_eq!(value["result_count"], 6);
    assert_eq!(value["model_view_complete"], true);
    let entries = entries.join("\n");
    assert!(entries.contains("red_phase"), "{entries}");
    assert!(entries.contains("verified_current_content_5"));
}

#[test]
fn finalization_catalog_does_not_trust_raw_steps_over_canonical_results() {
    let app = crate::AppState::test_default_with_fixture_provider();
    let mut state = LoopState::new();
    state
        .executed_step_results
        .push(step("unbound_step", "read_file", "unbound_raw_claim"));
    let mut expected_ids = Vec::new();
    for index in 0..40 {
        let result = CapabilityResultEnvelope::ok(
            "fixture.observe",
            Some("inspect".to_string()),
            json!({"ordinal":index,"observed_fact":format!("grounded_value_{index}")}),
        );
        expected_ids.push(result.canonical_evidence_identity().evidence_id);
        state.capability_results.push(result);
    }
    let entries = finalization_observed_output_entries(&app, &task(), &state).unwrap();
    let value = catalog(&entries);
    let records = value["entries"].as_array().unwrap();
    assert!(has_observed_answer_candidates(&state));
    assert_eq!(records.len(), 40);
    assert!(!entries.join("\n").contains("unbound_raw_claim"));
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record["ordinal"], index + 1);
        assert_eq!(record["evidence_id"], expected_ids[index]);
        assert_eq!(record["model_view"]["result"]["data"]["ordinal"], index);
    }
}

#[test]
fn finalization_catalog_redacts_earlier_secrets_and_preserves_error_status() {
    let app = crate::AppState::test_default_with_fixture_provider();
    let mut state = LoopState::new();
    assert!(!has_observed_answer_candidates(&state));
    state
        .capability_results
        .push(CapabilityResultEnvelope::failed(
            "fixture.inspect",
            Some("inspect".to_string()),
            claw_core::capability_result::StructuredError {
                code: "fixture_failed".to_string(),
                message_key: "fixture.failed".to_string(),
                retryable: false,
                details: json!({"api_key":"fixture_credential_must_not_escape"}),
            },
        ));
    for index in 0..5 {
        state.capability_results.push(CapabilityResultEnvelope::ok(
            "fixture.inspect",
            Some("inspect".to_string()),
            json!({"value":index}),
        ));
    }
    let entries = finalization_observed_output_entries(&app, &task(), &state).unwrap();
    assert!(!entries
        .join("\n")
        .contains("fixture_credential_must_not_escape"));
    let value = catalog(&entries);
    assert_eq!(value["entries"][0]["status"], "error");
    assert_eq!(value["entries"][0]["model_view_redacted"], true);
    assert_eq!(value["entries"][5]["status"], "ok");
}

#[test]
fn finalization_without_capability_results_keeps_existing_observation_contract() {
    let app = crate::AppState::test_default_with_fixture_provider();
    let mut state = LoopState::new();
    assert!(finalization_observed_output_entries(&app, &task(), &state)
        .unwrap()
        .is_empty());
    state
        .executed_step_results
        .push(step("observed", "read_file", "observed_content"));
    state
        .executed_step_results
        .push(step("answer", "respond", "model_claim"));
    assert_eq!(
        finalization_observed_output_entries(&app, &task(), &state).unwrap(),
        observed_output_entries(&state)
    );
}

struct CatalogWorkspace(std::path::PathBuf);

impl CatalogWorkspace {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("agent-finalization-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for CatalogWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn finalization_catalog_preserves_large_earlier_results_with_verified_continuation() {
    use sha2::Digest;
    let workspace = CatalogWorkspace::new();
    let mut app = crate::AppState::test_default_with_fixture_provider();
    app.skill_rt.workspace_root = workspace.0.clone();
    let content = "large_observed_content_".repeat(16_000);
    let mut state = LoopState::new();
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "fixture.read",
        Some("read".to_string()),
        json!({"content":content,"complete":true}),
    ));
    for index in 0..8 {
        state.capability_results.push(CapabilityResultEnvelope::ok(
            "fixture.inspect",
            Some("inspect".to_string()),
            json!({"value":index}),
        ));
    }
    let owner = task();
    let entries = finalization_observed_output_entries(&app, &owner, &state).unwrap();
    let value = catalog(&entries);
    assert_eq!(value["result_count"], 9);
    assert_eq!(value["canonical_complete"], true);
    assert_eq!(value["model_view_complete"], false);
    let first = &value["entries"][0];
    assert_eq!(first["model_view"]["complete"], false);
    assert_eq!(
        first["model_view"]["continuation"]["kind"],
        "artifact_range"
    );
    let relative = first["model_view"]["continuation"]["range_handle"]["path"]
        .as_str()
        .unwrap();
    let artifact = workspace.0.join(relative);
    assert!(artifact.canonicalize().unwrap().starts_with(&workspace.0));
    let bytes = std::fs::read(artifact).unwrap();
    assert_eq!(
        first["model_projection_sha256"],
        format!("{:x}", sha2::Sha256::digest(&bytes))
    );
    let recovered: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(recovered["data"]["content"], content);
    assert_eq!(
        first["evidence_id"],
        state.capability_results[0]
            .canonical_evidence_identity()
            .evidence_id
    );
}

#[test]
fn finalization_catalog_publication_failure_does_not_silently_revert_to_recent_steps() {
    let workspace = CatalogWorkspace::new();
    let mut app = crate::AppState::test_default_with_fixture_provider();
    app.skill_rt.workspace_root = workspace.0.join("not-a-directory");
    std::fs::write(&app.skill_rt.workspace_root, "fixture").unwrap();
    let mut state = LoopState::new();
    state.capability_results.push(CapabilityResultEnvelope::ok(
        "fixture.read",
        Some("read".to_string()),
        json!({"content":"large_observed_content_".repeat(16_000)}),
    ));
    state
        .executed_step_results
        .push(step("raw", "read_file", "unbound_claim"));
    let error = finalization_observed_output_entries(&app, &task(), &state).unwrap_err();
    assert_eq!(error, "model_evidence_artifact_write_failed");
}

#[path = "observed_output_entries_gateway_tests.rs"]
mod gateway_tests;
