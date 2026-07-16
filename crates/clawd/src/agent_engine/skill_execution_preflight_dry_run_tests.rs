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
fn evidence_policy_preflight_requires_explicit_run_cmd_dry_run_field() {
    let state = test_state();
    let loop_state = LoopState::new(2);
    let args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_DRY_RUN",
        "async_start": true,
        "poll_after_seconds": 2,
        "expires_in_seconds": 600,
        "dry_run": true
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

    let live_args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_DRY_RUN",
        "async_start": true,
        "poll_after_seconds": 2,
        "expires_in_seconds": 600
    });
    assert!(
        evidence_policy_action_policy_error(
            &state,
            &loop_state,
            "run_cmd",
            &live_args,
            "call_skill",
        )
        .is_none(),
        "preflight must not inherit dry-run mode from removed route state"
    );
}
