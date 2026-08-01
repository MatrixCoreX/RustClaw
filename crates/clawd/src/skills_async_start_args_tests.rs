use serde_json::json;
use std::path::Path;

#[test]
fn planner_run_cmd_actions_keep_deadline_provenance_machine_readable() {
    let mut ordinary = json!({
        "action": "exec",
        "command": "sleep 70",
        "async_start": true,
        "timeout_seconds": 15
    });
    super::prepare_builtin_run_cmd_async_start_args(Path::new("."), 86_400, 5, &mut ordinary);
    assert!(ordinary["_clawd_async_runtime_timeout_seconds"].is_null());
    assert!(ordinary["_clawd_async_runtime_deadline_at"].is_null());

    let mut explicit = json!({
        "action": "exec_with_deadline",
        "command": "sleep 70",
        "async_start": true,
        "timeout_seconds": 15
    });
    super::prepare_builtin_run_cmd_async_start_args(Path::new("."), 86_400, 5, &mut explicit);
    assert_eq!(explicit["_clawd_async_runtime_timeout_seconds"], 15);

    let mut disabled = json!({
        "action": "exec_without_deadline",
        "command": "sleep 70",
        "async_start": true
    });
    super::prepare_builtin_run_cmd_async_start_args(Path::new("."), 86_400, 5, &mut disabled);
    assert!(disabled["_clawd_async_runtime_timeout_seconds"].is_null());
    assert!(disabled["_clawd_async_runtime_deadline_at"].is_null());
}
