use super::builtin_schedule::{schedule_args_contain_structured_intent, schedule_workflow_prompt};
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
