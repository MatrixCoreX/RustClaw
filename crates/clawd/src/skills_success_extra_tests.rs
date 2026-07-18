use std::path::Path;

use serde_json::{json, Value};

use super::builtin_success_extra;

fn schedule_extra(args: Value) -> Value {
    builtin_success_extra(Path::new("/tmp"), "schedule", &args).expect("schedule success extra")
}

#[test]
fn schedule_preview_actions_are_always_structured_dry_runs() {
    for args in [
        json!({"action": "preview"}),
        json!({"action": "create", "mode": "compile_only"}),
    ] {
        let extra = schedule_extra(args);
        assert_eq!(extra.get("dry_run").and_then(Value::as_bool), Some(true));
        assert_eq!(
            extra.get("preview_only").and_then(Value::as_bool),
            Some(true)
        );
    }
}

#[test]
fn schedule_execute_action_keeps_non_preview_defaults() {
    let extra = schedule_extra(json!({"action": "create"}));
    assert_eq!(extra.get("dry_run").and_then(Value::as_bool), Some(false));
    assert_eq!(
        extra.get("preview_only").and_then(Value::as_bool),
        Some(false)
    );
}
