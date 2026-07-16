use super::tests::{install_test_registry, test_state};
use super::{evidence_policy_action_policy_error, preflight_permission_decision, LoopState};

#[test]
fn preflight_permission_decision_marks_media_dry_run_as_low_risk_observe() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "image.generate", action = "generate", effect = "external", required = ["prompt"], optional = ["dry_run", "output_path"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "action" },
]
"#,
        &["image_generate"],
    );
    let args = serde_json::json!({
        "action": "generate",
        "prompt": "status card",
        "output_path": "document/media_dry_run/image_status_card.png",
        "dry_run": true
    });

    let permission = preflight_permission_decision(
        &state,
        "image_generate",
        &args,
        "media_dry_run_probe",
        "media_dry_run_probe",
    );

    assert_eq!(permission["risk_level"], serde_json::json!("low"));
    assert_eq!(permission["needs_confirmation"], false);
    assert_eq!(permission["action_effect"], serde_json::json!("observe"));
}

#[test]
fn evidence_policy_preflight_rejects_async_start_for_dry_run_contract() {
    let state = test_state();
    let mut route = crate::RouteResult {
        resolved_intent:
            "async_job_protocol=version:1 mode=dry_run adapter_result_key=async_poll_adapter_result"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "async_job_protocol=version:1 mode=dry_run would_mutate=false required_job_fields=job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key"
                .to_string(),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..crate::IntentOutputContract::default()
        },
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    let args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_DRY_RUN",
        "async_start": true,
        "poll_after_seconds": 2,
        "expires_in_seconds": 600
    });

    let err =
        evidence_policy_action_policy_error(&state, &loop_state, "run_cmd", &args, "call_skill")
            .expect("dry-run async starts must not execute a local process");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("dry-run async preflight error should be structured");

    assert_eq!(parsed.error_kind, "contract_action_rejected");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!(
            "run_cmd_dry_run_requires_preview_contract"
        ))
    );
    assert_eq!(
        parsed.extra.as_ref().and_then(|extra| extra.get("dry_run")),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("forbidden_effect")),
        Some(&serde_json::json!("local_process_start"))
    );

    let plain_args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_DRY_RUN"
    });
    let err = evidence_policy_action_policy_error(
        &state,
        &loop_state,
        "run_cmd",
        &plain_args,
        "call_skill",
    )
    .expect("dry-run async route must also reject plain local process starts");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("plain dry-run process preflight error should be structured");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!(
            "run_cmd_dry_run_requires_preview_contract"
        ))
    );
}
