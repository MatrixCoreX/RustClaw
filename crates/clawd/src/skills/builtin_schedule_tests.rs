use super::builtin_schedule::schedule_workflow_prompt;
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
