use super::tests::{install_test_registry, test_state};
use super::{evidence_policy_action_policy_error, LoopState};

#[test]
fn evidence_policy_preflight_rejects_non_x_dry_run() {
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
input_schema = { type = "object", properties = { prompt = { type = "string" }, dry_run = { type = "boolean" }, output_path = { type = "string" } } }
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

    let err = evidence_policy_action_policy_error(
        &state,
        &LoopState::new(),
        "image_generate",
        &args,
        "call_skill",
    )
    .expect("non-X dry-run must be rejected before execution");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("non-X dry-run rejection should be structured");
    assert_eq!(parsed.error_code, "contract_action_rejected");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("dry_run_reserved_for_x"))
    );
}

#[test]
fn evidence_policy_preflight_rejects_run_cmd_dry_run() {
    let state = test_state();
    let loop_state = LoopState::new();
    let args = serde_json::json!({
        "command": "sleep 2 && echo APP_ASYNC_DRY_RUN",
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

    assert_eq!(parsed.error_code, "contract_action_rejected");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("dry_run_reserved_for_x"))
    );
    assert_eq!(
        parsed.extra.as_ref().and_then(|extra| extra.get("dry_run")),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("allowed_skill")),
        Some(&serde_json::json!("x"))
    );

    let live_args = serde_json::json!({
        "command": "sleep 2 && echo APP_ASYNC_DRY_RUN",
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

#[test]
fn run_cmd_async_start_uses_defaults_but_rejects_explicit_invalid_bounds() {
    let state = test_state();
    let loop_state = LoopState::new();
    let defaults = serde_json::json!({
        "command": "sleep 2",
        "async_start": true
    });
    assert!(evidence_policy_action_policy_error(
        &state,
        &loop_state,
        "run_cmd",
        &defaults,
        "call_skill",
    )
    .is_none());

    let invalid = serde_json::json!({
        "command": "sleep 2",
        "async_start": true,
        "poll_after_seconds": 0
    });
    let err =
        evidence_policy_action_policy_error(&state, &loop_state, "run_cmd", &invalid, "call_skill")
            .expect("explicit invalid lifecycle bound should be rejected");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("invalid lifecycle bound error should be structured");
    assert_eq!(parsed.error_code, "contract_action_rejected");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("async_start_invalid_lifecycle_bound"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("invalid_fields")),
        Some(&serde_json::json!(["poll_after_seconds"]))
    );
}
