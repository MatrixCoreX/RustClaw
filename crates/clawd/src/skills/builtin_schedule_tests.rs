use super::builtin_schedule::{
    explicit_schedule_intent_from_args, schedule_args_contain_structured_intent,
    schedule_replan_error, schedule_workflow_prompt,
};
use serde_json::json;

#[test]
fn schedule_workflow_prompt_accepts_string_intent_alias() {
    let args = json!({
        "action": "preview",
        "intent": "schedule source text"
    });
    let map = args.as_object().expect("schedule args object");

    assert_eq!(schedule_workflow_prompt(map, &args), "schedule source text");
}

#[test]
fn schedule_workflow_prompt_prefers_explicit_text_over_intent_alias() {
    let args = json!({
        "action": "preview",
        "text": "primary schedule source",
        "intent": "fallback schedule source"
    });
    let map = args.as_object().expect("schedule args object");

    assert_eq!(
        schedule_workflow_prompt(map, &args),
        "primary schedule source"
    );
}

#[test]
fn schedule_preview_control_fields_do_not_claim_structured_intent() {
    let args = json!({
        "action": "preview",
        "intent": "schedule source text",
        "dry_run": true,
        "preview_only": true,
        "create_real": false,
        "mode": "compile_only",
        "timezone": "Asia/Shanghai"
    });

    assert!(!schedule_args_contain_structured_intent(&args));
}

#[test]
fn schedule_machine_fields_claim_structured_intent() {
    for args in [
        json!({"kind": "list"}),
        json!({"schedule": {"type": "once"}}),
        json!({"task": {"kind": "ask"}}),
        json!({"target_job_id": "job_123"}),
    ] {
        assert!(schedule_args_contain_structured_intent(&args));
    }
}

#[test]
fn schedule_intent_json_preserves_nested_machine_types() {
    let args = json!({
        "intent_json": json!({
            "kind": "create",
            "schedule": {"type": "interval", "every_minutes": 60},
            "task": {
                "kind": "run_skill",
                "payload": {
                    "skill_name": "example_skill",
                    "args": {"action": "run_once", "platforms": ["example"], "scheduled_run": true}
                }
            }
        }).to_string()
    });
    let intent = explicit_schedule_intent_from_args(&args, "create", "schedule workflow request")
        .expect("valid structured intent")
        .expect("intent json must produce an intent");

    assert_eq!(intent.schedule.every_minutes, 60);
    assert_eq!(intent.task.payload["args"]["platforms"], json!(["example"]));
    assert_eq!(intent.task.payload["args"]["scheduled_run"], true);
}

#[test]
fn schedule_clarification_proves_mutation_was_not_applied() {
    let mut intent = crate::ScheduleIntentOutput::default();
    intent.needs_clarify = true;
    intent.clarify_question = "Please provide a time".to_string();

    let encoded = schedule_replan_error(&intent);
    let structured =
        crate::skills::parse_structured_skill_error(&encoded).expect("structured schedule error");
    let extra = structured.extra.expect("schedule error extra");
    assert_eq!(structured.error_code, "schedule_needs_more_info");
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["recovery_action"], "replan_arguments");
}

#[test]
fn schedule_list_without_filters_uses_structured_list_intent() {
    let args = json!({"action": "list"});
    let intent = explicit_schedule_intent_from_args(&args, "list", "schedule workflow request")
        .expect("valid list intent")
        .expect("standalone list intent");

    assert_eq!(intent.kind, "list");
    assert!(!intent.needs_clarify);
}
