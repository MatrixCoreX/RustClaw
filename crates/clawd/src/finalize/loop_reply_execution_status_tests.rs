use super::*;
use crate::finalize::loop_reply::successful_content_observation_should_precede_status_summary;

#[test]
fn agent_loop_rich_content_precedes_status_summary_without_legacy_content_flag() {
    let mut route = free_route_result();
    route.requires_content_evidence = false;
    route.response_shape = crate::OutputResponseShape::Free;
    route.delivery_required = false;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "media_download",
        r#"{"text":"article body","extra":{"content_type":"image_article"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "image_vision",
        r#"{"text":"recognized text"}"#,
    ));

    assert!(successful_content_observation_should_precede_status_summary(Some(&ctx), &loop_state,));
}

#[test]
fn deterministic_missing_observed_target_answer_skips_after_later_fallback_success() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /tmp/missing-status-case.txt",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "find_files",
        r#"{"results":["/tmp/recovered.txt"]}"#,
    ));
    let mut route = free_route_result();
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "/tmp/missing-status-case.txt".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(deterministic_missing_observed_target_answer(
        &state,
        "find the file",
        &loop_state,
        Some(&ctx),
    )
    .is_none());
}

#[test]
fn observed_execution_status_payload_is_an_internal_trace() {
    let payload = concat!(
        "schema_version=1\n",
        "reason_code=observed_execution_status\n",
        "step.1.skill=media_download\n",
        "step.1.status=ok\n",
        "step.2.skill=image_vision\n",
        "step.2.status=error"
    );

    assert!(crate::finalize::looks_like_internal_trace_artifact(payload));
}

#[test]
fn ordinary_schema_payload_is_not_misclassified_as_execution_trace() {
    let payload = concat!(
        "schema_version=1\n",
        "reason_code=completed\n",
        "step.1.status=ok"
    );

    assert!(!crate::finalize::looks_like_internal_trace_artifact(
        payload
    ));
}
