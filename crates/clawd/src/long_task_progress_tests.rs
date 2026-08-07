use serde_json::json;

use super::*;

#[test]
fn unknown_duration_reports_alive_without_fake_percentage() {
    let progress = operation_progress_from_lifecycle(
        &json!({"state": "background", "heartbeat_at": 42, "can_cancel": true}),
        None,
    );
    assert_eq!(progress["phase_key"], "background");
    assert_eq!(progress["progress_kind"], "alive_only");
    assert!(progress["total_units"].is_null());
    assert_eq!(progress["heartbeat_at"], 42);
}

#[test]
fn terminal_progress_is_complete_and_not_controllable() {
    let progress = operation_progress_from_lifecycle(&json!({"state": "succeeded"}), None);
    assert_eq!(progress["completed_units"], 1);
    assert_eq!(progress["total_units"], 1);
    assert_eq!(progress["can_pause"], false);
    assert_eq!(progress["can_cancel"], false);
}
