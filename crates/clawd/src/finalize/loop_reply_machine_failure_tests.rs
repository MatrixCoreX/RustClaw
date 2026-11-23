use super::*;
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use serde_json::{json, Value};

fn runner_error(extra: Value, diagnostic: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: "step_1".into(),
        skill: "fixture_skill".into(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "fixture_skill",
            "launch_receipt_missing",
            diagnostic,
            None,
            Some(extra),
        )),
        started_at: 1,
        finished_at: 2,
    }
}

fn host_failure() -> Value {
    json!({"message_key":"clawd.skill.pre_dispatch_failed",
        "failure_phase":"pre_dispatch","side_effect_applied":false,
        "recovery_action":"refresh_skill_admission"})
}

#[test]
fn typed_host_startup_failure_keeps_owner_without_reading_diagnostic_prose() {
    for diagnostic in ["No such file or directory", "Permission denied", "启动失败"] {
        let (payload, host_owned) =
            unresolved_failure_payload(&runner_error(host_failure(), diagnostic)).unwrap();
        assert!(host_owned);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["error_code"], "launch_receipt_missing");
        assert_eq!(
            payload["extra"]["recovery_action"],
            "refresh_skill_admission"
        );
        assert!(payload.get("error_text").is_none());
    }
}

#[test]
fn incomplete_or_domain_failure_cannot_claim_host_setup_ownership() {
    for (key, replacement) in [
        ("message_key", json!("fixture.domain_error")),
        ("failure_phase", json!("execution")),
        ("side_effect_applied", json!(true)),
        ("recovery_action", json!("request_user_input")),
    ] {
        let mut extra = host_failure();
        extra[key] = replacement;
        assert!(
            unresolved_failure_payload(&runner_error(extra, "refresh_skill_admission")).is_none()
        );
        let mut extra = host_failure();
        extra.as_object_mut().unwrap().remove(key);
        assert!(unresolved_failure_payload(&runner_error(extra, "pre_dispatch")).is_none());
    }
    let mut untyped = runner_error(host_failure(), "fixture");
    untyped.error = Some("launch_receipt_missing".into());
    assert!(unresolved_failure_payload(&untyped).is_none());
}

#[tokio::test]
async fn later_success_remains_authoritative_instead_of_reviving_old_host_failure() {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "fixture-later-success".into(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let mut loop_state = LoopState::default();
    loop_state
        .executed_step_results
        .push(runner_error(host_failure(), "fixture"));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".into(),
        skill: "respond".into(),
        status: StepExecutionStatus::Ok,
        output: Some("completed result".into()),
        error: None,
        started_at: 3,
        finished_at: 4,
    });
    assert!(
        finalize_unresolved_machine_failure(&state, &task, "fixture", &mut loop_state, None)
            .await
            .is_none()
    );
    assert_eq!(loop_state.executed_step_results.len(), 2);
}
